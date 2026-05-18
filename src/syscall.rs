use crate::errors::{EmulationError, EmulationResult};
use crate::filesystem::{FileStat, VirtualFileSystem};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use unicorn_engine::Unicorn;

/// Set by the host SIGINT / SIGTERM handler; checked at every syscall.
pub static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Install host signal handlers that set STOP_REQUESTED.
pub fn install_signal_handlers() {
    use nix::sys::signal::{signal, Signal, SigHandler};
    unsafe {
        let _ = signal(Signal::SIGINT, SigHandler::Handler(sigint_handler));
        let _ = signal(Signal::SIGTERM, SigHandler::Handler(sigint_handler));
    }
}

extern "C" fn sigint_handler(_sig: libc::c_int) {
    STOP_REQUESTED.store(true, Ordering::Relaxed);
}

/// Arguments to a syscall — register convention (EAX=num, EBX..EBP=args).
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

#[derive(Clone, Copy)]
pub struct SyscallHandler {
    pub trace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallOutcome {
    Continue,
    Exit(i32),
    /// Handler set PC/ESP directly — skip the normal `set_pc(addr+2)` advance.
    StateSet,
    /// Deliver a guest signal: cpu.rs sets up the handler frame then skips the INT.
    DeliverSignal { handler: u32 },
}

impl SyscallHandler {
    pub fn new() -> Self {
        SyscallHandler { trace: false }
    }
    pub fn new_with_trace(trace: bool) -> Self {
        SyscallHandler { trace }
    }
    pub fn setup_defaults(&mut self) {}

