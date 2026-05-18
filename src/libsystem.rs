/// libSystem trampoline — load-time dynamic symbol resolution.
///
/// At load time we fill every `__nl_symbol_ptr` / `__la_symbol_ptr` slot with
/// a unique address in the "trampoline region" (0x5000_0000+).  A Unicorn code
/// hook watches that region; when execution lands there we read the cdecl
/// arguments off the guest stack, call our Rust handler, write the return
/// value into EAX/EDX, and simulate `RET` by advancing ESP and setting PC
/// back to the caller.
///
/// Slot 0 (0x5000_0000) is always reserved for an implicit `_exit` so that
/// `main()`'s return address is always a valid, clean-shutdown target even when
/// the binary doesn't import `_exit` directly.
use crate::dyld::DyldBindings;
use crate::filesystem::VirtualFileSystem;
use std::collections::HashMap;
use unicorn_engine::{RegisterX86, Unicorn};

pub const TRAMPOLINE_BASE: u32 = 0x5000_0000;

// ── symbol table ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibSym {
    // I/O
    Write,
    Read,
    Open,
    Close,
    // Process
    Exit,
    Abort,
    // Memory
    Malloc,
    Free,
    Calloc,
    Realloc,
    // stdio
    Puts,
    Printf,
    Fprintf,
    Vprintf,
    Fputs,
    Fflush,
    // string / memory
    Strlen,
    Strcmp,
    Strncmp,
    Strcpy,
    Strncpy,
    Strcat,
    Strchr,
    Strdup,
    Memcpy,
    Memmove,
    Memset,
    Memcmp,
    // env
    Getenv,
    // silent no-op stubs
    Stub0,
}

/// Strip `$VARIANT$PLATFORM` version suffixes and the leading `_`.
fn base_name(raw: &str) -> &str {
    let s = raw.trim_start_matches('_');
    if let Some(i) = s.find('$') {
        &s[..i]
    } else {
        s
    }
}

pub fn known_symbol(name: &str) -> Option<LibSym> {
    match base_name(name) {
        "write" | "write_nocancel" | "pwrite" => Some(LibSym::Write),
        "read" | "read_nocancel" | "pread" => Some(LibSym::Read),
        "open" | "open_nocancel" => Some(LibSym::Open),
        "close" | "close_nocancel" => Some(LibSym::Close),
        "exit" | "_exit" | "quick_exit" => Some(LibSym::Exit),
        "abort" => Some(LibSym::Abort),
        "malloc" | "malloc_zone_malloc" => Some(LibSym::Malloc),
        "free" | "malloc_zone_free" => Some(LibSym::Free),
        "calloc" | "malloc_zone_calloc" => Some(LibSym::Calloc),
        "realloc" | "malloc_zone_realloc" => Some(LibSym::Realloc),
        "puts" => Some(LibSym::Puts),
        "printf" | "__printf_chk" | "printf_chk" => Some(LibSym::Printf),
        "fprintf" | "__fprintf_chk" | "fprintf_chk" => Some(LibSym::Fprintf),
        "vprintf" | "__vprintf_chk" => Some(LibSym::Vprintf),
        "fputs" => Some(LibSym::Fputs),
        "fflush" => Some(LibSym::Fflush),
        "strlen" => Some(LibSym::Strlen),
        "strcmp" => Some(LibSym::Strcmp),
        "strncmp" => Some(LibSym::Strncmp),
        "strcpy" => Some(LibSym::Strcpy),
        "strncpy" => Some(LibSym::Strncpy),
        "strcat" => Some(LibSym::Strcat),
        "strchr" => Some(LibSym::Strchr),
        "strdup" => Some(LibSym::Strdup),
        "memcpy" | "__memcpy_chk" => Some(LibSym::Memcpy),
        "memmove" | "__memmove_chk" => Some(LibSym::Memmove),
        "memset" | "__memset_chk" => Some(LibSym::Memset),
        "memcmp" => Some(LibSym::Memcmp),
        "getenv" => Some(LibSym::Getenv),
        // No-op stubs for common runtime setup calls
        "atexit"
        | "__cxa_atexit"
        | "__cxa_finalize"
        | "__cxa_thread_atexit"
        | "setlocale"
        | "bindtextdomain"
        | "textdomain"
        | "tzset"
        | "__pthread_sigmask"
        | "pthread_atfork"
        | "mach_init_routine"
        | "__dyld_func_lookup"
        | "dyld_stub_binding_helper"
        | "__keymgr_dwarf2_register_sections" => Some(LibSym::Stub0),
        _ => None,
    }
}

