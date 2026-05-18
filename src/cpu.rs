use crate::errors::{EmulationError, EmulationResult};
use crate::filesystem::VirtualFileSystem;
use crate::libsystem::{handle_libcall, LibCallOutcome, Trampoline};
use crate::syscall::SyscallHandler;
use crate::syscall::{SyscallArgs, SyscallOutcome};
use log::{debug, info};
use std::cell::RefCell;
use std::rc::Rc;
use unicorn_engine::unicorn_const::{Arch, Mode, Prot};
use unicorn_engine::{RegisterX86, Unicorn};

/// i386 CPU emulator wrapper around Unicorn Engine
pub struct CpuEmulator {
    emu: Unicorn<'static, ()>,
}

impl CpuEmulator {
    /// Create a new CPU emulator for i386 (32-bit x86)
    pub fn new() -> EmulationResult<Self> {
        let emu = Unicorn::new(Arch::X86, Mode::MODE_32).map_err(|e| {
            EmulationError::EmulationError(format!("Failed to create Unicorn instance: {:?}", e))
        })?;

        Ok(CpuEmulator { emu })
    }

    /// Map memory regions for the emulator
    pub fn map_memory(&mut self, base: u32, size: u32) -> EmulationResult<()> {
        // Allocate memory region with full permissions (R+W+X)
        self.emu
            .mem_map(base as u64, size as u64, Prot::ALL)
            .map_err(|e| EmulationError::MemoryError(format!("Failed to map memory: {:?}", e)))?;

        debug!("Mapped memory: 0x{:x} - 0x{:x}", base, base + size);
        Ok(())
    }

    /// Write data to emulated memory
    pub fn write_memory(&mut self, addr: u32, data: &[u8]) -> EmulationResult<()> {
        self.emu
            .mem_write(addr as u64, data)
            .map_err(|e| EmulationError::MemoryError(format!("Failed to write memory: {:?}", e)))?;

        Ok(())
    }

    /// Read data from emulated memory
    pub fn read_memory(&self, addr: u32, size: usize) -> EmulationResult<Vec<u8>> {
        let mut buffer = vec![0u8; size];
        self.emu
            .mem_read(addr as u64, &mut buffer)
            .map_err(|e| EmulationError::MemoryError(format!("Failed to read memory: {:?}", e)))?;
        Ok(buffer)
    }

    /// Set a register value
    pub fn set_register(&mut self, reg: RegisterX86, value: u32) -> EmulationResult<()> {
        self.emu.reg_write(reg, value as u64).map_err(|e| {
            EmulationError::EmulationError(format!("Failed to set register: {:?}", e))
        })?;

        Ok(())
    }

    /// Get a register value
    pub fn get_register(&self, reg: RegisterX86) -> EmulationResult<u32> {
        let value = self.emu.reg_read(reg).map_err(|e| {
            EmulationError::EmulationError(format!("Failed to read register: {:?}", e))
        })?;

        Ok(value as u32)
    }