    /// Handle an INT 0x80 syscall.
    ///
    /// Returns `(outcome, retval)` where `retval` is 64-bit; the low 32 bits
    /// go to EAX and the high 32 bits go to EDX (used by lseek).
    pub fn handle_syscall(
        &self,
        emu: &mut Unicorn<'_, ()>,
        fs: &mut VirtualFileSystem,
        args: SyscallArgs,
    ) -> EmulationResult<(SyscallOutcome, u64)> {
        // Check for host SIGINT / SIGTERM before every syscall.
        if STOP_REQUESTED.swap(false, Ordering::Relaxed) {
            log::info!("Host SIGINT/SIGTERM received");
            // If the guest registered a SIGINT handler, invoke it.
            if let Some(&handler) = fs.threads.signal_handlers.get(&2) {
                return Ok((SyscallOutcome::DeliverSignal { handler }, 0));
            }
            return Ok((SyscallOutcome::Exit(130), 0));
        }

        if self.trace {
            println!(
                "[syscall] {} ({}, {}, {}, {}, {}, {})",
                args.number,
                args.arg0,
                args.arg1,
                args.arg2,
                args.arg3,
                args.arg4,
                args.arg5
            );
        }

        match args.number {
            // ── Phase 2 core ─────────────────────────────────────────────────
            1 => {
                log::info!("exit({})", args.arg0);
                Ok((SyscallOutcome::Exit(args.arg0 as i32), 0))
            }
            3 => {
                // read(fd, buf, count)
                log::debug!("read({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
                let data = fs.read_bytes(args.arg0, args.arg2 as usize)?;
                if !data.is_empty() {
                    emu.mem_write(args.arg1 as u64, &data).map_err(|e| {
                        EmulationError::MemoryError(format!("read mem_write: {:?}", e))
                    })?;
                }
                Ok((SyscallOutcome::Continue, data.len() as u64))
            }
            4 => {
                // write(fd, buf, count)
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
            5 => {
                // open(path, flags, mode)
                let path = read_c_string(emu, args.arg0)?;
                log::debug!("open({:?}, 0x{:x})", path, args.arg1);
                let writable = (args.arg1 & 0x3) != 0;
                match fs.open(Path::new(&path), writable) {
                    Ok(fd) => Ok((SyscallOutcome::Continue, fd as u64)),
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                }
            }
            6 => {
                // close(fd)
                log::debug!("close({})", args.arg0);
                let _ = fs.close(args.arg0);
                Ok((SyscallOutcome::Continue, 0))
            }
            18 => {
                // stat(path, sb)
                let path = read_c_string(emu, args.arg0)?;
                log::debug!("stat({:?}, 0x{:x})", path, args.arg1);
                match fs.stat_path(Path::new(&path)) {
                    Ok(st) => {
                        write_stat_struct(emu, args.arg1, &st)?;
                        Ok((SyscallOutcome::Continue, 0))
                    }
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                }
            }
            20 => {
                log::debug!("getpid()");
                Ok((SyscallOutcome::Continue, std::process::id() as u64))
            }
            24 => {
                log::debug!("getuid()");
                Ok((SyscallOutcome::Continue, 0))
            }
            45 => {
                // brk(addr)
                log::debug!("brk(0x{:x})", args.arg0);
                Ok((SyscallOutcome::Continue, fs.brk(args.arg0) as u64))
            }
            62 => {
                // fstat(fd, sb)
                log::debug!("fstat({}, 0x{:x})", args.arg0, args.arg1);
                match fs.fstat_fd(args.arg0) {
                    Ok(st) => {
                        write_stat_struct(emu, args.arg1, &st)?;
                        Ok((SyscallOutcome::Continue, 0))
                    }
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                }
            }
            73 => {
                // munmap(addr, len)
                log::debug!("munmap(0x{:x}, {})", args.arg0, args.arg1);
                Ok((SyscallOutcome::Continue, 0))
            }
            197 => {
                // mmap(addr, len, prot, flags, fd, off)
                let flags = args.arg3;
                let fd_arg = args.arg4 as i32;
                log::debug!("mmap(0x{:x}, {}, flags=0x{:x}, fd={})", args.arg0, args.arg1, flags, fd_arg);
                let is_anon = (flags & 0x1000) != 0 || fd_arg == -1;
                if is_anon {
                    match fs.mmap_anon(args.arg1) {
                        Ok(addr) => Ok((SyscallOutcome::Continue, addr as u64)),
                        Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                    }
                } else {
                    Ok((SyscallOutcome::Continue, u32::MAX as u64))
                }
            }
            199 => {
                // lseek(fd, offset_lo, offset_hi, whence) — register convention
                let fd = args.arg0;
                let offset = (args.arg1 as u64 | ((args.arg2 as u64) << 32)) as i64;
                let whence = args.arg3 as i32;
                log::debug!("lseek({}, {}, {})", fd, offset, whence);
                match fs.seek(fd, offset, whence) {
                    Ok(off) => Ok((SyscallOutcome::Continue, off)),
                    Err(_) => Ok((SyscallOutcome::Continue, u64::MAX)),
                }
            }

            // ── Phase 4 ──────────────────────────────────────────────────────

            25 => {
                // geteuid()
                Ok((SyscallOutcome::Continue, 0))
            }
            33 => {
                // access(path, amode)
                let path = read_c_string(emu, args.arg0)?;
                log::debug!("access({:?}, {})", path, args.arg1);
                let exists = std::path::Path::new(&path).exists();
                Ok((SyscallOutcome::Continue, if exists { 0 } else { u32::MAX as u64 }))
            }
            39 => {
                // getppid() — fake parent pid = 1
                Ok((SyscallOutcome::Continue, 1))
            }
            41 => {
                // dup(fd)
                log::debug!("dup({})", args.arg0);
                match fs.dup(args.arg0) {
                    Ok(new_fd) => Ok((SyscallOutcome::Continue, new_fd as u64)),
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                }
            }
            43 => {
                // getegid()
                Ok((SyscallOutcome::Continue, 0))
            }
            46 => {
                // sigaction(sig, act, oact)
                // struct sigaction { sa_handler, sa_mask, sa_flags }
                let sig = args.arg0;
                let act_ptr = args.arg1;
                log::debug!("sigaction({}, act=0x{:x})", sig, act_ptr);
                if act_ptr != 0 {
                    // sa_handler is the first field (u32 on i386)
                    let mut h = [0u8; 4];
                    let _ = emu.mem_read(act_ptr as u64, &mut h);
                    let handler = u32::from_le_bytes(h);
                    if handler > 1 {
                        // > 1 means a real function pointer (SIG_DFL=0, SIG_IGN=1)
                        fs.threads.signal_handlers.insert(sig, handler);
                    }
                }
                Ok((SyscallOutcome::Continue, 0))
            }
            47 => {
                // getgid()
                Ok((SyscallOutcome::Continue, 0))
            }
            48 => {
                // sigprocmask(how, set, oset) — stub
                log::debug!("sigprocmask({}, …)", args.arg0);
                // Clear oset if provided
                if args.arg2 != 0 {
                    let zero = 0u32.to_le_bytes();
                    let _ = emu.mem_write(args.arg2 as u64, &zero);
                }
                Ok((SyscallOutcome::Continue, 0))
            }
            54 => {
                // ioctl(fd, request, arg)
                log::debug!("ioctl({}, 0x{:x}, 0x{:x})", args.arg0, args.arg1, args.arg2);
                // TIOCGWINSZ = 0x40087468 (on macOS) — return a dummy terminal size
                if args.arg1 == 0x4008_7468 && args.arg2 != 0 {
                    // struct winsize { ws_row, ws_col, ws_xpixel, ws_ypixel } — each u16
                    let mut ws = [0u8; 8];
                    ws[0..2].copy_from_slice(&24u16.to_le_bytes()); // rows
                    ws[2..4].copy_from_slice(&80u16.to_le_bytes()); // cols
                    let _ = emu.mem_write(args.arg2 as u64, &ws);
                }
                Ok((SyscallOutcome::Continue, 0))
            }
            74 => {
                // mprotect(addr, len, prot) — no-op (whole space already Prot::ALL)
                log::debug!("mprotect(0x{:x}, {}, {})", args.arg0, args.arg1, args.arg2);
                Ok((SyscallOutcome::Continue, 0))
            }
            82 => {
                // getpgrp() — return our pid
                Ok((SyscallOutcome::Continue, std::process::id() as u64))
            }
            83 => {
                // setpgid() — stub
                Ok((SyscallOutcome::Continue, 0))
            }
            90 => {
                // dup2(from, to)
                log::debug!("dup2({}, {})", args.arg0, args.arg1);
                match fs.dup2(args.arg0, args.arg1) {
                    Ok(fd) => Ok((SyscallOutcome::Continue, fd as u64)),
                    Err(_) => Ok((SyscallOutcome::Continue, u32::MAX as u64)),
                }
            }
            92 => {
                // fcntl(fd, cmd, arg)
                let cmd = args.arg1;
                log::debug!("fcntl({}, {}, {})", args.arg0, cmd, args.arg2);
                let result: u64 = match cmd {
                    0 => 0, // F_DUPFD — simplified: return 0
                    1 | 2 => 0, // F_GETFD / F_SETFD
                    3 => 2,     // F_GETFL — O_RDWR
                    4 => 0,     // F_SETFL
                    _ => 0,
                };
                Ok((SyscallOutcome::Continue, result))
            }
            93 => {
                // select(nd, in, ou, ex, tv) — stub: pretend nothing ready
                log::debug!("select({}, …)", args.arg0);
                Ok((SyscallOutcome::Continue, 0))
            }
            116 => {
                // gettimeofday(timeval*, timezone*)
                log::debug!("gettimeofday(0x{:x}, 0x{:x})", args.arg0, args.arg1);
                if args.arg0 != 0 {
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let dur = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default();
                    let sec = dur.as_secs() as u32;
                    let usec = dur.subsec_micros();
                    let mut tv = [0u8; 8];
                    tv[0..4].copy_from_slice(&sec.to_le_bytes());
                    tv[4..8].copy_from_slice(&usec.to_le_bytes());
                    let _ = emu.mem_write(args.arg0 as u64, &tv);
                }
                Ok((SyscallOutcome::Continue, 0))
            }
            121 => {
                // writev(fd, iov, iovcnt)
                // struct iovec { iov_base: u32, iov_len: u32 }
                log::debug!("writev({}, 0x{:x}, {})", args.arg0, args.arg1, args.arg2);
                let mut total: u64 = 0;
                for i in 0..args.arg2 as usize {
                    let iov_ptr = args.arg1 + (i as u32) * 8;
                    let mut iov = [0u8; 8];
                    if emu.mem_read(iov_ptr as u64, &mut iov).is_err() {
                        break;
                    }
                    let base = u32::from_le_bytes(iov[0..4].try_into().unwrap_or_default());
                    let len = u32::from_le_bytes(iov[4..8].try_into().unwrap_or_default());
                    if len == 0 {
                        continue;
                    }
                    let mut buf = vec![0u8; len as usize];
                    if emu.mem_read(base as u64, &mut buf).is_err() {
                        break;
                    }
                    total += fs.write_bytes(args.arg0, &buf).unwrap_or(0) as u64;
                }
                Ok((SyscallOutcome::Continue, total))
            }
            194 => {
                // readlink(path, buf, bufsiz) — ENOENT for unknown paths
                log::debug!("readlink(0x{:x}, …)", args.arg0);
                Ok((SyscallOutcome::Continue, u32::MAX as u64))
            }
            202 => {
                // sysctl(name, namelen, oldp, oldlenp, newp, newlen)
                sysctl_handler(emu, args)
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
}

// ── sysctl ────────────────────────────────────────────────────────────────────

fn sysctl_handler(
    emu: &mut Unicorn<'_, ()>,
    args: SyscallArgs,
) -> EmulationResult<(SyscallOutcome, u64)> {
    // name is an array of `namelen` u32 values
    let namelen = args.arg1 as usize;
    if namelen == 0 || args.arg0 == 0 {
        return Ok((SyscallOutcome::Continue, u32::MAX as u64));
    }

    let mut name = vec![0u32; namelen];
    for (i, slot) in name.iter_mut().enumerate() {
        let mut buf = [0u8; 4];
        let _ = emu.mem_read(args.arg0 as u64 + (i as u64) * 4, &mut buf);
        *slot = u32::from_le_bytes(buf);
    }

    log::debug!("sysctl name={:?}", &name[..namelen.min(4)]);

    // oldp=args.arg2, oldlenp=args.arg3
    let oldp = args.arg2;
    let oldlenp = args.arg3;

    let write_bytes = |emu: &mut Unicorn<'_, ()>, data: &[u8]| {
        if oldp != 0 {
            let _ = emu.mem_write(oldp as u64, data);
        }
        if oldlenp != 0 {
            let len = data.len() as u32;
            let _ = emu.mem_write(oldlenp as u64, &len.to_le_bytes());
        }
    };

    let write_u32 = |emu: &mut Unicorn<'_, ()>, v: u32| {
        write_bytes(emu, &v.to_le_bytes());
    };

    // CTL_KERN=1, CTL_HW=6
    match (name.first().copied(), name.get(1).copied()) {
        (Some(1), Some(2)) => {
            // KERN_OSRELEASE
            let s = b"10.13.0\0";
            write_bytes(emu, s);
        }
        (Some(1), Some(4)) => {
            // KERN_VERSION
            let s = b"Darwin 17.0.0\0";
            write_bytes(emu, s);
        }
        (Some(1), Some(10)) => {
            // KERN_HOSTNAME
            let s = b"localhost\0";
            write_bytes(emu, s);
        }
        (Some(1), Some(15)) => {
            // KERN_PROCARGS — not supported
            return Ok((SyscallOutcome::Continue, u32::MAX as u64));
        }
        (Some(6), Some(3)) => {
            // HW_NCPU = 1
            write_u32(emu, 1);
        }
        (Some(6), Some(5)) => {
            // HW_PHYSMEM = 4 GB (fake)
            write_u32(emu, 0xFFFF_FFFF);
        }
        (Some(6), Some(24)) => {
            // HW_PAGESIZE = 4096
            write_u32(emu, 4096);
        }
        _ => {
            // Unknown — write 0
            write_u32(emu, 0);
        }
    }

    Ok((SyscallOutcome::Continue, 0))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_c_string(emu: &mut Unicorn<'_, ()>, addr: u32) -> EmulationResult<String> {
    let mut out = Vec::new();
    for i in 0usize..4096 {
        let mut byte = [0u8; 1];
        emu.mem_read((addr as u64) + i as u64, &mut byte)
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
/// Field offsets (all little-endian):
///   0   st_dev     i32    4   st_ino    u32    8  st_mode   u16
///  10   st_nlink   u16   12   st_uid    u32   16  st_gid    u32
///  20   st_rdev    i32   24   (padding 4)
///  28   st_atimespec 8   36   st_mtimespec 8   44  st_ctimespec 8
///  48   st_size    i64   56   st_blocks i64   64  st_blksize i32
///  68   st_flags   u32   72   st_gen    u32   76  st_lspare i32
///  80   st_qspare  16
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
        0o020_666 // char device (stdin/stdout/stderr)
    };
    buf[8..10].copy_from_slice(&mode.to_le_bytes());
    buf[10..12].copy_from_slice(&1u16.to_le_bytes()); // st_nlink
    buf[48..56].copy_from_slice(&(stat.size as i64).to_le_bytes());
    let blocks = ((stat.size + 511) / 512) as i64;
    buf[56..64].copy_from_slice(&blocks.to_le_bytes());
    buf[64..68].copy_from_slice(&4096i32.to_le_bytes());

    emu.mem_write(addr as u64, &buf)
        .map_err(|e| EmulationError::MemoryError(format!("stat write: {:?}", e)))
}

impl Default for SyscallHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
pub mod syscall_numbers {
    pub const EXIT: u32 = 1;
    pub const READ: u32 = 3;
    pub const WRITE: u32 = 4;
    pub const OPEN: u32 = 5;
    pub const CLOSE: u32 = 6;
    pub const STAT: u32 = 18;
    pub const GETPID: u32 = 20;
    pub const GETUID: u32 = 24;
    pub const GETEUID: u32 = 25;
    pub const ACCESS: u32 = 33;
    pub const GETPPID: u32 = 39;
    pub const DUP: u32 = 41;
    pub const GETEGID: u32 = 43;
    pub const SIGACTION: u32 = 46;
    pub const GETGID: u32 = 47;
    pub const SIGPROCMASK: u32 = 48;
    pub const BRK: u32 = 45;
    pub const IOCTL: u32 = 54;
    pub const EXECVE: u32 = 59;
    pub const FSTAT: u32 = 62;
    pub const MPROTECT: u32 = 74;
    pub const GETPGRP: u32 = 82;
    pub const SETPGID: u32 = 83;
    pub const DUP2: u32 = 90;
    pub const FCNTL: u32 = 92;
    pub const SELECT: u32 = 93;
    pub const GETTIMEOFDAY: u32 = 116;
    pub const WRITEV: u32 = 121;
    pub const MUNMAP: u32 = 73;
    pub const READLINK: u32 = 194;
    pub const MMAP: u32 = 197;
    pub const LSEEK: u32 = 199;
    pub const SYSCTL: u32 = 202;
}