// ── trampoline table ──────────────────────────────────────────────────────────

pub struct Trampoline {
    /// trampoline address → handler
    pub dispatch: HashMap<u32, LibSym>,
    /// import name → trampoline address (for filling pointer slots)
    name_to_addr: HashMap<String, u32>,
    /// LibSym → canonical trampoline address (for dedup / exit_addr)
    #[allow(dead_code)]
    sym_to_addr: HashMap<LibSym, u32>,
    pub slot_count: u32,
}

impl Trampoline {
    pub fn build(bindings: &DyldBindings) -> Self {
        let mut dispatch: HashMap<u32, LibSym> = HashMap::new();
        let mut name_to_addr: HashMap<String, u32> = HashMap::new();
        let mut sym_to_addr: HashMap<LibSym, u32> = HashMap::new();
        let mut slot = 0u32;

        // Slot 0 is always Exit — guarantees main()'s fake return addr is valid.
        let exit_slot = TRAMPOLINE_BASE;
        dispatch.insert(exit_slot, LibSym::Exit);
        sym_to_addr.insert(LibSym::Exit, exit_slot);
        slot += 1;

        for imp in &bindings.imports {
            let Some(sym) = known_symbol(&imp.name) else {
                continue;
            };
            let addr = *sym_to_addr.entry(sym).or_insert_with(|| {
                let a = TRAMPOLINE_BASE + slot * 4;
                slot += 1;
                dispatch.insert(a, sym);
                a
            });
            name_to_addr.insert(imp.name.clone(), addr);
        }

        Trampoline {
            dispatch,
            name_to_addr,
            sym_to_addr,
            slot_count: slot,
        }
    }

    /// Address to use as main()'s fake return address — always Exit slot 0.
    pub fn exit_addr(&self) -> u32 {
        TRAMPOLINE_BASE
    }

    /// Trampoline address for a specific import name.
    pub fn addr_for_binding(&self, name: &str) -> Option<u32> {
        self.name_to_addr.get(name).copied()
    }

    /// One past the last allocated slot — used as the hook end address.
    pub fn region_end(&self) -> u32 {
        TRAMPOLINE_BASE + (self.slot_count + 1) * 4
    }
}

// ── call handler ─────────────────────────────────────────────────────────────

pub enum LibCallOutcome {
    Continue,
    Exit,
}

