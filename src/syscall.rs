use crate::errors::{EmulationError, EmulationResult};
use crate::filesystem::{FileStat, VirtualFileSystem};
use std::path::Path;
use unicorn_engine::Unicorn;

/// Arguments passed to a syscall (register-based convention: EAX=num, EBX..EBP=args)
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
#[derive(Clone, Copy)]
pub struct SyscallHandler {
    pub trace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallOutcome {
    Continue,
    Exit(i32),
}

impl SyscallHandler {
    pub fn new() -> Self {
        SyscallHandler { trace: false }
    }

    pub fn new_with_trace(trace: bool) -> Self {
        SyscallHandler { trace }
    }

    /// Handle a syscall with access to emulated memory and virtual filesystem.
    ///
    /// Returns `(outcome, retval)` where `retval` is a 64-bit value. For most
    /// syscalls the high 32 bits are zero. `lseek` places the full 64-bit
    /// offset in `retval`; cpu.rs writes the low word to EAX and high word to EDX.
    pub fn handle_syscall(
        &self,
        emu: &mut Unicorn<'_, ()>,
        fs: &mut VirtualFileSystem,
        args: SyscallArgs,
    ) -> EmulationResult<(SyscallOutcome, u64)> {
        if self.trace {
            println!(
                "[syscall] {} ({}, {}, {}, {}, {}, {})",
                args.number, args.arg0, args.arg1, args.arg2, args.arg3, args.arg4, args.arg5
            );
        }

        match args.number {
            // ── exit ─────────────────────────────────────────────────────────
            1 => {
                log::info!("exit({})", args.arg0);
                Ok((SyscallOutcome::Exit(args.arg0 as i32), 0))
            }

            // ── read ─────────────────────────────────────────────────────────
            3 => {
                log::debug!("read({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
                let data = fs.read_bytes(args.arg0, args.arg2 as usize)?;
                if !data.is_empty() {
                    emu.mem_write(args.arg1 as u64, &data).map_err(|e| {
                        EmulationError::MemoryError(format!("read mem_write: {:?}", e))
                    })?;
                }
                Ok((SyscallOutcome::Continue, data.len() as u64))
            }

            // ── write ────────────────────────────────────────────────────────
            4 => {
                log::debug!("write({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
                let mut bytes = vec![0u8; args.arg2 as usize];
                if !bytes.is_empty() {
                    emu.mem_read(args.arg1 as u64, &mut bytes).map_err(|e| {
                        EmulationError::MemoryError(format!("write mem_read: {:?}", e))
                    })?;
                }
                let written = fs.write_bytes(args.arg0, &bytes)? as u64;
                Ok((SyscallOutcome::Continue, written))
            }

            // ── open ─────────────────────────────────────────────────────────
            5 => {
                let path = read_c_string(emu, args.arg0)?;
                log::debug!("open({:?}, 0x{:x})", path, args.arg1);
                let writable = (args.arg1 & 0x3) != 0; // O_WRONLY | O_RDWR
                match fs.open(Path::new(&path), writable) {
                    Ok(fd) => Ok((SyscallOutcome::Continue, fd as u64)),
                    Err(_) => {
                        log::debug!("open({:?}) -> ENOENT", path);
                        Ok((SyscallOutcome::Continue, u32::MAX as u64)) // -1
                    }
                }
            }

            // ── close ────────────────────────────────────────────────────────
            6 => {
                log::debug!("close({})", args.arg0);
                // Ignore close errors (bad-fd is non-fatal for the guest).
                let _ = fs.close(args.arg0);
                Ok((SyscallOutcome::Continue, 0))
            }

            // ── stat ─────────────────────────────────────────────────────────
            // macOS i386 BSD stat(2): stat(const char *path, struct stat *sb)
            18 => {
                log::debug!("stat(0x{:x}, 0x{:x})", args.arg0, args.arg1);
                let path = read_c_string(emu, args.arg0)?;
                match fs.stat_path(Path::new(&path)) {
                    Ok(st) => {
                        write_stat_struct(emu, args.arg1, &st)?;
                        Ok((SyscallOutcome::Continue, 0))
                    }
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)), // -1
                }
            }

            // ── getpid ───────────────────────────────────────────────────────
            20 => {
                let pid = std::process::id();
                log::debug!("getpid() = {}", pid);
                Ok((SyscallOutcome::Continue, pid as u64))
            }

            // ── getuid ───────────────────────────────────────────────────────
            24 => {
                log::debug!("getuid()");
                Ok((SyscallOutcome::Continue, 0))
            }

            // ── brk ──────────────────────────────────────────────────────────
            // brk(void *addr) — set program break; returns new break.
            45 => {
                log::debug!("brk(0x{:x})", args.arg0);
                let new_break = fs.brk(args.arg0);
                Ok((SyscallOutcome::Continue, new_break as u64))
            }

            // ── fstat ────────────────────────────────────────────────────────
            // macOS i386 BSD fstat(2): fstat(int fd, struct stat *sb)
            62 => {
                log::debug!("fstat({}, 0x{:x})", args.arg0, args.arg1);
                match fs.fstat_fd(args.arg0) {
                    Ok(st) => {
                        write_stat_struct(emu, args.arg1, &st)?;
                        Ok((SyscallOutcome::Continue, 0))
                    }
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                }
            }

            // ── munmap ───────────────────────────────────────────────────────
            // munmap(void *addr, size_t len) — we track nothing, already mapped.
            73 => {
                log::debug!("munmap(0x{:x}, {})", args.arg0, args.arg1);
                Ok((SyscallOutcome::Continue, 0))
            }

            // ── mmap ─────────────────────────────────────────────────────────
            // mmap(addr, len, prot, flags, fd, off_lo)
            // We support anonymous mappings only (fd = -1 or flags & MAP_ANON).
            // MAP_ANONYMOUS on macOS i386 = 0x1000.
            197 => {
                let len = args.arg1;
                let flags = args.arg3;
                let fd_arg = args.arg4 as i32;
                log::debug!(
                    "mmap(0x{:x}, {}, prot={}, flags=0x{:x}, fd={})",
                    args.arg0, len, args.arg2, flags, fd_arg
                );
                let is_anon = (flags & 0x1000) != 0 || fd_arg == -1;
                if is_anon {
                    match fs.mmap_anon(len) {
                        Ok(addr) => Ok((SyscallOutcome::Continue, addr as u64)),
                        Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                    }
                } else {
                    // File-backed mmap not yet supported.
                    log::warn!("mmap: file-backed mapping not supported, returning MAP_FAILED");
                    Ok((SyscallOutcome::Continue, u32::MAX as u64))
                }
            }

            // ── lseek ────────────────────────────────────────────────────────
            // Register convention for our 32-bit lseek:
            //   arg0 (EBX) = fd
            //   arg1 (ECX) = offset_low  (low 32 bits of i64 offset)
            //   arg2 (EDX) = offset_high (high 32 bits of i64 offset)
            //   arg3 (ESI) = whence
            // Returns 64-bit new offset in EDX:EAX.
            199 => {
                let fd = args.arg0;
                let offset = (args.arg1 as u64 | ((args.arg2 as u64) << 32)) as i64;
                let whence = args.arg3 as i32;
                log::debug!("lseek({}, {}, {})", fd, offset, whence);
                match fs.seek(fd, offset, whence) {
                    Ok(new_off) => Ok((SyscallOutcome::Continue, new_off)),
                    Err(_) => Ok((SyscallOutcome::Continue, u64::MAX)), // -1
                }
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
    pub fn setup_defaults(&mut self) {}
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn read_c_string(emu: &mut Unicorn<'_, ()>, addr: u32) -> EmulationResult<String> {
    const MAX_PATH_LEN: usize = 4096;
    let mut out = Vec::new();
    for i in 0..MAX_PATH_LEN {
        let mut byte = [0u8; 1];
        emu.mem_read((addr as u64) + (i as u64), &mut byte)
            .map_err(|e| EmulationError::MemoryError(format!("path read: {:?}", e)))?;
        if byte[0] == 0 {
            break;
        }
        out.push(byte[0]);
    }
    String::from_utf8(out)
        .map_err(|e| EmulationError::SyscallError(format!("Invalid UTF-8 path: {}", e)))
}

/// Write a macOS i386 `struct stat` (96 bytes) to guest memory.
///
/// Layout (offsets in bytes, all little-endian):
///   0   st_dev     i32
///   4   st_ino     u32
///   8   st_mode    u16
///  10   st_nlink   u16
///  12   st_uid     u32
///  16   st_gid     u32
///  20   st_rdev    i32
///  24   st_atimespec  8  (tv_sec u32 + tv_nsec u32)
///  32   st_mtimespec  8
///  40   st_ctimespec  8
///  48   st_size    i64
///  56   st_blocks  i64
///  64   st_blksize i32
///  68   st_flags   u32
///  72   st_gen     u32
///  76   st_lspare  i32
///  80   st_qspare  16 (2 × i64)
/// total = 96 bytes
fn write_stat_struct(
    emu: &mut Unicorn<'_, ()>,
    addr: u32,
    stat: &FileStat,
) -> EmulationResult<()> {
    let mut buf = [0u8; 96];

    buf[0..4].copy_from_slice(&1i32.to_le_bytes()); // st_dev
    buf[4..8].copy_from_slice(&1u32.to_le_bytes()); // st_ino
    let mode: u16 = if stat.is_dir {
        0o040_755
    } else if stat.is_regular {
        0o100_644
    } else {
        0o020_666 // character device (e.g. stdin/stdout)
    };
    buf[8..10].copy_from_slice(&mode.to_le_bytes()); // st_mode
    buf[10..12].copy_from_slice(&1u16.to_le_bytes()); // st_nlink
    // uid, gid, rdev, timestamps — all zero
    buf[48..56].copy_from_slice(&(stat.size as i64).to_le_bytes()); // st_size
    let blocks = ((stat.size + 511) / 512) as i64;
    buf[56..64].copy_from_slice(&blocks.to_le_bytes()); // st_blocks
    buf[64..68].copy_from_slice(&4096i32.to_le_bytes()); // st_blksize

    emu.mem_write(addr as u64, &buf)
        .map_err(|e| EmulationError::MemoryError(format!("stat write: {:?}", e)))
}

impl Default for SyscallHandler {
    fn default() -> Self {
        Self::new()
    }
}

// ── i386 macOS BSD syscall numbers ───────────────────────────────────────────
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
    pub const BRK: u32 = 45;
    pub const FSTAT: u32 = 62;
    pub const MUNMAP: u32 = 73;
    pub const MMAP: u32 = 197;
    pub const LSEEK: u32 = 199;
    pub const EXECVE: u32 = 59;
}
