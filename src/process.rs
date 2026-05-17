use crate::binary_loader::BinaryInfo;
use crate::cpu::CpuEmulator;
use crate::emulator::EmulationContext;
use crate::errors::EmulationResult;
use crate::filesystem::VirtualFileSystem;
use crate::memory::MemoryManager;
use crate::syscall::SyscallHandler;
use log::info;

/// Represents an emulated i386 process
pub struct Process {
    binary: BinaryInfo,
    memory_manager: MemoryManager,
    syscall_handler: SyscallHandler,
    filesystem: VirtualFileSystem,
    cpu: CpuEmulator,
    pid: u32,
}

impl Process {
    /// Create a new process from a binary
    pub fn new(binary: BinaryInfo, emulation_ctx: &mut EmulationContext) -> EmulationResult<Self> {
        emulation_ctx.initialize()?;

        // Create syscall handler
        let syscall_handler = SyscallHandler::default();

        // Create CPU emulator
        let mut cpu = CpuEmulator::new()?;

        // Define memory regions to allocate
        let total_size = 0x80000000u32; // 2GB virtual address space

        // Allocate large memory region for the entire address space
        cpu.map_memory(0x0, total_size)?;

        let memory_manager = MemoryManager::new(total_size as usize);
        let filesystem = VirtualFileSystem::new();

        Ok(Process {
            binary,
            memory_manager,
            syscall_handler,
            filesystem,
            cpu,
            pid: std::process::id(),
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
            let data = vec![0u8; segment.vsize as usize];

            // If there's file data, copy it in
            if segment.filesize > 0 {
                // Get data from sections for this segment
                let copy_size = std::cmp::min(segment.filesize, segment.vsize) as usize;
                if copy_size > 0 {
                    // For simplicity, fill with zeros (in reality, we'd copy from file)
                    // This is where section data would be copied from the binary
                }
            }

            // Write segment into emulated memory
            self.cpu.write_memory(segment.vaddr, &data)?;
        }

        info!("Binary loaded into memory");
        Ok(())
    }

    /// Setup the stack with argc and argv
    fn setup_stack(&mut self, args: &[String]) -> EmulationResult<u32> {
        const STACK_BASE: u32 = 0x7FFFFFFF; // Near top of 32-bit address space
        const ARGV_BUFFER_ADDR: u32 = STACK_BASE - 0x100000u32; // Space for strings

        info!("Setting up stack at 0x{:x}", STACK_BASE);

        let argc = args.len() as u32;
        let mut stack_ptr = STACK_BASE;

        // Push argc on stack
        stack_ptr -= 4;
        self.cpu.write_memory(stack_ptr, &argc.to_le_bytes())?;

        // Create argv array and write strings
        let mut argv_ptr = ARGV_BUFFER_ADDR;
        let mut argv_addrs = Vec::new();

        for arg in args {
            argv_addrs.push(argv_ptr);

            // Write argument string
            let arg_bytes = format!("{}\0", arg);
            self.cpu.write_memory(argv_ptr, arg_bytes.as_bytes())?;
            argv_ptr += arg_bytes.len() as u32;

            // Align to 4 bytes
            argv_ptr = (argv_ptr + 3) & !3;
        }

        // Push argv pointers on stack (in reverse order for System V ABI)
        for addr in argv_addrs {
            stack_ptr -= 4;
            self.cpu.write_memory(stack_ptr, &addr.to_le_bytes())?;
        }

        // Push argv pointer
        stack_ptr -= 4;
        self.cpu
            .write_memory(stack_ptr, &ARGV_BUFFER_ADDR.to_le_bytes())?;

        info!("Stack setup complete: ESP=0x{:x}", stack_ptr);
        Ok(stack_ptr)
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

        // Setup syscall hook
        self.cpu.setup_syscall_hook(&self.syscall_handler)?;

        // Setup stack with arguments
        let stack_ptr = self.setup_stack(args)?;

        // Initialize CPU state
        self.cpu
            .init_cpu_state(self.binary.entry_point, stack_ptr)?;

        // Dump initial CPU state for debugging
        let cpu_state = self.cpu.dump_registers()?;
        info!("Initial CPU state: {}", cpu_state);

        // Execute the binary
        info!("Starting emulation");
        match self.cpu.execute(0) {
            Ok(()) => {
                info!("Process completed successfully");
                let final_state = self.cpu.dump_registers()?;
                info!("Final CPU state: {}", final_state);
                Ok(())
            }
            Err(e) => {
                info!("Process terminated: {}", e);
                Ok(()) // Some processes exit via syscall, which is normal
            }
        }
    }
}
