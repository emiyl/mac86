use crate::errors::{EmulationError, EmulationResult};
use std::collections::HashMap;

/// i386 macOS syscall handler
pub struct SyscallHandler {
    /// Mapping of syscall numbers to handlers
    handlers: HashMap<u32, Box<dyn Fn(SyscallArgs) -> EmulationResult<u32>>>,
}

/// Arguments passed to a syscall
#[derive(Debug, Clone)]
pub struct SyscallArgs {
    pub number: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub arg2: u32,
    pub arg3: u32,
    pub arg4: u32,
    pub arg5: u32,
}

impl SyscallHandler {
    pub fn new() -> Self {
        SyscallHandler {
            handlers: HashMap::new(),
        }
    }

    /// Register a syscall handler
    pub fn register<F>(&mut self, syscall_num: u32, handler: F)
    where
        F: Fn(SyscallArgs) -> EmulationResult<u32> + 'static,
    {
        self.handlers.insert(syscall_num, Box::new(handler));
    }

    /// Handle a syscall
    pub fn handle_syscall(&self, args: SyscallArgs) -> EmulationResult<u32> {
        let handler = self
            .handlers
            .get(&args.number)
            .ok_or_else(|| EmulationError::SyscallError(format!(
                "Unimplemented syscall: {}",
                args.number
            )))?;

        handler(args)
    }

    /// Initialize default i386 macOS syscalls
    pub fn setup_defaults(&mut self) {
        // Exit syscall (1)
        self.register(1, |args| {
            log::info!("exit({})", args.arg0);
            std::process::exit(args.arg0 as i32);
        });

        // Write syscall (4)
        self.register(4, |args| {
            log::debug!("write({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
            Ok(args.arg2 as u32) // Simplified: assume write succeeds
        });

        // Read syscall (3)
        self.register(3, |args| {
            log::debug!("read({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
            Ok(0) // Simplified: no data read
        });

        // Open syscall (5)
        self.register(5, |args| {
            log::debug!("open(0x{:x}, {})", args.arg0, args.arg1);
            Ok(3) // Return file descriptor
        });

        // Close syscall (6)
        self.register(6, |args| {
            log::debug!("close({})", args.arg0);
            Ok(0)
        });

        // Stat syscall (18)
        self.register(18, |args| {
            log::debug!("stat(0x{:x}, 0x{:x})", args.arg0, args.arg1);
            Ok(0)
        });

        // Getpid syscall (20)
        self.register(20, |_args| {
            let pid = std::process::id();
            log::debug!("getpid() = {}", pid);
            Ok(pid)
        });

        // Getuid syscall (24)
        self.register(24, |_args| {
            log::debug!("getuid()");
            Ok(0) // Simplified: return root
        });
    }
}

impl Default for SyscallHandler {
    fn default() -> Self {
        let mut handler = Self::new();
        handler.setup_defaults();
        handler
    }
}

// i386 macOS syscall numbers (BSD-style)
#[allow(dead_code)]
pub mod syscall_numbers {
    pub const EXIT: u32 = 1;
    pub const FORK: u32 = 2;
    pub const READ: u32 = 3;
    pub const WRITE: u32 = 4;
    pub const OPEN: u32 = 5;
    pub const CLOSE: u32 = 6;
    pub const STAT: u32 = 18;
    pub const GETPID: u32 = 20;
    pub const GETUID: u32 = 24;
    pub const EXECVE: u32 = 59;
}