/// Dispatch a libSystem call. Reads cdecl args from the guest stack, executes
/// the handler, writes the result to EAX/EDX, and simulates `RET`.
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

    log::debug!("[libsystem] {:?}({:#x}, {:#x}, {:#x}, {:#x})", sym, a0, a1, a2, a3);

    let (retval, stop) = dispatch(emu, fs, sym, a0, a1, a2, a3, esp);

    // Simulate cdecl RET: pop return address, jump back.
    let _ = emu.reg_write(RegisterX86::ESP, (esp + 4) as u64);
    let _ = emu.set_pc(ret_addr as u64);
    let _ = emu.reg_write(RegisterX86::EAX, retval & 0xFFFF_FFFF);
    let _ = emu.reg_write(RegisterX86::EDX, retval >> 32);

    if stop {
        LibCallOutcome::Exit
    } else {
        LibCallOutcome::Continue
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
    _a3: u32,
    esp: u32,
) -> (u64, bool) {
    match sym {
        LibSym::Write => {
            let data = read_bytes(emu, a1, a2 as usize);
            let n = fs.write_bytes(a0, &data).unwrap_or(0);
            (n as u64, false)
        }
        LibSym::Read => {
            let data = fs.read_bytes(a0, a2 as usize).unwrap_or_default();
            if !data.is_empty() {
                let _ = emu.mem_write(a1 as u64, &data);
            }
            (data.len() as u64, false)
        }
        LibSym::Open => {
            let path = read_cstr(emu, a0);
            let writable = (a1 & 0x3) != 0;
            match fs.open(std::path::Path::new(&path), writable) {
                Ok(fd) => (fd as u64, false),
                Err(_) => (u32::MAX as u64, false),
            }
        }
        LibSym::Close => {
            let _ = fs.close(a0);
            (0, false)
        }
        LibSym::Exit | LibSym::Abort => (0, true),

        // ── Memory ───────────────────────────────────────────────────────
        LibSym::Malloc => {
            let size = if a0 == 0 { 4 } else { (a0 + 15) & !15 };
            let addr = fs.mmap_anon(size).unwrap_or(0);
            (addr as u64, false)
        }
        LibSym::Free => (0, false), // bump allocator — no-op
        LibSym::Calloc => {
            // calloc(count, size) — memory is already zeroed by Unicorn
            let total = a0.saturating_mul(a1).max(4);
            let size = (total + 15) & !15;
            let addr = fs.mmap_anon(size).unwrap_or(0);
            (addr as u64, false)
        }
        LibSym::Realloc => {
            let size = if a1 == 0 { 4 } else { (a1 + 15) & !15 };
            let new_addr = fs.mmap_anon(size).unwrap_or(0);
            if a0 != 0 && new_addr != 0 {
                let old = read_bytes(emu, a0, a1 as usize);
                let _ = emu.mem_write(new_addr as u64, &old);
            }
            (new_addr as u64, false)
        }

        // ── stdio ────────────────────────────────────────────────────────
        LibSym::Puts => {
            let s = read_cstr(emu, a0);
            let mut out = s.into_bytes();
            out.push(b'\n');
            let n = out.len();
            let _ = fs.write_bytes(1, &out);
            (n as u64, false)
        }
        LibSym::Printf => {
            // printf(fmt, varargs…) — varargs start at esp+8
            let n = fmt_printf(emu, fs, 1, a0, esp + 8);
            (n as u64, false)
        }
        LibSym::Fprintf => {
            // fprintf(FILE*, fmt, varargs…)
            // FILE* values 0/1/2 map directly to fds; anything else → stdout.
            let fd = if a0 <= 2 { a0 } else { 1 };
            let n = fmt_printf(emu, fs, fd, a1, esp + 12);
            (n as u64, false)
        }
        LibSym::Vprintf => (0, false), // va_list expansion not supported
        LibSym::Fputs => {
            let s = read_cstr(emu, a0);
            let fd = if a1 <= 2 { a1 } else { 1 };
            let n = fs.write_bytes(fd, s.as_bytes()).unwrap_or(0);
            (n as u64, false)
        }
        LibSym::Fflush => (0, false),

        // ── string / memory ──────────────────────────────────────────────
        LibSym::Strlen => {
            let s = read_cstr(emu, a0);
            (s.len() as u64, false)
        }
        LibSym::Strcmp => {
            let s1 = read_cstr(emu, a0);
            let s2 = read_cstr(emu, a1);
            let r = s1.as_bytes().cmp(s2.as_bytes()) as i8 as i32;
            (r as u32 as u64, false)
        }
        LibSym::Strncmp => {
            let n = a2 as usize;
            let s1 = read_cstr_max(emu, a0, n);
            let s2 = read_cstr_max(emu, a1, n);
            let cmp = s1.as_bytes()[..s1.len().min(n)]
                .cmp(&s2.as_bytes()[..s2.len().min(n)]);
            let r = cmp as i8 as i32;
            (r as u32 as u64, false)
        }
        LibSym::Strcpy => {
            let s = read_cstr(emu, a1);
            let mut b = s.into_bytes();
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            (a0 as u64, false)
        }
        LibSym::Strncpy => {
            let n = a2 as usize;
            let s = read_cstr_max(emu, a1, n);
            let mut b = s.into_bytes();
            b.truncate(n);
            while b.len() < n {
                b.push(0);
            }
            let _ = emu.mem_write(a0 as u64, &b);
            (a0 as u64, false)
        }
        LibSym::Strcat => {
            let dest = read_cstr(emu, a0);
            let src = read_cstr(emu, a1);
            let mut b = dest.into_bytes();
            b.extend_from_slice(src.as_bytes());
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            (a0 as u64, false)
        }
        LibSym::Strchr => {
            let s = read_cstr(emu, a0);
            let c = (a1 & 0xFF) as u8;
            match s.as_bytes().iter().position(|&b| b == c) {
                Some(i) => (a0 as u64 + i as u64, false),
                None => (0, false),
            }
        }
        LibSym::Strdup => {
            let s = read_cstr(emu, a0);
            let mut b = s.into_bytes();
            b.push(0);
            let len = b.len() as u32;
            let addr = fs.mmap_anon((len + 15) & !15).unwrap_or(0);
            if addr != 0 {
                let _ = emu.mem_write(addr as u64, &b);
            }
            (addr as u64, false)
        }
        LibSym::Memcpy | LibSym::Memmove => {
            let data = read_bytes(emu, a1, a2 as usize);
            let _ = emu.mem_write(a0 as u64, &data);
            (a0 as u64, false)
        }
        LibSym::Memset => {
            let val = (a1 & 0xFF) as u8;
            let buf = vec![val; a2 as usize];
            let _ = emu.mem_write(a0 as u64, &buf);
            (a0 as u64, false)
        }
        LibSym::Memcmp => {
            let b1 = read_bytes(emu, a0, a2 as usize);
            let b2 = read_bytes(emu, a1, a2 as usize);
            let r = b1.cmp(&b2) as i8 as i32;
            (r as u32 as u64, false)
        }

        LibSym::Getenv => (0, false), // NULL — no env
        LibSym::Stub0 => (0, false),
    }
}