    /// Setup syscall hook and optional instruction trace hook.
    ///
    /// The syscall hook intercepts `INT 0x80` in the code stream. Syscall args
    /// follow the Linux i386 register convention: EAX=num, EBX..EBP=args.
    /// The return value is 64-bit; the low word goes to EAX, the high word to
    /// EDX (used by lseek and similar syscalls that return a 64-bit result).
    ///
    /// When `trace_instr` is true a second code hook prints every instruction
    /// address to stdout, prefixed with `[instr]`.
    pub fn setup_syscall_hook(
        &mut self,
        handler: SyscallHandler,
        fs: Rc<RefCell<VirtualFileSystem>>,
        trace_instr: bool,
    ) -> EmulationResult<()> {
        self.emu
            .add_code_hook(
                0,
                u64::MAX,
                move |emu: &mut Unicorn<'_, ()>, addr: u64, _size: u32| {
                    let mut op = [0u8; 2];
                    if emu.mem_read(addr, &mut op).is_err() {
                        return;
                    }
                    if op[0] != 0xCD || op[1] != 0x80 {
                        return;
                    }

                    let args = SyscallArgs {
                        number: emu.reg_read(RegisterX86::EAX).unwrap_or(0) as u32,
                        arg0: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
                        arg1: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
                        arg2: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
                        arg3: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
                        arg4: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
                        arg5: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
                    };

                    let result = {
                        let mut fs_guard = fs.borrow_mut();
                        handler.handle_syscall(emu, &mut fs_guard, args)
                    };

                    match result {
                        Ok((SyscallOutcome::Continue, retval)) => {
                            let _ = emu.reg_write(RegisterX86::EAX, retval & 0xFFFF_FFFF);
                            let _ = emu.reg_write(RegisterX86::EDX, retval >> 32);
                            let _ = emu.set_pc(addr + 2);
                        }
                        Ok((SyscallOutcome::Exit(status), _)) => {
                            log::info!("sys_exit({})", status);
                            let _ = emu.reg_write(RegisterX86::EAX, status as u64);
                            let _ = emu.emu_stop();
                        }
                        Ok((SyscallOutcome::StateSet, _)) => {
                            // Handler set PC/ESP directly; do not advance past INT.
                        }
                        Ok((SyscallOutcome::DeliverSignal { handler }, _)) => {
                            // Set up a guest signal frame and jump to the handler.
                            // Resume address is addr+2 (past the INT instruction).
                            {
                                let mut fs_guard = fs.borrow_mut();
                                deliver_signal(emu, &mut fs_guard, handler, addr as u32 + 2);
                            }
                            // PC is now pointed at the handler; do NOT advance.
                        }
                        Err(err) => {
                            log::warn!("syscall error: {}", err);
                            let _ = emu.reg_write(RegisterX86::EAX, 0xFFFF_FFFF);
                            let _ = emu.set_pc(addr + 2);
                        }
                    }
                },
            )
            .map_err(|e| {
                EmulationError::EmulationError(format!("Failed to add syscall hook: {:?}", e))
            })?;

        if trace_instr {
            self.emu
                .add_code_hook(
                    0,
                    u64::MAX,
                    |_emu: &mut Unicorn<'_, ()>, addr: u64, size: u32| {
                        println!("[instr] @ 0x{:08x}  ({} bytes)", addr, size);
                    },
                )
                .map_err(|e| {
                    EmulationError::EmulationError(format!(
                        "Failed to add instruction trace hook: {:?}",
                        e
                    ))
                })?;
        }

        // Memory-fault hook: log the exact address before Unicorn terminates.
        self.emu
            .add_mem_hook(
                unicorn_engine::unicorn_const::HookType::MEM_INVALID,
                0,
                u64::MAX,
                |emu, mem_type, addr, size, _value| {
                    let eip = emu.reg_read(RegisterX86::EIP).unwrap_or(0);
                    log::error!(
                        "Memory fault: {:?} at 0x{:08x} (size={}) from EIP=0x{:08x}",
                        mem_type,
                        addr,
                        size,
                        eip
                    );
                    false // let Unicorn propagate the error
                },
            )
            .map_err(|e| {
                EmulationError::EmulationError(format!("Failed to add mem-fault hook: {:?}", e))
            })?;

        debug!("Syscall hook setup");
        Ok(())
    }

    /// Register a code hook that intercepts calls into the libSystem trampoline
    /// region and dispatches them to the Rust handler implementations.
    pub fn setup_trampoline_hook(
        &mut self,
        fs: Rc<RefCell<VirtualFileSystem>>,
        trampoline: &Trampoline,
    ) -> EmulationResult<()> {
        let end = trampoline.region_end();
        let slot_count = trampoline.slot_count;
        let dispatch = trampoline.dispatch.clone();

        self.emu
            .add_code_hook(
                crate::libsystem::TRAMPOLINE_BASE as u64,
                end as u64,
                move |emu: &mut Unicorn<'_, ()>, addr: u64, _size: u32| {
                    let sym_opt = dispatch.get(&(addr as u32)).copied();
                    match sym_opt {
                        Some(sym) => {
                            info!("trampoline hit 0x{:08x} -> {:?}", addr, sym);
                            let outcome = {
                                let mut fs_guard = fs.borrow_mut();
                                handle_libcall(emu, &mut fs_guard, sym)
                            };
                            if matches!(outcome, LibCallOutcome::Exit) {
                                let _ = emu.emu_stop();
                            }
                        }
                        None => {
                            info!("trampoline hit 0x{:08x} -> <unknown>", addr);
                        }
                    }
                },
            )
            .map_err(|e| {
                EmulationError::EmulationError(format!("Failed to add trampoline hook: {:?}", e))
            })?;

        debug!("Trampoline hook setup ({} slots)", slot_count);
        Ok(())
    }

    /// Initialize CPU state for program execution
    pub fn init_cpu_state(&mut self, entry_point: u32, stack_ptr: u32) -> EmulationResult<()> {
        // Set instruction pointer to entry point
        self.set_register(RegisterX86::EIP, entry_point)?;

        // Set stack pointer
        self.set_register(RegisterX86::ESP, stack_ptr)?;

        // EBP=0 marks the outermost call frame (no caller).
        self.set_register(RegisterX86::EBP, 0)?;

        // Initialize other registers to 0
        for reg in &[
            RegisterX86::EAX,
            RegisterX86::EBX,
            RegisterX86::ECX,
            RegisterX86::EDX,
            RegisterX86::ESI,
            RegisterX86::EDI,
        ] {
            self.set_register(*reg, 0)?;
        }

        info!(
            "CPU state initialized: EIP=0x{:x}, ESP=0x{:x}",
            entry_point, stack_ptr
        );

        Ok(())
    }

    /// Execute code from the current entry point until completion
    pub fn execute(&mut self, _timeout: u64) -> EmulationResult<()> {
        // Get current EIP
        let eip = self.get_register(RegisterX86::EIP)?;

        // Run emulation - start from EIP, no end address limit
        self.emu
            .emu_start(eip as u64, 0xFFFFFFFFu64, 0, 0)
            .map_err(|e| EmulationError::EmulationError(format!("Execution error: {:?}", e)))?;

        Ok(())
    }

    /// Get current CPU state for debugging
    pub fn dump_registers(&self) -> EmulationResult<CpuState> {
        Ok(CpuState {
            eax: self.emu.reg_read(RegisterX86::EAX).unwrap_or(0) as u32,
            ebx: self.emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
            ecx: self.emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
            edx: self.emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
            esi: self.emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
            edi: self.emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
            ebp: self.emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
            esp: self.emu.reg_read(RegisterX86::ESP).unwrap_or(0) as u32,
            eip: self.emu.reg_read(RegisterX86::EIP).unwrap_or(0) as u32,
        })
    }
}

