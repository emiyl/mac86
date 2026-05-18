use std::path;
use unicorn_engine::{RegisterX86, Unicorn};

use crate::filesystem::VirtualFileSystem;
use crate::threads::ThreadContinuation;

use super::fts::{allocate_fts_handle, close_fts_handle, with_fts_handle};
use super::math::{read_f32, read_f64, write_f64_st0};
use super::mem::{read_bytes, read_cstr, read_cstr_max, read_u32, write_u32};
use super::printf::{fmt_printf, fmt_printf_str, format_str};
use super::symbols::LibSym;
use super::trampoline::{
    ERRNO_STORAGE_ADDR, OPTARG_STORAGE_ADDR, OPTIND_STORAGE_ADDR, THREAD_SENTINEL_ADDR,
};

pub enum DispatchOutcome {
    Ret(u64),
    Exit,
    StateSet,
}

pub enum LibCallOutcome {
    Continue,
    Exit,
}

pub fn handle_libcall(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    sym: LibSym,
) -> LibCallOutcome {
    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0) as u32;
    let ret_addr = read_u32(emu, esp);
    let a0 = read_u32(emu, esp + 4);
    let a1 = read_u32(emu, esp + 8);
    let a2 = read_u32(emu, esp + 12);
    let a3 = read_u32(emu, esp + 16);

    log::debug!(
        "[libsystem] {:?}({:#x}, {:#x}, {:#x}, {:#x})",
        sym,
        a0,
        a1,
        a2,
        a3
    );

    let outcome = dispatch(emu, fs, sym, a0, a1, a2, a3, esp, ret_addr);

    match outcome {
        DispatchOutcome::Ret(v) => {
            let _ = emu.reg_write(RegisterX86::ESP, (esp + 4) as u64);
            let _ = emu.set_pc(ret_addr as u64);
            let _ = emu.reg_write(RegisterX86::EAX, v & 0xFFFF_FFFF);
            let _ = emu.reg_write(RegisterX86::EDX, v >> 32);
            LibCallOutcome::Continue
        }
        DispatchOutcome::Exit => LibCallOutcome::Exit,
        DispatchOutcome::StateSet => LibCallOutcome::Continue,
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    sym: LibSym,
    a0: u32,
    a1: u32,
    a2: u32,
    a3: u32,
    esp: u32,
    ret_addr: u32,
) -> DispatchOutcome {
    match sym {
        LibSym::Error => DispatchOutcome::Ret(ERRNO_STORAGE_ADDR as u64),
        // ── I/O ──────────────────────────────────────────────────────────────
        LibSym::Write => {
            let data = read_bytes(emu, a1, a2 as usize);
            DispatchOutcome::Ret(fs.write_bytes(a0, &data).unwrap_or(0) as u64)
        }
        LibSym::Writev => {
            let mut total: u64 = 0;
            for i in 0..a2 as usize {
                let base = read_u32(emu, a1 + (i as u32) * 8);
                let len = read_u32(emu, a1 + (i as u32) * 8 + 4) as usize;
                if len == 0 {
                    continue;
                }
                let data = read_bytes(emu, base, len);
                total += fs.write_bytes(a0, &data).unwrap_or(0) as u64;
            }
            DispatchOutcome::Ret(total)
        }
        LibSym::Read => {
            let data = fs.read_bytes(a0, a2 as usize).unwrap_or_default();
            if !data.is_empty() {
                let _ = emu.mem_write(a1 as u64, &data);
            }
            DispatchOutcome::Ret(data.len() as u64)
        }
        LibSym::Open => {
            let path = read_cstr(emu, a0);
            match fs.open(path::Path::new(&path), (a1 & 0x3) != 0) {
                Ok(fd) => DispatchOutcome::Ret(fd as u64),
                Err(_) => {
                    write_u32(emu, ERRNO_STORAGE_ADDR, 2); // ENOENT
                    DispatchOutcome::Ret(u32::MAX as u64)
                }
            }
        }
        LibSym::Close => {
            let _ = fs.close(a0);
            DispatchOutcome::Ret(0)
        }
        LibSym::Mkdir => {
            let path = read_cstr(emu, a0);
            match fs.mkdir(path::Path::new(&path)) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Unlink => {
            let path = read_cstr(emu, a0);
            match fs.unlink(path::Path::new(&path)) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Rmdir => {
            let path = read_cstr(emu, a0);
            match fs.rmdir(path::Path::new(&path)) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Fstat => {
            let fd = a0;
            let stat_ptr = a1;

            match fs.fstat_fd(fd) {
                Ok(fst) => {
                    let mut buf = vec![0u8; 120];
                    crate::filesystem::encode_stat_i386(&fst, &mut buf);
                    let _ = emu.mem_write(stat_ptr as u64, &buf);
                    DispatchOutcome::Ret(0)
                }
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Stat => {
            let path = read_cstr(emu, a0);
            let stat_ptr = a1;

            match fs.stat_path(path::Path::new(&path)) {
                Ok(fst) => {
                    let mut buf = vec![0u8; 120];
                    crate::filesystem::encode_stat_i386(&fst, &mut buf);
                    let _ = emu.mem_write(stat_ptr as u64, &buf);
                    DispatchOutcome::Ret(0)
                }
                Err(_) => {
                    write_u32(emu, ERRNO_STORAGE_ADDR, 2); // ENOENT
                    DispatchOutcome::Ret(u32::MAX as u64)
                }
            }
        }
        LibSym::Lstat => {
            let path = read_cstr(emu, a0);
            let stat_ptr = a1;

            match fs.lstat_path(path::Path::new(&path)) {
                Ok(fst) => {
                    let mut buf = vec![0u8; 120];
                    crate::filesystem::encode_stat_i386(&fst, &mut buf);
                    let _ = emu.mem_write(stat_ptr as u64, &buf);
                    DispatchOutcome::Ret(0)
                }
                Err(_) => {
                    write_u32(emu, ERRNO_STORAGE_ADDR, 2); // ENOENT
                    DispatchOutcome::Ret(u32::MAX as u64)
                }
            }
        }
        LibSym::Fcopyfile => {
            let src_fd = a0;
            let dst_fd = a1;
            match fs.fcopyfile(src_fd, dst_fd) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Copyfile => {
            let src = read_cstr(emu, a0);
            let dst = read_cstr(emu, a1);
            match fs.copyfile(path::Path::new(&src), path::Path::new(&dst)) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Rename => {
            let old_path = read_cstr(emu, a0);
            let new_path = read_cstr(emu, a1);
            match fs.rename(path::Path::new(&old_path), path::Path::new(&new_path)) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Chmod => {
            let path = read_cstr(emu, a0);
            let mode = a1 & 0xFFF;
            match fs.chmod(path::Path::new(&path), mode) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Fchmod => {
            let fd = a0;
            let mode = a1 & 0xFFF;
            match fs.fchmod(fd, mode) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Chown => {
            let path = read_cstr(emu, a0);
            let owner = a1;
            let group = a2;
            match fs.chown(path::Path::new(&path), owner, group) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Fchown => {
            let fd = a0;
            let owner = a1;
            let group = a2;
            match fs.fchown(fd, owner, group) {
                Ok(_) => DispatchOutcome::Ret(0),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::FtsOpen => {
            let argv_ptr = a0;
            let first_path_ptr = read_u32(emu, argv_ptr);
            let path = read_cstr(emu, first_path_ptr);

            let host_path = match fs.resolve_path(path::Path::new(&path)) {
                Ok(p) => p,
                Err(_) => return DispatchOutcome::Ret(0),
            };

            match allocate_fts_handle(host_path.to_str().unwrap_or("")) {
                Some(handle_id) => DispatchOutcome::Ret((0x50000100 + handle_id as u32) as u64),
                None => DispatchOutcome::Ret(0),
            }
        }
        LibSym::FtsRead => {
            let fts_opaque = a0 as u32;
            let handle_id = fts_opaque.saturating_sub(0x50000100);

            let entry = with_fts_handle(handle_id, |h| h.next_entry()).flatten();

            match entry {
                None => DispatchOutcome::Ret(0),
                Some((entry_path, fts_info, level, idx)) => {
                    let full_path = entry_path.to_string_lossy().to_string();

                    // Allocate FTSENT at a fixed location: 0x5fff0000 base
                    let ftsent_ptr = 0x5fff0000u32 + (handle_id as u32 * 4096) + (idx as u32 * 128);

                    let mut ftsent = vec![0u8; 66 + full_path.len() + 1];

                    // fts_cycle, fts_parent, fts_link, fts_number, fts_pointer
                    ftsent[0..20].fill(0);

                    // fts_accpath / fts_path — pointer to the inline filename string
                    let name_ptr = ftsent_ptr + 66;
                    ftsent[20..24].copy_from_slice(&name_ptr.to_le_bytes());
                    ftsent[24..28].copy_from_slice(&name_ptr.to_le_bytes());

                    // fts_errno, fts_symfd
                    ftsent[28..36].fill(0);

                    // fts_pathlen, fts_namelen
                    ftsent[36..38].copy_from_slice(&(full_path.len() as u16).to_le_bytes());
                    ftsent[38..40].copy_from_slice(&(full_path.len() as u16).to_le_bytes());

                    // fts_ino (8 bytes)
                    ftsent[40..48].copy_from_slice(&(idx as u64).to_le_bytes());

                    // fts_dev
                    ftsent[48..52].fill(0);

                    // fts_nlink
                    ftsent[52..54].copy_from_slice(&1u16.to_le_bytes());

                    // fts_level
                    ftsent[54..56].copy_from_slice(&(level as i16).to_le_bytes());

                    // fts_info
                    ftsent[56..58].copy_from_slice(&(fts_info as u16).to_le_bytes());

                    // fts_flags, fts_instr
                    ftsent[58..62].fill(0);

                    // fts_statp (NULL)
                    ftsent[62..66].fill(0);

                    // inline filename string
                    ftsent[66..66 + full_path.len()].copy_from_slice(full_path.as_bytes());
                    ftsent[66 + full_path.len()] = 0;

                    let _ = emu.mem_write(ftsent_ptr as u64, &ftsent);
                    DispatchOutcome::Ret(ftsent_ptr as u64)
                }
            }
        }
        LibSym::FtsClose => {
            let handle_id = (a0 as u32).saturating_sub(0x50000100);
            DispatchOutcome::Ret(if close_fts_handle(handle_id) {
                0
            } else {
                u32::MAX as u64
            })
        }
        LibSym::FtsSet => DispatchOutcome::Ret(0),
        LibSym::Getopt => {
            let argc = a0 as i32;
            let argv = a1;

            let mut optind = read_u32(emu, OPTIND_STORAGE_ADDR) as i32;
            if optind <= 0 {
                optind = 1;
            }
            write_u32(emu, OPTARG_STORAGE_ADDR, 0);

            if optind >= argc {
                DispatchOutcome::Ret(u32::MAX as u64)
            } else {
                let arg_ptr = read_u32(emu, argv.wrapping_add((optind as u32) * 4));
                let arg = read_cstr(emu, arg_ptr);

                if arg == "--" {
                    optind += 1;
                    write_u32(emu, OPTIND_STORAGE_ADDR, optind as u32);
                    DispatchOutcome::Ret(u32::MAX as u64)
                } else if !arg.starts_with('-') || arg == "-" {
                    DispatchOutcome::Ret(u32::MAX as u64)
                } else {
                    let opt = arg.as_bytes().get(1).copied().unwrap_or(b'?');
                    optind += 1;
                    write_u32(emu, OPTIND_STORAGE_ADDR, optind as u32);
                    DispatchOutcome::Ret(opt as u64)
                }
            }
        }
        LibSym::Exit | LibSym::Abort => DispatchOutcome::Exit,
        LibSym::Putchar => {
            let _ = fs.write_bytes(1, &[a0 as u8]);
            DispatchOutcome::Ret(a0 as u64 & 0xFF)
        }
        LibSym::Getchar => {
            let data = fs.read_bytes(0, 1).unwrap_or_default();
            DispatchOutcome::Ret(
                data.first()
                    .copied()
                    .map(|b| b as u64)
                    .unwrap_or(u32::MAX as u64),
            )
        }
        LibSym::Perror => {
            let msg = read_cstr(emu, a0);
            let _ = fs.write_bytes(2, format!("{}: error\n", msg).as_bytes());
            DispatchOutcome::Ret(0)
        }

        // ── Memory ───────────────────────────────────────────────────────────
        LibSym::Malloc => {
            let addr = fs
                .mmap_anon(if a0 == 0 { 4 } else { (a0 + 15) & !15 })
                .unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Free => DispatchOutcome::Ret(0),
        LibSym::Calloc => {
            let addr = fs
                .mmap_anon((a0.saturating_mul(a1).max(4) + 15) & !15)
                .unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Realloc => {
            let new = fs
                .mmap_anon(if a1 == 0 { 4 } else { (a1 + 15) & !15 })
                .unwrap_or(0);
            if a0 != 0 && new != 0 {
                let _ = emu.mem_write(new as u64, &read_bytes(emu, a0, a1 as usize));
            }
            DispatchOutcome::Ret(new as u64)
        }

        // ── stdio ─────────────────────────────────────────────────────────────
        LibSym::Puts => {
            let mut out = read_cstr(emu, a0).into_bytes();
            out.push(b'\n');
            let n = out.len();
            let _ = fs.write_bytes(1, &out);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Printf => DispatchOutcome::Ret(fmt_printf(emu, fs, 1, a0, esp + 8) as u64),
        LibSym::Fprintf => {
            let fd = if a0 <= 2 { a0 } else { 1 };
            DispatchOutcome::Ret(fmt_printf(emu, fs, fd, a1, esp + 12) as u64)
        }
        LibSym::Vprintf => DispatchOutcome::Ret(0),
        LibSym::Fputs => {
            let s = read_cstr(emu, a0);
            let fd = if a1 <= 2 { a1 } else { 1 };
            DispatchOutcome::Ret(fs.write_bytes(fd, s.as_bytes()).unwrap_or(0) as u64)
        }
        LibSym::Fflush => DispatchOutcome::Ret(0),
        LibSym::Sprintf => {
            let (text, _n) = format_str(emu, a1, esp + 12);
            let mut out = text.into_bytes();
            out.push(0);
            let n = out.len() - 1;
            let _ = emu.mem_write(a0 as u64, &out);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Snprintf => {
            let (text, _n) = format_str(emu, a2, esp + 16);
            let max = a1 as usize;
            let mut out = text.into_bytes();
            out.truncate(max.saturating_sub(1));
            out.push(0);
            let n = out.len() - 1;
            let _ = emu.mem_write(a0 as u64, &out);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Vsnprintf => DispatchOutcome::Ret(0),

        // ── string ───────────────────────────────────────────────────────────
        LibSym::Strlen => DispatchOutcome::Ret(read_cstr(emu, a0).len() as u64),
        LibSym::Strcmp => {
            let r = read_cstr(emu, a0)
                .as_bytes()
                .cmp(read_cstr(emu, a1).as_bytes()) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strncmp => {
            let n = a2 as usize;
            let s1 = read_cstr_max(emu, a0, n);
            let s2 = read_cstr_max(emu, a1, n);
            let r = s1.as_bytes()[..s1.len().min(n)].cmp(&s2.as_bytes()[..s2.len().min(n)]) as i8
                as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strcasecmp => {
            let s1 = read_cstr(emu, a0).to_ascii_lowercase();
            let s2 = read_cstr(emu, a1).to_ascii_lowercase();
            let r = s1.as_bytes().cmp(s2.as_bytes()) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strncasecmp => {
            let n = a2 as usize;
            let s1 = read_cstr_max(emu, a0, n).to_ascii_lowercase();
            let s2 = read_cstr_max(emu, a1, n).to_ascii_lowercase();
            let r = s1.as_bytes()[..s1.len().min(n)].cmp(&s2.as_bytes()[..s2.len().min(n)]) as i8
                as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strcpy => {
            let mut b = read_cstr(emu, a1).into_bytes();
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strncpy => {
            let n = a2 as usize;
            let mut b = read_cstr_max(emu, a1, n).into_bytes();
            b.truncate(n);
            while b.len() < n {
                b.push(0);
            }
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strcat => {
            let mut b = read_cstr(emu, a0).into_bytes();
            b.extend_from_slice(read_cstr(emu, a1).as_bytes());
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strncat => {
            let n = a2 as usize;
            let mut b = read_cstr(emu, a0).into_bytes();
            let src = read_cstr_max(emu, a1, n);
            b.extend_from_slice(&src.as_bytes()[..src.len().min(n)]);
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strlcpy => {
            let src = read_cstr(emu, a1);
            let dstsize = a2 as usize;
            if dstsize > 0 {
                let copy_len = src.len().min(dstsize - 1);
                let mut b = src.as_bytes()[..copy_len].to_vec();
                b.push(0);
                let _ = emu.mem_write(a0 as u64, &b);
            }
            DispatchOutcome::Ret(src.len() as u64)
        }
        LibSym::Strlcat => {
            let dst = read_cstr(emu, a0);
            let src = read_cstr(emu, a1);
            let dstsize = a2 as usize;
            let dst_len = dst.len();
            if dstsize > dst_len + 1 {
                let space = dstsize - dst_len - 1;
                let copy_len = src.len().min(space);
                let mut b = src.as_bytes()[..copy_len].to_vec();
                b.push(0);
                let _ = emu.mem_write((a0 + dst_len as u32) as u64, &b);
            }
            DispatchOutcome::Ret((dst_len + src.len()) as u64)
        }
        LibSym::Strchr => {
            let s = read_cstr(emu, a0);
            let c = (a1 & 0xFF) as u8;
            match s.as_bytes().iter().position(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strrchr => {
            let s = read_cstr(emu, a0);
            let c = (a1 & 0xFF) as u8;
            match s.as_bytes().iter().rposition(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strstr => {
            let haystack = read_cstr(emu, a0);
            let needle = read_cstr(emu, a1);
            if needle.is_empty() {
                return DispatchOutcome::Ret(a0 as u64);
            }
            match haystack.find(&needle as &str) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strdup => {
            let mut b = read_cstr(emu, a0).into_bytes();
            b.push(0);
            let addr = fs.mmap_anon((b.len() as u32 + 15) & !15).unwrap_or(0);
            if addr != 0 {
                let _ = emu.mem_write(addr as u64, &b);
            }
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Strtok => {
            if a0 == 0 {
                return DispatchOutcome::Ret(0);
            }
            let s = read_cstr(emu, a0);
            let delims = read_cstr(emu, a1);
            match s.find(|c: char| delims.contains(c)) {
                Some(i) => {
                    let _ = emu.mem_write(a0 as u64 + i as u64, &[0u8]);
                    DispatchOutcome::Ret(a0 as u64)
                }
                None => DispatchOutcome::Ret(a0 as u64),
            }
        }
        LibSym::Strsep => {
            if a0 == 0 {
                return DispatchOutcome::Ret(0);
            }
            let str_ptr_ptr = a0;
            let str_ptr = read_u32(emu, str_ptr_ptr);
            if str_ptr == 0 {
                return DispatchOutcome::Ret(0);
            }
            let s = read_cstr(emu, str_ptr);
            let delims = read_cstr(emu, a1);
            let token_start = str_ptr;
            match s.find(|c: char| delims.contains(c)) {
                Some(i) => {
                    let null_pos = str_ptr + i as u32;
                    let _ = emu.mem_write(null_pos as u64, &[0u8]);
                    let next = null_pos + 1;
                    let _ = emu.mem_write(str_ptr_ptr as u64, &next.to_le_bytes());
                }
                None => {
                    let _ = emu.mem_write(str_ptr_ptr as u64, &0u32.to_le_bytes());
                }
            }
            DispatchOutcome::Ret(token_start as u64)
        }

        // ── memory ops ───────────────────────────────────────────────────────
        LibSym::Memcpy | LibSym::Memmove => {
            let data = read_bytes(emu, a1, a2 as usize);
            let _ = emu.mem_write(a0 as u64, &data);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Memset => {
            let _ = emu.mem_write(a0 as u64, &vec![(a1 & 0xFF) as u8; a2 as usize]);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Memcmp => {
            let r = read_bytes(emu, a0, a2 as usize).cmp(&read_bytes(emu, a1, a2 as usize)) as i8
                as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Memchr => {
            let buf = read_bytes(emu, a0, a2 as usize);
            let c = (a1 & 0xFF) as u8;
            match buf.iter().position(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None => DispatchOutcome::Ret(0),
            }
        }

        // ── conversions ───────────────────────────────────────────────────────
        LibSym::Atoi => {
            DispatchOutcome::Ret(read_cstr(emu, a0).trim().parse::<i32>().unwrap_or(0) as u32 as u64)
        }
        LibSym::Atol => {
            DispatchOutcome::Ret(read_cstr(emu, a0).trim().parse::<i32>().unwrap_or(0) as u32 as u64)
        }
        LibSym::Atoll => {
            let v = read_cstr(emu, a0).trim().parse::<i64>().unwrap_or(0);
            DispatchOutcome::Ret(v as u64)
        }
        LibSym::Strtol | LibSym::Strtoul | LibSym::Strtoll | LibSym::Strtoull => {
            let s = read_cstr(emu, a0);
            let base = if a2 == 0 { 10 } else { a2 };
            let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
            let v = u64::from_str_radix(s, base).unwrap_or(0);
            if a1 != 0 {
                let consumed = a0 + read_cstr(emu, a0).len() as u32;
                let _ = emu.mem_write(a1 as u64, &consumed.to_le_bytes());
            }
            DispatchOutcome::Ret(v)
        }
        LibSym::Strtod | LibSym::Atof => {
            let s = read_cstr(emu, a0);
            let v = s.trim().parse::<f64>().unwrap_or(0.0);
            write_f64_st0(emu, v);
            DispatchOutcome::Ret(0)
        }

        // ── char classification ───────────────────────────────────────────────
        LibSym::Isdigit => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_digit() as u64),
        LibSym::Isalpha => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_alphabetic() as u64),
        LibSym::Isalnum => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_alphanumeric() as u64),
        LibSym::Isspace => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_whitespace() as u64),
        LibSym::Isupper => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_uppercase() as u64),
        LibSym::Islower => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_lowercase() as u64),
        LibSym::Ispunct => DispatchOutcome::Ret(((a0 & 0xFF) as u8).is_ascii_punctuation() as u64),
        LibSym::Toupper => DispatchOutcome::Ret(((a0 & 0xFF) as u8).to_ascii_uppercase() as u64),
        LibSym::Tolower => DispatchOutcome::Ret(((a0 & 0xFF) as u8).to_ascii_lowercase() as u64),

        // ── sorting / search ─────────────────────────────────────────────────
        LibSym::Qsort => DispatchOutcome::Ret(0),
        LibSym::Bsearch => DispatchOutcome::Ret(0),
        LibSym::Abs => DispatchOutcome::Ret((a0 as i32).unsigned_abs() as u64),
        LibSym::Labs => DispatchOutcome::Ret((a0 as i32).unsigned_abs() as u64),

        // ── env ───────────────────────────────────────────────────────────────
        LibSym::Getenv | LibSym::Setenv | LibSym::Unsetenv => DispatchOutcome::Ret(0),

        // ── dynamic linking ───────────────────────────────────────────────────
        LibSym::Dlopen => {
            let path = if a0 == 0 {
                String::new()
            } else {
                read_cstr(emu, a0)
            };
            log::debug!("dlopen({:?}, 0x{:x})", path, a1);
            let handle: u32 = if path.is_empty()
                || path.contains("libSystem")
                || path.contains("libm")
                || path.contains("libpthread")
                || path.contains("libc")
                || path.contains("libc++")
                || path.contains("CoreFoundation")
                || path.contains("Foundation")
                || path.contains("libdyld")
            {
                0x1000_0001
            } else {
                0
            };
            DispatchOutcome::Ret(handle as u64)
        }
        LibSym::Dlsym => {
            let sym_name = read_cstr(emu, a1);
            log::debug!("dlsym({:#x}, {:?})", a0, sym_name);
            let clean = sym_name.trim_start_matches('_');
            let addr = fs.trampoline_map.get(clean).copied().unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Dlclose | LibSym::Dlerror => DispatchOutcome::Ret(0),

        // ── pthread ───────────────────────────────────────────────────────────
        LibSym::PthreadCreate => {
            let tid_out = a0;
            let start_fn = a2;
            let arg = a3;
            let tid = fs.threads.alloc_tid();
            let _ = emu.mem_write(tid_out as u64, &tid.to_le_bytes());

            let stack_size: u32 = 0x1_0000;
            let stack_base = fs.mmap_anon(stack_size).unwrap_or(0);
            let mut tsp = (stack_base + stack_size) & !0xF;
            tsp -= 4;
            let _ = emu.mem_write(tsp as u64, &arg.to_le_bytes());
            tsp -= 4;
            let _ = emu.mem_write(tsp as u64, &THREAD_SENTINEL_ADDR.to_le_bytes());

            fs.threads.continuations.push(ThreadContinuation {
                ret_addr,
                tid,
                ebx: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
                ecx: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
                edx: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
                esi: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
                edi: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
                ebp: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
                esp,
            });
            let _ = emu.reg_write(RegisterX86::ESP, tsp as u64);
            let _ = emu.reg_write(RegisterX86::EBP, 0u64);
            for r in [
                RegisterX86::EAX,
                RegisterX86::EBX,
                RegisterX86::ECX,
                RegisterX86::EDX,
                RegisterX86::ESI,
                RegisterX86::EDI,
            ] {
                let _ = emu.reg_write(r, 0u64);
            }
            let _ = emu.set_pc(start_fn as u64);
            DispatchOutcome::StateSet
        }
        LibSym::ThreadSentinel => {
            let retval = emu.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;
            if let Some(cont) = fs.threads.continuations.pop() {
                fs.threads.store_result(cont.tid, retval);
                let _ = emu.reg_write(RegisterX86::EBX, cont.ebx as u64);
                let _ = emu.reg_write(RegisterX86::ECX, cont.ecx as u64);
                let _ = emu.reg_write(RegisterX86::EDX, cont.edx as u64);
                let _ = emu.reg_write(RegisterX86::ESI, cont.esi as u64);
                let _ = emu.reg_write(RegisterX86::EDI, cont.edi as u64);
                let _ = emu.reg_write(RegisterX86::EBP, cont.ebp as u64);
                let _ = emu.reg_write(RegisterX86::ESP, (cont.esp + 4) as u64);
                let _ = emu.reg_write(RegisterX86::EAX, 0u64);
                let _ = emu.set_pc(cont.ret_addr as u64);
                DispatchOutcome::StateSet
            } else {
                DispatchOutcome::Exit
            }
        }
        LibSym::SignalReturn => {
            if let Some(cont) = fs.threads.continuations.pop() {
                let _ = emu.reg_write(RegisterX86::EBX, cont.ebx as u64);
                let _ = emu.reg_write(RegisterX86::ECX, cont.ecx as u64);
                let _ = emu.reg_write(RegisterX86::EDX, cont.edx as u64);
                let _ = emu.reg_write(RegisterX86::ESI, cont.esi as u64);
                let _ = emu.reg_write(RegisterX86::EDI, cont.edi as u64);
                let _ = emu.reg_write(RegisterX86::EBP, cont.ebp as u64);
                let _ = emu.reg_write(RegisterX86::ESP, cont.esp as u64);
                let _ = emu.set_pc(cont.ret_addr as u64);
                DispatchOutcome::StateSet
            } else {
                DispatchOutcome::Exit
            }
        }
        LibSym::PthreadJoin => {
            if let Some(result) = fs.threads.get_result(a0) {
                if a1 != 0 {
                    let _ = emu.mem_write(a1 as u64, &(result as u64).to_le_bytes());
                }
            }
            DispatchOutcome::Ret(0)
        }
        LibSym::PthreadSelf => DispatchOutcome::Ret(1),
        LibSym::PthreadCancel | LibSym::PthreadTestcancel => DispatchOutcome::Ret(0),
        LibSym::PthreadMutexInit
        | LibSym::PthreadMutexLock
        | LibSym::PthreadMutexUnlock
        | LibSym::PthreadMutexTrylock
        | LibSym::PthreadMutexDestroy
        | LibSym::PthreadRwlockInit
        | LibSym::PthreadRwlockRdlock
        | LibSym::PthreadRwlockWrlock
        | LibSym::PthreadRwlockUnlock
        | LibSym::PthreadRwlockDestroy
        | LibSym::PthreadCondInit
        | LibSym::PthreadCondWait
        | LibSym::PthreadCondTimedwait
        | LibSym::PthreadCondSignal
        | LibSym::PthreadCondBroadcast
        | LibSym::PthreadCondDestroy
        | LibSym::PthreadAttrInit
        | LibSym::PthreadAttrSetdetachstate
        | LibSym::PthreadAttrSetstacksize
        | LibSym::PthreadAttrDestroy => DispatchOutcome::Ret(0),
        LibSym::PthreadOnce => {
            if fs.threads.once_check_and_set(a0) && a1 != 0 {
                let mut tsp = esp;
                tsp -= 4;
                let _ = emu.mem_write(tsp as u64, &THREAD_SENTINEL_ADDR.to_le_bytes());
                fs.threads.continuations.push(ThreadContinuation {
                    ret_addr,
                    tid: 1,
                    ebx: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
                    ecx: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
                    edx: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
                    esi: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
                    edi: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
                    ebp: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
                    esp,
                });
                let _ = emu.reg_write(RegisterX86::ESP, tsp as u64);
                let _ = emu.set_pc(a1 as u64);
                DispatchOutcome::StateSet
            } else {
                DispatchOutcome::Ret(0)
            }
        }
        LibSym::PthreadKeyCreate => {
            let key = fs.threads.create_key();
            if a0 != 0 {
                let _ = emu.mem_write(a0 as u64, &key.to_le_bytes());
            }
            DispatchOutcome::Ret(0)
        }
        LibSym::PthreadKeyDelete => DispatchOutcome::Ret(0),
        LibSym::PthreadGetspecific => DispatchOutcome::Ret(fs.threads.get_tls(a0) as u64),
        LibSym::PthreadSetspecific => {
            fs.threads.set_tls(a0, a1);
            DispatchOutcome::Ret(0)
        }

        // ── setjmp / longjmp ─────────────────────────────────────────────────
        LibSym::Setjmp => {
            let jbuf = a0;
            let mut buf = [0u8; 32];
            let w = |b: &mut [u8; 32], off: usize, v: u32| {
                b[off..off + 4].copy_from_slice(&v.to_le_bytes())
            };
            w(
                &mut buf,
                0,
                emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
            );
            w(
                &mut buf,
                4,
                emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
            );
            w(
                &mut buf,
                8,
                emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
            );
            w(
                &mut buf,
                12,
                emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
            );
            w(&mut buf, 16, esp + 8);
            w(&mut buf, 20, ret_addr);
            let _ = emu.mem_write(jbuf as u64, &buf);
            DispatchOutcome::Ret(0)
        }
        LibSym::Longjmp => {
            let mut buf = [0u8; 32];
            let _ = emu.mem_read(a0 as u64, &mut buf);
            let r = |b: &[u8; 32], off: usize| {
                u32::from_le_bytes(b[off..off + 4].try_into().unwrap_or_default())
            };
            let _ = emu.reg_write(RegisterX86::EBX, r(&buf, 0) as u64);
            let _ = emu.reg_write(RegisterX86::ESI, r(&buf, 4) as u64);
            let _ = emu.reg_write(RegisterX86::EDI, r(&buf, 8) as u64);
            let _ = emu.reg_write(RegisterX86::EBP, r(&buf, 12) as u64);
            let _ = emu.reg_write(RegisterX86::ESP, r(&buf, 16) as u64);
            let _ = emu.reg_write(RegisterX86::EAX, if a1 == 0 { 1 } else { a1 } as u64);
            let _ = emu.set_pc(r(&buf, 20) as u64);
            DispatchOutcome::StateSet
        }

        // ── math (double) ─────────────────────────────────────────────────────
        LibSym::Sin => {
            write_f64_st0(emu, read_f64(emu, esp + 4).sin());
            DispatchOutcome::Ret(0)
        }
        LibSym::Cos => {
            write_f64_st0(emu, read_f64(emu, esp + 4).cos());
            DispatchOutcome::Ret(0)
        }
        LibSym::Tan => {
            write_f64_st0(emu, read_f64(emu, esp + 4).tan());
            DispatchOutcome::Ret(0)
        }
        LibSym::Sqrt => {
            write_f64_st0(emu, read_f64(emu, esp + 4).sqrt());
            DispatchOutcome::Ret(0)
        }
        LibSym::Pow => {
            write_f64_st0(emu, read_f64(emu, esp + 4).powf(read_f64(emu, esp + 12)));
            DispatchOutcome::Ret(0)
        }
        LibSym::Log => {
            write_f64_st0(emu, read_f64(emu, esp + 4).ln());
            DispatchOutcome::Ret(0)
        }
        LibSym::Log2 => {
            write_f64_st0(emu, read_f64(emu, esp + 4).log2());
            DispatchOutcome::Ret(0)
        }
        LibSym::Log10 => {
            write_f64_st0(emu, read_f64(emu, esp + 4).log10());
            DispatchOutcome::Ret(0)
        }
        LibSym::Exp => {
            write_f64_st0(emu, read_f64(emu, esp + 4).exp());
            DispatchOutcome::Ret(0)
        }
        LibSym::Exp2 => {
            write_f64_st0(emu, read_f64(emu, esp + 4).exp2());
            DispatchOutcome::Ret(0)
        }
        LibSym::Floor => {
            write_f64_st0(emu, read_f64(emu, esp + 4).floor());
            DispatchOutcome::Ret(0)
        }
        LibSym::Ceil => {
            write_f64_st0(emu, read_f64(emu, esp + 4).ceil());
            DispatchOutcome::Ret(0)
        }
        LibSym::Round => {
            write_f64_st0(emu, read_f64(emu, esp + 4).round());
            DispatchOutcome::Ret(0)
        }
        LibSym::Fabs => {
            write_f64_st0(emu, read_f64(emu, esp + 4).abs());
            DispatchOutcome::Ret(0)
        }
        LibSym::Fmod => {
            write_f64_st0(emu, read_f64(emu, esp + 4) % read_f64(emu, esp + 12));
            DispatchOutcome::Ret(0)
        }
        LibSym::Atan => {
            write_f64_st0(emu, read_f64(emu, esp + 4).atan());
            DispatchOutcome::Ret(0)
        }
        LibSym::Atan2 => {
            write_f64_st0(emu, read_f64(emu, esp + 4).atan2(read_f64(emu, esp + 12)));
            DispatchOutcome::Ret(0)
        }
        LibSym::Asin => {
            write_f64_st0(emu, read_f64(emu, esp + 4).asin());
            DispatchOutcome::Ret(0)
        }
        LibSym::Acos => {
            write_f64_st0(emu, read_f64(emu, esp + 4).acos());
            DispatchOutcome::Ret(0)
        }
        LibSym::Sinh => {
            write_f64_st0(emu, read_f64(emu, esp + 4).sinh());
            DispatchOutcome::Ret(0)
        }
        LibSym::Cosh => {
            write_f64_st0(emu, read_f64(emu, esp + 4).cosh());
            DispatchOutcome::Ret(0)
        }
        LibSym::Tanh => {
            write_f64_st0(emu, read_f64(emu, esp + 4).tanh());
            DispatchOutcome::Ret(0)
        }

        // ── math (float) ──────────────────────────────────────────────────────
        LibSym::Sinf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).sin() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Cosf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).cos() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Tanf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).tan() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Sqrtf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).sqrt() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Powf => {
            write_f64_st0(
                emu,
                read_f32(emu, esp + 4).powf(read_f32(emu, esp + 8)) as f64,
            );
            DispatchOutcome::Ret(0)
        }
        LibSym::Logf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).ln() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Expf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).exp() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Fabsf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).abs() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Floorf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).floor() as f64);
            DispatchOutcome::Ret(0)
        }
        LibSym::Ceilf => {
            write_f64_st0(emu, read_f32(emu, esp + 4).ceil() as f64);
            DispatchOutcome::Ret(0)
        }

        // ── ObjC runtime stubs ────────────────────────────────────────────────
        LibSym::ObjcMsgSend | LibSym::ObjcMsgSendStret => DispatchOutcome::Ret(0),
        LibSym::ObjcGetClass | LibSym::ObjcLookUpClass => DispatchOutcome::Ret(0),
        LibSym::NSLog => {
            let cstr_ptr = read_u32(emu, a0 + 8);
            let fmt_str = if cstr_ptr != 0 {
                read_cstr(emu, cstr_ptr)
            } else {
                read_cstr(emu, a0)
            };
            let _ = fmt_printf_str(emu, fs, 2, &fmt_str, esp + 8);
            let _ = fs.write_bytes(2, b"\n");
            DispatchOutcome::Ret(0)
        }

        LibSym::Stub0 => DispatchOutcome::Ret(0),
    }
}
