use crate::binary_loader::BinaryInfo;
use crate::cpu::CpuEmulator;
use crate::emulator::{EmulationContext, TraceConfig};
use crate::errors::EmulationResult;
use crate::filesystem::VirtualFileSystem;
use crate::libsystem;
use crate::memory::MemoryManager;
use crate::syscall::SyscallHandler;
use log::info;
use std::cell::RefCell;
use std::rc::Rc;

/// Represents an emulated i386 process
pub struct Process {
    binary: BinaryInfo,
    #[allow(dead_code)]
    memory_manager: MemoryManager,
    syscall_handler: SyscallHandler,
    filesystem: Rc<RefCell<VirtualFileSystem>>,
    cpu: CpuEmulator,
    #[allow(dead_code)]
    pid: u32,
    trace_config: TraceConfig,
}

impl Process {
    pub fn new(binary: BinaryInfo, emulation_ctx: &mut EmulationContext) -> EmulationResult<Self> {
        emulation_ctx.initialize()?;

        let trace_config = emulation_ctx.trace();
        let syscall_handler = SyscallHandler::new_with_trace(trace_config.syscalls);

        let mut cpu = CpuEmulator::new()?;
        let total_size = 0x80000000u32;
        cpu.map_memory(0x0, total_size)?;

        let memory_manager = MemoryManager::new(total_size as usize);
        let filesystem = Rc::new(RefCell::new(VirtualFileSystem::new()));

        Ok(Process {
            binary,
            memory_manager,
            syscall_handler,
            filesystem,
            cpu,
            pid: std::process::id(),
            trace_config,
        })
    }

    fn load_binary_into_memory(&mut self) -> EmulationResult<()> {
        info!("Loading binary segments into memory");

        for segment in &self.binary.segments {
            if segment.vsize == 0 {
                continue;
            }
            info!(
                "Loading segment '{}': vaddr=0x{:x} size=0x{:x}",
                segment.name, segment.vaddr, segment.vsize
            );

            let mut data = vec![0u8; segment.vsize as usize];
            if segment.filesize > 0 {
                let copy_size = segment.filesize.min(segment.vsize) as usize;
                let file_off = segment.fileoff as usize;
                let file_end = file_off + copy_size;
                if file_end <= self.binary.raw.len() {
                    data[..copy_size].copy_from_slice(&self.binary.raw[file_off..file_end]);
                } else {
                    let copy_len = self.binary.raw.len().min(data.len());
                    data[..copy_len].copy_from_slice(&self.binary.raw[..copy_len]);
                }
            }
            self.cpu.write_memory(segment.vaddr, &data)?;
        }

        for section in &self.binary.sections {
            if section.size > 0 && !section.data.is_empty() {
                info!(
                    "Writing section {} at 0x{:x} (size=0x{:x})",
                    section.name, section.addr, section.size
                );
                self.cpu.write_memory(section.addr, &section.data)?;
            }
        }

        // Fallback: if entry bytes are all zeros, patch from raw file.
        let entry = self.binary.entry_point as usize;
        if entry + 16 <= self.binary.raw.len() {
            let current = self.cpu.read_memory(self.binary.entry_point, 16)?;
            if current.iter().all(|b| *b == 0) {
                let patch_len = 256usize.min(self.binary.raw.len() - entry);
                self.cpu.write_memory(
                    self.binary.entry_point,
                    &self.binary.raw[entry..entry + patch_len],
                )?;
            }
        }

        info!("Binary loaded into memory");
        Ok(())
    }

