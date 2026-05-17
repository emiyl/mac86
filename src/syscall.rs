use crate::errors::{EmulationError, EmulationResult};

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

/// i386 macOS syscall handler
pub struct SyscallHandler;

impl SyscallHandler {
    pub fn new() -> Self {
        SyscallHandler
    }

    /// Handle a syscall based on number
    pub fn handle_syscall(&self, args: SyscallArgs) -> EmulationResult<u32> {
        match args.number {
            1 => {
                // exit(status)
                log::info!("exit({})", args.arg0);
                std::process::exit(args.arg0 as i32);
            }
            3 => {
                // read(fd, buf, count)
                log::debug!("read({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
                Ok(0) // Simplified: no data read
            }
            4 => {
                // write(fd, buf, count)
                log::debug!("write({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
                Ok(args.arg2 as u32) // Simplified: assume write succeeds
            }
            5 => {
                // open(path, flags)
                log::debug!("open(0x{:x}, {})", args.arg0, args.arg1);
                Ok(3) // Return file descriptor
            }
            6 => {
                // close(fd)
                log::debug!("close({})", args.arg0);
                Ok(0)
            }
            18 => {
                // stat(path, sb)
                log::debug!("stat(0x{:x}, 0x{:x})", args.arg0, args.arg1);
                Ok(0)
            }
            20 => {
                // getpid()
                let pid = std::process::id();
                log::debug!("getpid() = {}", pid);
                Ok(pid)
            }
            24 => {
                // getuid()
                log::debug!("getuid()");
                Ok(0) // Simplified: return root
            }
            _ => {
                log::warn!("Unimplemented syscall: {}", args.number);
                Err(EmulationError::SyscallError(format!(
                    "Unimplemented syscall: {}",
                    args.number
                )))
            }
        }
    }

    /// Setup default handlers (kept for backward compatibility)
    pub fn setup_defaults(&mut self) {
        // No-op: handlers are now built into handle_syscall
    }
}

impl Default for SyscallHandler {
    fn default() -> Self {
        Self::new()
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
