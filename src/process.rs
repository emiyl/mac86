use crate::binary_loader::BinaryInfo;
use crate::cpu::CpuEmulator;
use crate::dyld;
use crate::emulator::{EmulationContext, TraceConfig};
use crate::errors::EmulationResult;
use crate::filesystem::VirtualFileSystem;
use crate::libsystem;
use crate::syscall::SyscallHandler;
use log::info;
use std::cell::RefCell;
use std::rc::Rc;

/// Represents an emulated i386 process.
pub struct Process {
    binary: BinaryInfo,
    syscall_handler: SyscallHandler,
    filesystem: Rc<RefCell<VirtualFileSystem>>,
    cpu: CpuEmulator,
    trace_config: TraceConfig,
}

impl Process {
    pub fn new(binary: BinaryInfo, emulation_ctx: &mut EmulationContext) -> EmulationResult<Self> {
        emulation_ctx.initialize()?;

        let trace_config = emulation_ctx.trace();
        let syscall_handler = SyscallHandler::new_with_trace(trace_config.syscalls);

        let mut cpu = CpuEmulator::new()?;
        // Map the full 2 GB i386 address space with read/write/execute permissions.
        cpu.map_memory(0x0, 0x8000_0000)?;

        let filesystem = Rc::new(RefCell::new(VirtualFileSystem::new()));

        Ok(Process {
            binary,
            syscall_handler,
            filesystem,
            cpu,
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

    /// Build the initial process stack.
    ///
    /// Two layouts depending on entry convention:
    ///
    /// **`ret_addr = None` — crt0/crt1 flat startup stack (Darwin ABI)**
    ///
    /// ```text
    /// ESP → argc
    ///        argv[0]   (string pointer directly on the stack)
    ///        argv[1]   …
    ///        NULL      (argv sentinel)
    ///        NULL      (envp sentinel)
    ///        NULL      (apple[] sentinel)
    /// ```
    ///
    /// crt1's `_start` computes `argv = leal 0x8(%ebp)` after `push $0;
    /// mov %esp,%ebp`, giving the address of the `argv[0]` slot — i.e. a
    /// correct `char**` with no extra indirection.
    ///
    /// **`ret_addr = Some(x)` — direct-to-`main()` (LC_MAIN)**
    ///
    /// ```text
    /// ESP → x          (fake return address = exit trampoline)
    ///        argc
    ///        argv**    (pointer to pointer array at PTRARRAY_REGION)
    ///        envp**
    /// ```
    fn setup_stack(&mut self, args: &[String], ret_addr: Option<u32>) -> EmulationResult<u32> {
        const STR_REGION: u32 = 0x7FFE_0000;
        const PTRARRAY_REGION: u32 = 0x7FFD_0000;
        const STACK_TOP: u32 = 0x7FFC_0000;

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

        let esp = if let Some(ret) = ret_addr {
            // LC_MAIN: put argv pointers in a separate array, build cdecl frame.
            let mut arr = PTRARRAY_REGION;
            for &p in &argv_ptrs {
                self.cpu.write_memory(arr, &p.to_le_bytes())?;
                arr += 4;
            }
            self.cpu.write_memory(arr, &0u32.to_le_bytes())?; // argv null
            arr += 4;
            let envp_addr = arr;
            self.cpu.write_memory(arr, &0u32.to_le_bytes())?; // envp null

            let esp = (STACK_TOP - 16) & !0xF;
            self.cpu.write_memory(esp, &ret.to_le_bytes())?;
            self.cpu.write_memory(esp + 4, &argc.to_le_bytes())?;
            self.cpu
                .write_memory(esp + 8, &PTRARRAY_REGION.to_le_bytes())?;
            self.cpu.write_memory(esp + 12, &envp_addr.to_le_bytes())?;
            esp
        } else {
            // crt0/crt1: flat startup stack — argv pointers laid out directly.
            // Words: argc(1) + argv[0..n](n) + argv_null(1) + envp_null(1) + apple_null(1)
            let words = argc + 4;
            let esp = (STACK_TOP - words * 4) & !0xF;
            let mut ptr = esp;
            self.cpu.write_memory(ptr, &argc.to_le_bytes())?;
            ptr += 4;
            for &p in &argv_ptrs {
                self.cpu.write_memory(ptr, &p.to_le_bytes())?;
                ptr += 4;
            }
            self.cpu.write_memory(ptr, &0u32.to_le_bytes())?;
            ptr += 4; // argv null
            self.cpu.write_memory(ptr, &0u32.to_le_bytes())?;
            ptr += 4; // envp null
            self.cpu.write_memory(ptr, &0u32.to_le_bytes())?; // apple null
            esp
        };

        info!("Stack setup complete: ESP=0x{:x}", esp);
        Ok(esp)
    }

    pub fn execute(&mut self, user_args: &[String]) -> EmulationResult<()> {
        // argv[0] is the binary name; user_args are argv[1..].
        let mut args: Vec<String> = Vec::with_capacity(user_args.len() + 1);
        args.push(self.binary.name.clone());
        args.extend_from_slice(user_args);

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

        // ── rebase pass (slide = 0, so all rebase ops are no-ops in value) ──────
        if self.binary.is_dynamic {
            if let Ok(macho) = goblin::mach::MachO::parse(&self.binary.raw, 0) {
                let rebases = dyld::parse_rebases(&macho, &self.binary.raw);
                if !rebases.is_empty() {
                    info!("Rebase: {} pointer entries (slide=0, no-op)", rebases.len());
                }
            }
        }

        // ── libSystem trampoline (dynamic binaries) ───────────────────────────
        let direct_ret_addr: Option<u32> = if let Some(ref bindings) = self.binary.dyld_bindings {
            let trampoline = libsystem::Trampoline::build(bindings);

            // Build dlsym lookup map and store it in the VFS before borrowing.
            let sym_map = trampoline.symbol_map();

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
            self.cpu
                .setup_trampoline_hook(Rc::clone(&self.filesystem), trampoline)?;

            // Initialize common libSystem globals used by BSD getopt.
            self.cpu
                .write_memory(libsystem::OPTIND_STORAGE_ADDR, &1u32.to_le_bytes())?;
            self.cpu
                .write_memory(libsystem::OPTARG_STORAGE_ADDR, &0u32.to_le_bytes())?;

            // Initialize __stdinp/__stdoutp/__stderrp storage with fake FILE* values.
            self.cpu.write_memory(
                libsystem::STDINP_STORAGE,
                &libsystem::STDIN_FILE_PTR.to_le_bytes(),
            )?;
            self.cpu.write_memory(
                libsystem::STDOUTP_STORAGE,
                &libsystem::STDOUT_FILE_PTR.to_le_bytes(),
            )?;
            self.cpu.write_memory(
                libsystem::STDERRP_STORAGE,
                &libsystem::STDERR_FILE_PTR.to_le_bytes(),
            )?;

            self.filesystem.borrow_mut().trampoline_map = sym_map;

            for (slot, taddr) in patches {
                info!("  binding 0x{:x} → trampoline 0x{:x}", slot, taddr);
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
        let stack_ptr = self.setup_stack(&args, direct_ret_addr)?;

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