    /// Build the initial stack.
    ///
    /// If `ret_addr` is `Some(addr)` the stack starts with that return address
    /// (direct-to-main call convention for LC_MAIN binaries).  If `None`, the
    /// stack starts directly with `argc` (crt0 will push the return address
    /// itself via `call _main`).
    ///
    /// Layout (`ret_addr = None`, crt0 convention):
    ///   ESP → [argc, argv*, envp*]
    ///
    /// Layout (`ret_addr = Some(x)`, direct-to-main):
    ///   ESP → [x (fake ret), argc, argv*, envp*]
    fn setup_stack(&mut self, args: &[String], ret_addr: Option<u32>) -> EmulationResult<u32> {
        const STR_REGION: u32 = 0x7FFE0000;
        const PTRARRAY_REGION: u32 = 0x7FFD0000;
        const STACK_TOP: u32 = 0x7FFC0000;

        let argc = args.len() as u32;

        // Write null-terminated argument strings.
        let mut str_ptr = STR_REGION;
        let mut argv_ptrs: Vec<u32> = Vec::with_capacity(args.len());
        for arg in args {
            argv_ptrs.push(str_ptr);
            let mut bytes = arg.as_bytes().to_vec();
            bytes.push(0);
            self.cpu.write_memory(str_ptr, &bytes)?;
            str_ptr += bytes.len() as u32;
        }

        // Write argv pointer array then envp null sentinel.
        let mut arr_ptr = PTRARRAY_REGION;
        for &p in &argv_ptrs {
            self.cpu.write_memory(arr_ptr, &p.to_le_bytes())?;
            arr_ptr += 4;
        }
        self.cpu.write_memory(arr_ptr, &0u32.to_le_bytes())?; // argv null
        arr_ptr += 4;
        let envp_array_addr = arr_ptr;
        self.cpu.write_memory(arr_ptr, &0u32.to_le_bytes())?; // envp null

        // Build the frame; align ESP to 16 bytes.
        let frame_words: u32 = if ret_addr.is_some() { 4 } else { 3 };
        let esp = (STACK_TOP - frame_words * 4) & !0xF;

        let mut ptr = esp;
        if let Some(ra) = ret_addr {
            self.cpu.write_memory(ptr, &ra.to_le_bytes())?;
            ptr += 4;
        }
        self.cpu.write_memory(ptr, &argc.to_le_bytes())?;
        ptr += 4;
        self.cpu.write_memory(ptr, &PTRARRAY_REGION.to_le_bytes())?;
        ptr += 4;
        self.cpu.write_memory(ptr, &envp_array_addr.to_le_bytes())?;

        info!("Stack setup complete: ESP=0x{:x}", esp);
        Ok(esp)
    }

    pub fn execute(&mut self, args: &[String]) -> EmulationResult<()> {
        info!(
            "Executing {} (entry: 0x{:x}, is_main={}, dynamic={})",
            self.binary.name,
            self.binary.entry_point,
            self.binary.entry_is_main,
            self.binary.is_dynamic,
        );

        self.load_binary_into_memory()?;

        // ── INT 0x80 / instruction-trace hooks ───────────────────────────────
        self.cpu.setup_syscall_hook(
            self.syscall_handler,
            Rc::clone(&self.filesystem),
            self.trace_config.instructions,
        )?;

        // ── libSystem trampoline (dynamic binaries) ───────────────────────────
        let direct_ret_addr: Option<u32> = if let Some(ref bindings) = self.binary.dyld_bindings {
            let trampoline = libsystem::Trampoline::build(bindings);

            // Collect (ptr_slot_addr, trampoline_addr) patches before any borrows.
            let patches: Vec<(u32, u32)> = bindings
                .imports
                .iter()
                .filter_map(|imp| {
                    trampoline
                        .addr_for_binding(&imp.name)
                        .map(|ta| (imp.ptr_addr, ta))
                })
                .collect();

            let exit_addr = trampoline.exit_addr();

            // Install trampoline hook.
            self.cpu
                .setup_trampoline_hook(Rc::clone(&self.filesystem), trampoline)?;

            // Fill pointer slots in guest memory.
            for (slot, taddr) in patches {
                info!(
                    "  binding slot 0x{:x} → trampoline 0x{:x}",
                    slot, taddr
                );
                let _ = self.cpu.write_memory(slot, &taddr.to_le_bytes());
            }

            if self.binary.entry_is_main {
                Some(exit_addr)
            } else {
                None
            }
        } else {
            None
        };

        // ── stack ─────────────────────────────────────────────────────────────
        let stack_ptr = self.setup_stack(args, direct_ret_addr)?;

        // ── CPU start ─────────────────────────────────────────────────────────
        self.cpu
            .init_cpu_state(self.binary.entry_point, stack_ptr)?;

        let cpu_state = self.cpu.dump_registers()?;
        info!("Initial CPU state: {}", cpu_state);

        // Intercept host SIGINT / SIGTERM so Ctrl-C shuts down cleanly.
        crate::syscall::install_signal_handlers();

        info!("Starting emulation");
        self.cpu.execute(0)?;

        info!("Process completed");
        let final_state = self.cpu.dump_registers()?;
        info!("Final CPU state: {}", final_state);
        Ok(())
    }
}