// ── printf ────────────────────────────────────────────────────────────────────

/// Very small printf that handles %d %i %u %x %X %o %p %s %c %%.
/// `vararg_esp` is the stack address of the first format argument.
fn fmt_printf(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    fd: u32,
    fmt_ptr: u32,
    mut vararg_esp: u32,
) -> usize {
    let fmt = read_cstr(emu, fmt_ptr);
    let mut out: Vec<u8> = Vec::with_capacity(fmt.len() + 32);
    let bytes = fmt.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            break;
        }

        // Flags
        let zero_pad = bytes[i] == b'0';
        if zero_pad {
            i += 1;
        }

        // Width
        let mut width = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            width = width * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }

        // Skip length modifier (l, ll, h, z, …)
        while i < bytes.len() && matches!(bytes[i], b'l' | b'h' | b'z' | b'j' | b't') {
            i += 1;
        }

        if i >= bytes.len() {
            break;
        }

        let spec = bytes[i];
        i += 1;

        if spec == b'%' {
            out.push(b'%');
            continue;
        }
        if spec == b'n' {
            // write nothing, consume no arg
            continue;
        }

        let arg = read_u32(emu, vararg_esp);
        vararg_esp += 4;

        let fragment: Vec<u8> = match spec {
            b'd' | b'i' => {
                let s = format!("{}", arg as i32);
                pad(s.into_bytes(), width, if zero_pad { b'0' } else { b' ' }, true)
            }
            b'u' => {
                let s = format!("{}", arg);
                pad(s.into_bytes(), width, if zero_pad { b'0' } else { b' ' }, true)
            }
            b'x' => {
                let s = format!("{:x}", arg);
                pad(s.into_bytes(), width, if zero_pad { b'0' } else { b' ' }, true)
            }
            b'X' => {
                let s = format!("{:X}", arg);
                pad(s.into_bytes(), width, if zero_pad { b'0' } else { b' ' }, true)
            }
            b'o' => {
                let s = format!("{:o}", arg);
                pad(s.into_bytes(), width, if zero_pad { b'0' } else { b' ' }, true)
            }
            b'p' => format!("0x{:x}", arg).into_bytes(),
            b's' => {
                let s = if arg == 0 {
                    b"(null)".to_vec()
                } else {
                    read_cstr(emu, arg).into_bytes()
                };
                pad(s, width, b' ', false)
            }
            b'c' => vec![arg as u8],
            _ => {
                // Unknown specifier — emit literally
                vararg_esp -= 4;
                vec![b'%', spec]
            }
        };
        out.extend_from_slice(&fragment);
    }

    let n = out.len();
    let _ = fs.write_bytes(fd, &out);
    n
}

fn pad(mut b: Vec<u8>, width: usize, pad_char: u8, right_align: bool) -> Vec<u8> {
    if width <= b.len() {
        return b;
    }
    let padding = vec![pad_char; width - b.len()];
    if right_align {
        let mut out = padding;
        out.append(&mut b);
        out
    } else {
        b.extend_from_slice(&padding);
        b
    }
}

// ── guest memory helpers ──────────────────────────────────────────────────────

fn read_u32(emu: &Unicorn<'_, ()>, addr: u32) -> u32 {
    let mut buf = [0u8; 4];
    let _ = emu.mem_read(addr as u64, &mut buf);
    u32::from_le_bytes(buf)
}

fn read_bytes(emu: &Unicorn<'_, ()>, addr: u32, len: usize) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; len];
    let _ = emu.mem_read(addr as u64, &mut buf);
    buf
}

fn read_cstr(emu: &Unicorn<'_, ()>, addr: u32) -> String {
    read_cstr_max(emu, addr, 65_536)
}

fn read_cstr_max(emu: &Unicorn<'_, ()>, addr: u32, max: usize) -> String {
    let mut bytes = Vec::new();
    for i in 0..max {
        let mut b = [0u8; 1];
        if emu.mem_read(addr as u64 + i as u64, &mut b).is_err() {
            break;
        }
        if b[0] == 0 {
            break;
        }
        bytes.push(b[0]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