/// Set up a guest signal frame and switch execution to the handler.
///
/// Uses the same continuation-stack mechanism as `pthread_create`.  When the
/// handler returns to `SIGNAL_RETURN_ADDR`, the libsystem trampoline pops the
/// saved context and resumes execution at `resume_addr`.
fn deliver_signal(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    handler: u32,
    resume_addr: u32,
) {
    use crate::libsystem::SIGNAL_RETURN_ADDR;
    use crate::threads::ThreadContinuation;

    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0) as u32;

    // Push signal number (2 = SIGINT) then the sentinel return address.
    let mut new_esp = esp;
    new_esp -= 4;
    let _ = emu.mem_write(new_esp as u64, &2u32.to_le_bytes()); // signum
    new_esp -= 4;
    let _ = emu.mem_write(new_esp as u64, &SIGNAL_RETURN_ADDR.to_le_bytes());

    // Save the interrupted context.
    let cont = ThreadContinuation {
        ret_addr: resume_addr,
        tid: 0, // 0 = signal return (not a real thread)
        ebx: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
        ecx: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
        edx: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
        esi: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
        edi: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
        ebp: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
        esp,
    };
    fs.threads.continuations.push(cont);

    let _ = emu.reg_write(RegisterX86::ESP, new_esp as u64);
    let _ = emu.set_pc(handler as u64);
}

/// Snapshot of CPU state for debugging
#[derive(Debug, Clone)]
pub struct CpuState {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub eip: u32,
}

impl std::fmt::Display for CpuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EIP=0x{:08x} ESP=0x{:08x} EBP=0x{:08x} EAX=0x{:08x} EBX=0x{:08x} ECX=0x{:08x} EDX=0x{:08x} ESI=0x{:08x} EDI=0x{:08x}",
            self.eip, self.esp, self.ebp, self.eax, self.ebx, self.ecx, self.edx, self.esi, self.edi
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_creation() {
        let cpu = CpuEmulator::new();
        assert!(cpu.is_ok());
    }
}
