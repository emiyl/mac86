use crate::errors::{EmulationError, EmulationResult};
use crate::syscall::SyscallHandler;
use log::{debug, info};
use std::io::{self, Read, Write};
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

    /// Setup syscall hook with the provided handler
    pub fn setup_syscall_hook(&mut self, _handler: &SyscallHandler) -> EmulationResult<()> {
        // Add a code hook to intercept the INT 0x80 two-byte sequence (0xCD 0x80).
        // This runs before instruction execution; when we detect the INT instruction
        // we emulate the syscall here and advance the PC to skip the instruction.
        self.emu
            .add_code_hook(
                0,
                u64::MAX,
                |emu: &mut Unicorn<'_, ()>, addr: u64, _size: u32| {
                    // Read two bytes at `addr` safely
                    let mut op = [0u8; 2];
                    if emu.mem_read(addr, &mut op).is_err() {
                        return;
                    }
                    // INT imm8 instruction is 0xCD imm8
                    if op[0] != 0xCD {
                        return;
                    }
                    if op[1] != 0x80 {
                        return;
                    }

                    // It's INT 0x80 - handle syscall
                    let syscall_no = emu.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
                    let arg0 = emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32;
                    let arg1 = emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32;
                    let arg2 = emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32;

                    match syscall_no {
                        1 => {
                            // exit(status)
                            let status = arg0 as i32;
                            log::info!("sys_exit({})", status);
                            let _ = emu.emu_stop();
                        }
                        4 => {
                            // write(fd, buf, count)
                            let fd = arg0;
                            let buf = arg1 as u64;
                            let count = arg2 as usize;
                            log::debug!("sys_write(fd={}, buf=0x{:x}, count={})", fd, buf, count);
                            if count == 0 {
                                let _ = emu.reg_write(RegisterX86::EAX, 0);
                            } else {
                                let mut out = vec![0u8; count];
                                if emu.mem_read(buf, &mut out).is_ok() {
                                    if fd == 1 || fd == 2 {
                                        let _ = io::stdout().write_all(&out);
                                        let _ = io::stdout().flush();
                                    }
                                    let _ = emu.reg_write(RegisterX86::EAX, count as u64);
                                } else {
                                    let _ = emu.reg_write(RegisterX86::EAX, u64::MAX);
                                }
                            }
                        }
                        3 => {
                            // read(fd, buf, count)
                            let fd = arg0;
                            let buf = arg1 as u64;
                            let count = arg2 as usize;
                            log::debug!("sys_read(fd={}, buf=0x{:x}, count={})", fd, buf, count);
                            let mut input = vec![0u8; count];
                            let n = io::stdin().read(&mut input).unwrap_or(0);
                            if n > 0 {
                                let _ = emu.mem_write(buf, &input[..n]);
                            }
                            let _ = emu.reg_write(RegisterX86::EAX, n as u64);
                        }
                        _ => {
                            log::warn!("Unimplemented syscall: {}", syscall_no);
                            let _ = emu.reg_write(RegisterX86::EAX, u64::MAX);
                        }
                    }

                    // Advance PC to skip the INT instruction (2 bytes)
                    let _ = emu.set_pc(addr + 2);
                },
            )
            .map_err(|e| {
                EmulationError::EmulationError(format!("Failed to add code hook: {:?}", e))
            })?;

        debug!("Syscall hook setup");
        Ok(())
    }

    /// Initialize CPU state for program execution
    pub fn init_cpu_state(&mut self, entry_point: u32, stack_ptr: u32) -> EmulationResult<()> {
        // Set instruction pointer to entry point
        self.set_register(RegisterX86::EIP, entry_point)?;

        // Set stack pointer
        self.set_register(RegisterX86::ESP, stack_ptr)?;

        // Set base pointer
        self.set_register(RegisterX86::EBP, stack_ptr)?;

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
