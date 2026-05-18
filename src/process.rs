use crate::binary_loader::BinaryInfo;
use crate::cpu::CpuEmulator;
use crate::emulator::{EmulationContext, TraceConfig};
use crate::errors::EmulationResult;
use crate::filesystem::VirtualFileSystem;
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
    /// Create a new process from a binary
    pub fn new(binary: BinaryInfo, emulation_ctx: &mut EmulationContext) -> EmulationResult<Self> {
        emulation_ctx.initialize()?;

        let trace_config = emulation_ctx.trace();
        let syscall_handler = SyscallHandler::new_with_trace(trace_config.syscalls);

        let mut cpu = CpuEmulator::new()?;

        let total_size = 0x80000000u32; // 2GB virtual address space
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

    /// Load binary code and data into emulated memory
    fn load_binary_into_memory(&mut self) -> EmulationResult<()> {
        info!("Loading binary segments into memory");

        for segment in &self.binary.segments {
            // Skip empty segments
            if segment.vsize == 0 {
                continue;
            }

            info!(
                "Loading segment: vaddr=0x{:x}, size=0x{:x}",
                segment.vaddr, segment.vsize
            );

            // Create a zero-filled buffer for the segment
            let mut data = vec![0u8; segment.vsize as usize];

            // If there's file data, copy it in from the original binary raw bytes
            if segment.filesize > 0 {
                let copy_size = std::cmp::min(segment.filesize, segment.vsize) as usize;
                if copy_size > 0 {
                    let file_off = segment.fileoff as usize;
                    let file_end = file_off + copy_size;
                    if file_end <= self.binary.raw.len() {
                        data[..copy_size].copy_from_slice(&self.binary.raw[file_off..file_end]);
                    } else {
                        // Fallback: if goblin reported odd file ranges, copy the whole file at segment base.
                        let copy_len = std::cmp::min(self.binary.raw.len(), data.len());
                        data[..copy_len].copy_from_slice(&self.binary.raw[..copy_len]);
                    }
                }
            }

            // Write segment into emulated memory
            self.cpu.write_memory(segment.vaddr, &data)?;
        }

        // Write sections (actual file-backed data) into memory at their addresses
        for section in &self.binary.sections {
            if section.size > 0 && !section.data.is_empty() {
                info!(
                    "Writing section {} at 0x{:x} (size=0x{:x})",
                    section.name, section.addr, section.size
                );
                self.cpu.write_memory(section.addr, &section.data)?;
            }
        }

        // Fallback for minimalist binaries with inconsistent segment metadata:
        // if entry bytes are all zeros but raw file has data at that offset, patch it in.
        let entry = self.binary.entry_point as usize;
        if entry + 16 <= self.binary.raw.len() {
            let current = self.cpu.read_memory(self.binary.entry_point, 16)?;
            if current.iter().all(|b| *b == 0) {
                let patch_len = std::cmp::min(256usize, self.binary.raw.len() - entry);
                self.cpu.write_memory(
                    self.binary.entry_point,
                    &self.binary.raw[entry..entry + patch_len],
                )?;
            }
        }

        info!("Binary loaded into memory");
        Ok(())
    }

    /// Setup the initial stack per the i386 SysV ABI.
    ///
    /// Layout at ESP on _start entry:
    ///   [esp+0]  argc (u32)
    ///   [esp+4]  argv  (char** → argv_array)
    ///   [esp+8]  envp  (char** → envp_array, currently empty)
    ///
    /// argv_array lives at PTRARRAY_REGION:
    ///   argv[0], argv[1], ..., NULL
    ///   NULL  (envp sentinel immediately after)
    ///
    /// String data lives at STR_REGION.
    ///
    /// After crt0's `call _main`, _main sees argc at [esp+4] and argv at [esp+8].
    fn setup_stack(&mut self, args: &[String]) -> EmulationResult<u32> {
        const STR_REGION: u32 = 0x7FFE0000;
        const PTRARRAY_REGION: u32 = 0x7FFD0000;
        const STACK_TOP: u32 = 0x7FFC0000;

        let argc = args.len() as u32;

        // 1. Write null-terminated argument strings.
        let mut str_ptr = STR_REGION;
        let mut argv_ptrs: Vec<u32> = Vec::with_capacity(args.len());
        for arg in args {
            argv_ptrs.push(str_ptr);
            let mut bytes = arg.as_bytes().to_vec();
            bytes.push(0);
            self.cpu.write_memory(str_ptr, &bytes)?;
            str_ptr += bytes.len() as u32;
        }

        // 2. Write argv pointer array followed by argv NULL then envp NULL.
        let mut arr_ptr = PTRARRAY_REGION;
        for &p in &argv_ptrs {
            self.cpu.write_memory(arr_ptr, &p.to_le_bytes())?;
            arr_ptr += 4;
        }
        self.cpu.write_memory(arr_ptr, &0u32.to_le_bytes())?; // argv NULL sentinel
        arr_ptr += 4;
        let envp_array_addr = arr_ptr;
        self.cpu.write_memory(arr_ptr, &0u32.to_le_bytes())?; // envp NULL sentinel

        // 3. Write [argc, argv**, envp**] at the stack top, 16-byte aligned.
        let mut esp = STACK_TOP - 12;
        esp &= !0xF;
        self.cpu.write_memory(esp, &argc.to_le_bytes())?;
        self.cpu.write_memory(esp + 4, &PTRARRAY_REGION.to_le_bytes())?;
        self.cpu.write_memory(esp + 8, &envp_array_addr.to_le_bytes())?;

        info!("Stack setup complete: ESP=0x{:x}", esp);
        Ok(esp)
    }

    /// Execute the process with given arguments
    pub fn execute(&mut self, args: &[String]) -> EmulationResult<()> {
        info!(
            "Executing process: {} (entry: 0x{:x})",
            self.binary.name, self.binary.entry_point
        );
        info!("Arguments: {:?}", args);

        // Load binary into memory
        self.load_binary_into_memory()?;

        // Setup syscall hook and optional instruction trace
        self.cpu.setup_syscall_hook(
            self.syscall_handler,
            Rc::clone(&self.filesystem),
            self.trace_config.instructions,
        )?;

        // Setup stack with arguments
        let stack_ptr = self.setup_stack(args)?;

        // Initialize CPU state; EBP=0 marks the outermost frame per the ABI.
        self.cpu
            .init_cpu_state(self.binary.entry_point, stack_ptr)?;

        // Dump initial CPU state for debugging
        let cpu_state = self.cpu.dump_registers()?;
        info!("Initial CPU state: {}", cpu_state);

        // Execute the binary
        info!("Starting emulation");
        self.cpu.execute(0)?;

        info!("Process completed successfully");
        let final_state = self.cpu.dump_registers()?;
        info!("Final CPU state: {}", final_state);
        Ok(())
    }
}
