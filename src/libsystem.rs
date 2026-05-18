/// libSystem trampoline — load-time dynamic symbol resolution.
///
/// ## Fixed trampoline slots (always present)
///
/// | Slot | Address      | Purpose                        |
/// |------|------------- |-------------------------------|
/// |  0   | 0x5000_0000  | Exit — main's fake return addr |
/// |  1   | 0x5000_0004  | ThreadSentinel — thread/once fn return |
/// |  2   | 0x5000_0008  | SignalReturn — signal handler return |
/// |  3+  | 0x5000_000C+ | Imported symbols (DyldBindings) |
use crate::dyld::DyldBindings;
use crate::filesystem::VirtualFileSystem;
use crate::threads::ThreadContinuation;
use std::collections::HashMap;
use unicorn_engine::{RegisterX86, Unicorn};

pub const TRAMPOLINE_BASE: u32 = 0x5000_0000;
pub const THREAD_SENTINEL_ADDR: u32 = TRAMPOLINE_BASE + 4;
pub const SIGNAL_RETURN_ADDR: u32 = TRAMPOLINE_BASE + 8;

// ── symbol catalogue ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LibSym {
    // I/O
    Write, Read, Open, Close,
    // Process
    Exit, Abort,
    // Memory
    Malloc, Free, Calloc, Realloc,
    // stdio
    Puts, Printf, Fprintf, Vprintf, Fputs, Fflush,
    Sprintf, Snprintf, Vsnprintf,
    // string
    Strlen, Strcmp, Strncmp, Strcpy, Strncpy, Strcat, Strchr, Strdup,
    Strcasecmp, Strncasecmp, Strstr, Strtok, Strsep, Strrchr,
    // memory
    Memcpy, Memmove, Memset, Memcmp, Memchr,
    // conversions
    Atoi, Atol, Atoll, Strtol, Strtoul, Strtoll, Strtoull, Strtod, Atof,
    // char classification / conversion
    Isdigit, Isalpha, Isalnum, Isspace, Isupper, Islower, Ispunct,
    Toupper, Tolower,
    // sorting
    Qsort, Bsearch, Abs, Labs,
    // env
    Getenv, Setenv, Unsetenv,
    // I/O (misc)
    Perror, Putchar, Getchar,
    // dynamic linking
    Dlopen, Dlsym, Dlclose, Dlerror,
    // pthread Phase 5
    PthreadCreate, PthreadJoin, PthreadSelf,
    PthreadMutexInit, PthreadMutexLock, PthreadMutexUnlock,
    PthreadMutexTrylock, PthreadMutexDestroy,
    PthreadRwlockInit, PthreadRwlockRdlock, PthreadRwlockWrlock,
    PthreadRwlockUnlock, PthreadRwlockDestroy,
    PthreadCondInit, PthreadCondWait, PthreadCondTimedwait,
    PthreadCondSignal, PthreadCondBroadcast, PthreadCondDestroy,
    PthreadAttrInit, PthreadAttrSetdetachstate, PthreadAttrSetstacksize, PthreadAttrDestroy,
    PthreadOnce, PthreadCancel, PthreadTestcancel,
    PthreadKeyCreate, PthreadKeyDelete, PthreadGetspecific, PthreadSetspecific,
    // setjmp / longjmp
    Setjmp, Longjmp,
    // math — results go to x87 ST(0)
    Sin, Cos, Tan, Sqrt, Pow, Log, Log2, Log10, Exp, Exp2,
    Floor, Ceil, Round, Fabs, Fmod,
    Atan, Atan2, Asin, Acos, Sinh, Cosh, Tanh,
    Sinf, Cosf, Tanf, Sqrtf, Powf, Logf, Expf, Fabsf, Floorf, Ceilf,
    // ObjC runtime stubs
    ObjcMsgSend, ObjcMsgSendStret, ObjcGetClass, ObjcLookUpClass,
    NSLog,
    // Internal sentinels
    ThreadSentinel, SignalReturn,
    // Silent no-ops
    Stub0,
}

fn base_name(raw: &str) -> &str {
    let s = raw.trim_start_matches('_');
    if let Some(i) = s.find('$') { &s[..i] } else { s }
}

pub fn known_symbol(name: &str) -> Option<LibSym> {
    match base_name(name) {
        "write"|"write_nocancel"|"pwrite"      => Some(LibSym::Write),
        "read" |"read_nocancel" |"pread"       => Some(LibSym::Read),
        "open" |"open_nocancel"                => Some(LibSym::Open),
        "close"|"close_nocancel"               => Some(LibSym::Close),
        "exit" |"_exit"|"quick_exit"           => Some(LibSym::Exit),
        "abort"                                => Some(LibSym::Abort),
        "malloc"|"malloc_zone_malloc"          => Some(LibSym::Malloc),
        "free"  |"malloc_zone_free"            => Some(LibSym::Free),
        "calloc"|"malloc_zone_calloc"          => Some(LibSym::Calloc),
        "realloc"|"malloc_zone_realloc"        => Some(LibSym::Realloc),
        "puts"                                 => Some(LibSym::Puts),
        "printf"|"__printf_chk"|"printf_chk"   => Some(LibSym::Printf),
        "fprintf"|"__fprintf_chk"              => Some(LibSym::Fprintf),
        "vprintf"|"__vprintf_chk"              => Some(LibSym::Vprintf),
        "fputs"                                => Some(LibSym::Fputs),
        "fflush"                               => Some(LibSym::Fflush),
        "sprintf"|"__sprintf_chk"              => Some(LibSym::Sprintf),
        "snprintf"|"__snprintf_chk"            => Some(LibSym::Snprintf),
        "vsnprintf"|"__vsnprintf_chk"          => Some(LibSym::Vsnprintf),
        "strlen"                               => Some(LibSym::Strlen),
        "strcmp"                               => Some(LibSym::Strcmp),
        "strncmp"                              => Some(LibSym::Strncmp),
        "strcpy"|"__strcpy_chk"                => Some(LibSym::Strcpy),
        "strncpy"|"__strncpy_chk"              => Some(LibSym::Strncpy),
        "strcat"|"__strcat_chk"                => Some(LibSym::Strcat),
        "strchr"                               => Some(LibSym::Strchr),
        "strrchr"                              => Some(LibSym::Strrchr),
        "strdup"                               => Some(LibSym::Strdup),
        "strcasecmp"                           => Some(LibSym::Strcasecmp),
        "strncasecmp"                          => Some(LibSym::Strncasecmp),
        "strstr"                               => Some(LibSym::Strstr),
        "strtok"|"strtok_r"                    => Some(LibSym::Strtok),
        "strsep"                               => Some(LibSym::Strsep),
        "memcpy"|"__memcpy_chk"                => Some(LibSym::Memcpy),
        "memmove"|"__memmove_chk"              => Some(LibSym::Memmove),
        "memset"|"__memset_chk"                => Some(LibSym::Memset),
        "memcmp"                               => Some(LibSym::Memcmp),
        "memchr"                               => Some(LibSym::Memchr),
        "atoi"                                 => Some(LibSym::Atoi),
        "atol"                                 => Some(LibSym::Atol),
        "atoll"                                => Some(LibSym::Atoll),
        "strtol"                               => Some(LibSym::Strtol),
        "strtoul"                              => Some(LibSym::Strtoul),
        "strtoll"                              => Some(LibSym::Strtoll),
        "strtoull"                             => Some(LibSym::Strtoull),
        "strtod"                               => Some(LibSym::Strtod),
        "atof"                                 => Some(LibSym::Atof),
        "isdigit"|"isdigit_l"                  => Some(LibSym::Isdigit),
        "isalpha"|"isalpha_l"                  => Some(LibSym::Isalpha),
        "isalnum"|"isalnum_l"                  => Some(LibSym::Isalnum),
        "isspace"|"isspace_l"                  => Some(LibSym::Isspace),
        "isupper"|"isupper_l"                  => Some(LibSym::Isupper),
        "islower"|"islower_l"                  => Some(LibSym::Islower),
        "ispunct"|"ispunct_l"                  => Some(LibSym::Ispunct),
        "toupper"|"toupper_l"                  => Some(LibSym::Toupper),
        "tolower"|"tolower_l"                  => Some(LibSym::Tolower),
        "qsort"|"qsort_r"                      => Some(LibSym::Qsort),
        "bsearch"                              => Some(LibSym::Bsearch),
        "abs"                                  => Some(LibSym::Abs),
        "labs"                                 => Some(LibSym::Labs),
        "getenv"                               => Some(LibSym::Getenv),
        "setenv"                               => Some(LibSym::Setenv),
        "unsetenv"                             => Some(LibSym::Unsetenv),
        "perror"                               => Some(LibSym::Perror),
        "putchar"|"putchar_unlocked"           => Some(LibSym::Putchar),
        "getchar"|"getchar_unlocked"           => Some(LibSym::Getchar),
        "dlopen"                               => Some(LibSym::Dlopen),
        "dlsym"                                => Some(LibSym::Dlsym),
        "dlclose"                              => Some(LibSym::Dlclose),
        "dlerror"                              => Some(LibSym::Dlerror),
        // pthread
        "pthread_create"                       => Some(LibSym::PthreadCreate),
        "pthread_join"                         => Some(LibSym::PthreadJoin),
        "pthread_self"                         => Some(LibSym::PthreadSelf),
        "pthread_mutex_init"                   => Some(LibSym::PthreadMutexInit),
        "pthread_mutex_lock"                   => Some(LibSym::PthreadMutexLock),
        "pthread_mutex_unlock"                 => Some(LibSym::PthreadMutexUnlock),
        "pthread_mutex_trylock"                => Some(LibSym::PthreadMutexTrylock),
        "pthread_mutex_destroy"                => Some(LibSym::PthreadMutexDestroy),
        "pthread_rwlock_init"                  => Some(LibSym::PthreadRwlockInit),
        "pthread_rwlock_rdlock"                => Some(LibSym::PthreadRwlockRdlock),
        "pthread_rwlock_wrlock"                => Some(LibSym::PthreadRwlockWrlock),
        "pthread_rwlock_unlock"                => Some(LibSym::PthreadRwlockUnlock),
        "pthread_rwlock_destroy"               => Some(LibSym::PthreadRwlockDestroy),
        "pthread_cond_init"                    => Some(LibSym::PthreadCondInit),
        "pthread_cond_wait"                    => Some(LibSym::PthreadCondWait),
        "pthread_cond_timedwait"               => Some(LibSym::PthreadCondTimedwait),
        "pthread_cond_signal"                  => Some(LibSym::PthreadCondSignal),
        "pthread_cond_broadcast"               => Some(LibSym::PthreadCondBroadcast),
        "pthread_cond_destroy"                 => Some(LibSym::PthreadCondDestroy),
        "pthread_attr_init"                    => Some(LibSym::PthreadAttrInit),
        "pthread_attr_setdetachstate"          => Some(LibSym::PthreadAttrSetdetachstate),
        "pthread_attr_setstacksize"            => Some(LibSym::PthreadAttrSetstacksize),
        "pthread_attr_destroy"                 => Some(LibSym::PthreadAttrDestroy),
        "pthread_once"                         => Some(LibSym::PthreadOnce),
        "pthread_cancel"                       => Some(LibSym::PthreadCancel),
        "pthread_testcancel"                   => Some(LibSym::PthreadTestcancel),
        "pthread_key_create"                   => Some(LibSym::PthreadKeyCreate),
        "pthread_key_delete"                   => Some(LibSym::PthreadKeyDelete),
        "pthread_getspecific"                  => Some(LibSym::PthreadGetspecific),
        "pthread_setspecific"                  => Some(LibSym::PthreadSetspecific),
        // setjmp
        "setjmp"|"_setjmp"|"sigsetjmp"         => Some(LibSym::Setjmp),
        "longjmp"|"_longjmp"|"siglongjmp"      => Some(LibSym::Longjmp),
        // math (double)
        "sin"                                  => Some(LibSym::Sin),
        "cos"                                  => Some(LibSym::Cos),
        "tan"                                  => Some(LibSym::Tan),
        "sqrt"                                 => Some(LibSym::Sqrt),
        "pow"                                  => Some(LibSym::Pow),
        "log"                                  => Some(LibSym::Log),
        "log2"                                 => Some(LibSym::Log2),
        "log10"                                => Some(LibSym::Log10),
        "exp"                                  => Some(LibSym::Exp),
        "exp2"                                 => Some(LibSym::Exp2),
        "floor"                                => Some(LibSym::Floor),
        "ceil"                                 => Some(LibSym::Ceil),
        "round"                                => Some(LibSym::Round),
        "fabs"                                 => Some(LibSym::Fabs),
        "fmod"                                 => Some(LibSym::Fmod),
        "atan"                                 => Some(LibSym::Atan),
        "atan2"                                => Some(LibSym::Atan2),
        "asin"                                 => Some(LibSym::Asin),
        "acos"                                 => Some(LibSym::Acos),
        "sinh"                                 => Some(LibSym::Sinh),
        "cosh"                                 => Some(LibSym::Cosh),
        "tanh"                                 => Some(LibSym::Tanh),
        // math (float)
        "sinf"                                 => Some(LibSym::Sinf),
        "cosf"                                 => Some(LibSym::Cosf),
        "tanf"                                 => Some(LibSym::Tanf),
        "sqrtf"                                => Some(LibSym::Sqrtf),
        "powf"                                 => Some(LibSym::Powf),
        "logf"                                 => Some(LibSym::Logf),
        "expf"                                 => Some(LibSym::Expf),
        "fabsf"                                => Some(LibSym::Fabsf),
        "floorf"                               => Some(LibSym::Floorf),
        "ceilf"                                => Some(LibSym::Ceilf),
        // ObjC runtime
        "objc_msgSend"                         => Some(LibSym::ObjcMsgSend),
        "objc_msgSend_stret"                   => Some(LibSym::ObjcMsgSendStret),
        "objc_getClass"                        => Some(LibSym::ObjcGetClass),
        "objc_lookUpClass"                     => Some(LibSym::ObjcLookUpClass),
        "NSLog"                                => Some(LibSym::NSLog),
        // no-op stubs
        "atexit"|"__cxa_atexit"|"__cxa_finalize"|"__cxa_thread_atexit"
        |"setlocale"|"bindtextdomain"|"textdomain"|"tzset"
        |"__pthread_sigmask"|"pthread_sigmask"|"pthread_atfork"
        |"mach_init_routine"|"__dyld_func_lookup"
        |"dyld_stub_binding_helper"|"__keymgr_dwarf2_register_sections"
        |"_Block_object_assign"|"_Block_object_dispose" => Some(LibSym::Stub0),
        _ => None,
    }
}

// ── trampoline table ──────────────────────────────────────────────────────────

pub struct Trampoline {
    pub dispatch: HashMap<u32, LibSym>,
    name_to_addr: HashMap<String, u32>,
    pub slot_count: u32,
}

impl Trampoline {
    pub fn build(bindings: &DyldBindings) -> Self {
        let mut dispatch: HashMap<u32, LibSym> = HashMap::new();
        let mut name_to_addr: HashMap<String, u32> = HashMap::new();
        // sym_to_addr lets us deduplicate symbols that map to the same handler.
        let mut sym_to_addr: HashMap<LibSym, u32> = HashMap::new();

        // Fixed slots 0-2 (always present regardless of imports)
        dispatch.insert(TRAMPOLINE_BASE, LibSym::Exit);
        sym_to_addr.insert(LibSym::Exit, TRAMPOLINE_BASE);
        dispatch.insert(THREAD_SENTINEL_ADDR, LibSym::ThreadSentinel);
        sym_to_addr.insert(LibSym::ThreadSentinel, THREAD_SENTINEL_ADDR);
        dispatch.insert(SIGNAL_RETURN_ADDR, LibSym::SignalReturn);
        sym_to_addr.insert(LibSym::SignalReturn, SIGNAL_RETURN_ADDR);

        let mut slot = 3u32;
        for imp in &bindings.imports {
            // Unknown symbols fall back to Stub0 (return 0, log at debug level).
            // This ensures every pointer slot gets filled with a valid trampoline
            // address; no stub-helper code is ever reached.
            let sym = known_symbol(&imp.name).unwrap_or_else(|| {
                log::debug!("unknown import {:?} → Stub0", imp.name);
                LibSym::Stub0
            });
            let addr = *sym_to_addr.entry(sym).or_insert_with(|| {
                let a = TRAMPOLINE_BASE + slot * 4;
                slot += 1;
                dispatch.insert(a, sym);
                a
            });
            name_to_addr.insert(imp.name.clone(), addr);
        }
        Trampoline { dispatch, name_to_addr, slot_count: slot }
    }

    pub fn exit_addr(&self) -> u32 { TRAMPOLINE_BASE }
    pub fn addr_for_binding(&self, name: &str) -> Option<u32> {
        self.name_to_addr.get(name).copied()
    }
    pub fn region_end(&self) -> u32 { TRAMPOLINE_BASE + (self.slot_count + 1) * 4 }

    /// Return a name→address map for dlsym lookups (leading `_` stripped).
    pub fn symbol_map(&self) -> HashMap<String, u32> {
        self.name_to_addr
            .iter()
            .map(|(k, &v)| (k.trim_start_matches('_').to_string(), v))
            .collect()
    }
}

// ── dispatch outcome ──────────────────────────────────────────────────────────

pub enum DispatchOutcome {
    Ret(u64),
    Exit,
    StateSet,
}

// ── call entry ────────────────────────────────────────────────────────────────

pub enum LibCallOutcome { Continue, Exit }

pub fn handle_libcall(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    sym: LibSym,
) -> LibCallOutcome {
    let esp      = emu.reg_read(RegisterX86::ESP).unwrap_or(0) as u32;
    let ret_addr = read_u32(emu, esp);
    let a0 = read_u32(emu, esp + 4);
    let a1 = read_u32(emu, esp + 8);
    let a2 = read_u32(emu, esp + 12);
    let a3 = read_u32(emu, esp + 16);

    log::debug!("[libsystem] {:?}({:#x}, {:#x}, {:#x}, {:#x})", sym, a0, a1, a2, a3);

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

// ── dispatch ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn dispatch(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    sym: LibSym,
    a0: u32, a1: u32, a2: u32, a3: u32,
    esp: u32, ret_addr: u32,
) -> DispatchOutcome {
    match sym {
        // ── I/O ──────────────────────────────────────────────────────────────
        LibSym::Write => {
            let data = read_bytes(emu, a1, a2 as usize);
            DispatchOutcome::Ret(fs.write_bytes(a0, &data).unwrap_or(0) as u64)
        }
        LibSym::Read => {
            let data = fs.read_bytes(a0, a2 as usize).unwrap_or_default();
            if !data.is_empty() { let _ = emu.mem_write(a1 as u64, &data); }
            DispatchOutcome::Ret(data.len() as u64)
        }
        LibSym::Open => {
            let path = read_cstr(emu, a0);
            match fs.open(std::path::Path::new(&path), (a1 & 0x3) != 0) {
                Ok(fd) => DispatchOutcome::Ret(fd as u64),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Close => { let _ = fs.close(a0); DispatchOutcome::Ret(0) }
        LibSym::Exit | LibSym::Abort => DispatchOutcome::Exit,
        LibSym::Putchar => {
            let _ = fs.write_bytes(1, &[a0 as u8]);
            DispatchOutcome::Ret(a0 as u64 & 0xFF)
        }
        LibSym::Getchar => {
            let data = fs.read_bytes(0, 1).unwrap_or_default();
            DispatchOutcome::Ret(data.first().copied().map(|b| b as u64).unwrap_or(u32::MAX as u64))
        }
        LibSym::Perror => {
            let msg = read_cstr(emu, a0);
            let _ = fs.write_bytes(2, format!("{}: error\n", msg).as_bytes());
            DispatchOutcome::Ret(0)
        }

        // ── Memory ───────────────────────────────────────────────────────────
        LibSym::Malloc => {
            let addr = fs.mmap_anon(if a0 == 0 { 4 } else { (a0 + 15) & !15 }).unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Free => DispatchOutcome::Ret(0),
        LibSym::Calloc => {
            let addr = fs.mmap_anon((a0.saturating_mul(a1).max(4) + 15) & !15).unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Realloc => {
            let new = fs.mmap_anon(if a1 == 0 { 4 } else { (a1 + 15) & !15 }).unwrap_or(0);
            if a0 != 0 && new != 0 { let _ = emu.mem_write(new as u64, &read_bytes(emu, a0, a1 as usize)); }
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
        LibSym::Printf    => DispatchOutcome::Ret(fmt_printf(emu, fs, 1, a0, esp + 8) as u64),
        LibSym::Fprintf   => {
            let fd = if a0 <= 2 { a0 } else { 1 };
            DispatchOutcome::Ret(fmt_printf(emu, fs, fd, a1, esp + 12) as u64)
        }
        LibSym::Vprintf   => DispatchOutcome::Ret(0),
        LibSym::Fputs     => {
            let s = read_cstr(emu, a0);
            let fd = if a1 <= 2 { a1 } else { 1 };
            DispatchOutcome::Ret(fs.write_bytes(fd, s.as_bytes()).unwrap_or(0) as u64)
        }
        LibSym::Fflush    => DispatchOutcome::Ret(0),
        LibSym::Sprintf   => {
            // sprintf(buf, fmt, ...)
            let (text, _n) = format_str(emu, a1, esp + 12);
            let mut out = text.into_bytes(); out.push(0);
            let n = out.len() - 1;
            let _ = emu.mem_write(a0 as u64, &out);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Snprintf  => {
            // snprintf(buf, size, fmt, ...)
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
        LibSym::Strlen    => DispatchOutcome::Ret(read_cstr(emu, a0).len() as u64),
        LibSym::Strcmp    => {
            let r = read_cstr(emu, a0).as_bytes().cmp(read_cstr(emu, a1).as_bytes()) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strncmp   => {
            let n = a2 as usize;
            let s1 = read_cstr_max(emu, a0, n);
            let s2 = read_cstr_max(emu, a1, n);
            let r = s1.as_bytes()[..s1.len().min(n)].cmp(&s2.as_bytes()[..s2.len().min(n)]) as i8 as i32;
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
            let r = s1.as_bytes()[..s1.len().min(n)].cmp(&s2.as_bytes()[..s2.len().min(n)]) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strcpy    => {
            let mut b = read_cstr(emu, a1).into_bytes(); b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strncpy   => {
            let n = a2 as usize;
            let mut b = read_cstr_max(emu, a1, n).into_bytes();
            b.truncate(n); while b.len() < n { b.push(0); }
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strcat    => {
            let mut b = read_cstr(emu, a0).into_bytes();
            b.extend_from_slice(read_cstr(emu, a1).as_bytes());
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strchr    => {
            let s = read_cstr(emu, a0);
            let c = (a1 & 0xFF) as u8;
            match s.as_bytes().iter().position(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None    => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strrchr   => {
            let s = read_cstr(emu, a0);
            let c = (a1 & 0xFF) as u8;
            match s.as_bytes().iter().rposition(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None    => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strstr    => {
            let haystack = read_cstr(emu, a0);
            let needle   = read_cstr(emu, a1);
            if needle.is_empty() { return DispatchOutcome::Ret(a0 as u64); }
            match haystack.find(&needle as &str) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None    => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strdup    => {
            let mut b = read_cstr(emu, a0).into_bytes(); b.push(0);
            let addr = fs.mmap_anon((b.len() as u32 + 15) & !15).unwrap_or(0);
            if addr != 0 { let _ = emu.mem_write(addr as u64, &b); }
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Strtok    => {
            // strtok(str, delim) — very simplified: treats str as a C string, splits once
            // Full strtok needs static state; use a0 == 0 as continuation (return 0)
            if a0 == 0 { return DispatchOutcome::Ret(0); }
            let s = read_cstr(emu, a0);
            let delims = read_cstr(emu, a1);
            match s.find(|c: char| delims.contains(c)) {
                Some(i) => {
                    // Null-terminate at delimiter position
                    let _ = emu.mem_write(a0 as u64 + i as u64, &[0u8]);
                    DispatchOutcome::Ret(a0 as u64)
                }
                None => DispatchOutcome::Ret(a0 as u64),
            }
        }
        LibSym::Strsep    => {
            if a0 == 0 { return DispatchOutcome::Ret(0); }
            let str_ptr_ptr = a0;
            let str_ptr = read_u32(emu, str_ptr_ptr);
            if str_ptr == 0 { return DispatchOutcome::Ret(0); }
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
            let r = read_bytes(emu, a0, a2 as usize).cmp(&read_bytes(emu, a1, a2 as usize)) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Memchr => {
            let buf = read_bytes(emu, a0, a2 as usize);
            let c = (a1 & 0xFF) as u8;
            match buf.iter().position(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None    => DispatchOutcome::Ret(0),
            }
        }

        // ── conversions ───────────────────────────────────────────────────────
        LibSym::Atoi  => DispatchOutcome::Ret(read_cstr(emu, a0).trim().parse::<i32>().unwrap_or(0) as u32 as u64),
        LibSym::Atol  => DispatchOutcome::Ret(read_cstr(emu, a0).trim().parse::<i32>().unwrap_or(0) as u32 as u64),
        LibSym::Atoll => {
            let v = read_cstr(emu, a0).trim().parse::<i64>().unwrap_or(0);
            DispatchOutcome::Ret(v as u64)
        }
        LibSym::Strtol | LibSym::Strtoul | LibSym::Strtoll | LibSym::Strtoull => {
            let s = read_cstr(emu, a0);
            let base = if a2 == 0 { 10 } else { a2 };
            let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
            let v = u64::from_str_radix(s, base).unwrap_or(0);
            // Write endptr if provided
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
        LibSym::Qsort  => DispatchOutcome::Ret(0), // stub — see Phase 7
        LibSym::Bsearch => DispatchOutcome::Ret(0),
        LibSym::Abs    => DispatchOutcome::Ret((a0 as i32).unsigned_abs() as u64),
        LibSym::Labs   => DispatchOutcome::Ret((a0 as i32).unsigned_abs() as u64),

        // ── env ───────────────────────────────────────────────────────────────
        LibSym::Getenv  => DispatchOutcome::Ret(0),
        LibSym::Setenv  => DispatchOutcome::Ret(0),
        LibSym::Unsetenv => DispatchOutcome::Ret(0),

        // ── dynamic linking ───────────────────────────────────────────────────
        LibSym::Dlopen => {
            // dlopen(path, flags) — return a fake handle for known libraries
            let path = if a0 == 0 { String::new() } else { read_cstr(emu, a0) };
            log::debug!("dlopen({:?}, 0x{:x})", path, a1);
            let handle: u32 = if path.is_empty() || path.contains("libSystem")
                || path.contains("libm") || path.contains("libpthread")
                || path.contains("libc") || path.contains("libc++")
                || path.contains("CoreFoundation") || path.contains("Foundation")
                || path.contains("libdyld")
            {
                0x1000_0001 // fake but non-null handle
            } else {
                0 // NULL = failure for unknown libs
            };
            DispatchOutcome::Ret(handle as u64)
        }
        LibSym::Dlsym => {
            // dlsym(handle, symbol)
            let sym_name = read_cstr(emu, a1);
            log::debug!("dlsym({:#x}, {:?})", a0, sym_name);
            let clean = sym_name.trim_start_matches('_');
            let addr = fs.trampoline_map.get(clean).copied().unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Dlclose  => DispatchOutcome::Ret(0),
        LibSym::Dlerror  => DispatchOutcome::Ret(0),

        // ── pthread (Phase 5 — same impl) ────────────────────────────────────
        LibSym::PthreadCreate => {
            let tid_out  = a0;
            let start_fn = a2;
            let arg      = a3;
            let tid = fs.threads.alloc_tid();
            let _ = emu.mem_write(tid_out as u64, &tid.to_le_bytes());

            let stack_size: u32 = 0x1_0000;
            let stack_base = fs.mmap_anon(stack_size).unwrap_or(0);
            let mut tsp = (stack_base + stack_size) & !0xF;
            tsp -= 4; let _ = emu.mem_write(tsp as u64, &arg.to_le_bytes());
            tsp -= 4; let _ = emu.mem_write(tsp as u64, &THREAD_SENTINEL_ADDR.to_le_bytes());

            fs.threads.continuations.push(ThreadContinuation {
                ret_addr, tid,
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
            for r in [RegisterX86::EAX, RegisterX86::EBX, RegisterX86::ECX,
                      RegisterX86::EDX, RegisterX86::ESI, RegisterX86::EDI] {
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
            // Signal handler has returned; restore the interrupted context.
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
                if a1 != 0 { let _ = emu.mem_write(a1 as u64, &(result as u64).to_le_bytes()); }
            }
            DispatchOutcome::Ret(0)
        }
        LibSym::PthreadSelf => DispatchOutcome::Ret(1),
        LibSym::PthreadCancel | LibSym::PthreadTestcancel => DispatchOutcome::Ret(0),
        LibSym::PthreadMutexInit | LibSym::PthreadMutexLock | LibSym::PthreadMutexUnlock
        | LibSym::PthreadMutexTrylock | LibSym::PthreadMutexDestroy
        | LibSym::PthreadRwlockInit | LibSym::PthreadRwlockRdlock | LibSym::PthreadRwlockWrlock
        | LibSym::PthreadRwlockUnlock | LibSym::PthreadRwlockDestroy
        | LibSym::PthreadCondInit | LibSym::PthreadCondWait | LibSym::PthreadCondTimedwait
        | LibSym::PthreadCondSignal | LibSym::PthreadCondBroadcast | LibSym::PthreadCondDestroy
        | LibSym::PthreadAttrInit | LibSym::PthreadAttrSetdetachstate
        | LibSym::PthreadAttrSetstacksize | LibSym::PthreadAttrDestroy => DispatchOutcome::Ret(0),
        LibSym::PthreadOnce => {
            if fs.threads.once_check_and_set(a0) && a1 != 0 {
                let mut tsp = esp;
                tsp -= 4; let _ = emu.mem_write(tsp as u64, &THREAD_SENTINEL_ADDR.to_le_bytes());
                fs.threads.continuations.push(ThreadContinuation {
                    ret_addr, tid: 1,
                    ebx: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
                    ecx: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
                    edx: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
                    esi: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
                    edi: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
                    ebp: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32, esp,
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
            if a0 != 0 { let _ = emu.mem_write(a0 as u64, &key.to_le_bytes()); }
            DispatchOutcome::Ret(0)
        }
        LibSym::PthreadKeyDelete => DispatchOutcome::Ret(0),
        LibSym::PthreadGetspecific => DispatchOutcome::Ret(fs.threads.get_tls(a0) as u64),
        LibSym::PthreadSetspecific => { fs.threads.set_tls(a0, a1); DispatchOutcome::Ret(0) }

        // ── setjmp / longjmp ─────────────────────────────────────────────────
        LibSym::Setjmp => {
            let jbuf = a0;
            let mut buf = [0u8; 32];
            let w = |b: &mut [u8; 32], off: usize, v: u32| b[off..off+4].copy_from_slice(&v.to_le_bytes());
            w(&mut buf, 0, emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32);
            w(&mut buf, 4, emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32);
            w(&mut buf, 8, emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32);
            w(&mut buf,12, emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32);
            w(&mut buf,16, esp + 8); // caller's ESP
            w(&mut buf,20, ret_addr);
            let _ = emu.mem_write(jbuf as u64, &buf);
            DispatchOutcome::Ret(0)
        }
        LibSym::Longjmp => {
            let mut buf = [0u8; 32];
            let _ = emu.mem_read(a0 as u64, &mut buf);
            let r = |b: &[u8; 32], off: usize| u32::from_le_bytes(b[off..off+4].try_into().unwrap_or_default());
            let _ = emu.reg_write(RegisterX86::EBX, r(&buf,  0) as u64);
            let _ = emu.reg_write(RegisterX86::ESI, r(&buf,  4) as u64);
            let _ = emu.reg_write(RegisterX86::EDI, r(&buf,  8) as u64);
            let _ = emu.reg_write(RegisterX86::EBP, r(&buf, 12) as u64);
            let _ = emu.reg_write(RegisterX86::ESP, r(&buf, 16) as u64);
            let _ = emu.reg_write(RegisterX86::EAX, if a1 == 0 { 1 } else { a1 } as u64);
            let _ = emu.set_pc(r(&buf, 20) as u64);
            DispatchOutcome::StateSet
        }

        // ── math ─────────────────────────────────────────────────────────────
        // Double functions: arg at [esp+4..esp+12], result in ST(0).
        LibSym::Sin   => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.sin()); DispatchOutcome::Ret(0) }
        LibSym::Cos   => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.cos()); DispatchOutcome::Ret(0) }
        LibSym::Tan   => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.tan()); DispatchOutcome::Ret(0) }
        LibSym::Sqrt  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.sqrt()); DispatchOutcome::Ret(0) }
        LibSym::Pow   => { let x = read_f64(emu, esp+4); let y = read_f64(emu, esp+12); write_f64_st0(emu, x.powf(y)); DispatchOutcome::Ret(0) }
        LibSym::Log   => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.ln()); DispatchOutcome::Ret(0) }
        LibSym::Log2  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.log2()); DispatchOutcome::Ret(0) }
        LibSym::Log10 => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.log10()); DispatchOutcome::Ret(0) }
        LibSym::Exp   => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.exp()); DispatchOutcome::Ret(0) }
        LibSym::Exp2  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.exp2()); DispatchOutcome::Ret(0) }
        LibSym::Floor => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.floor()); DispatchOutcome::Ret(0) }
        LibSym::Ceil  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.ceil()); DispatchOutcome::Ret(0) }
        LibSym::Round => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.round()); DispatchOutcome::Ret(0) }
        LibSym::Fabs  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.abs()); DispatchOutcome::Ret(0) }
        LibSym::Fmod  => { let x = read_f64(emu, esp+4); let y = read_f64(emu, esp+12); write_f64_st0(emu, x % y); DispatchOutcome::Ret(0) }
        LibSym::Atan  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.atan()); DispatchOutcome::Ret(0) }
        LibSym::Atan2 => { let y = read_f64(emu, esp+4); let x = read_f64(emu, esp+12); write_f64_st0(emu, y.atan2(x)); DispatchOutcome::Ret(0) }
        LibSym::Asin  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.asin()); DispatchOutcome::Ret(0) }
        LibSym::Acos  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.acos()); DispatchOutcome::Ret(0) }
        LibSym::Sinh  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.sinh()); DispatchOutcome::Ret(0) }
        LibSym::Cosh  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.cosh()); DispatchOutcome::Ret(0) }
        LibSym::Tanh  => { let d = read_f64(emu, esp+4); write_f64_st0(emu, d.tanh()); DispatchOutcome::Ret(0) }
        // Float functions: arg at [esp+4], result in ST(0) as f32 widened to f64
        LibSym::Sinf  => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.sin() as f64); DispatchOutcome::Ret(0) }
        LibSym::Cosf  => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.cos() as f64); DispatchOutcome::Ret(0) }
        LibSym::Tanf  => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.tan() as f64); DispatchOutcome::Ret(0) }
        LibSym::Sqrtf => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.sqrt() as f64); DispatchOutcome::Ret(0) }
        LibSym::Powf  => { let x = read_f32(emu, esp+4); let y = read_f32(emu, esp+8); write_f64_st0(emu, x.powf(y) as f64); DispatchOutcome::Ret(0) }
        LibSym::Logf  => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.ln() as f64); DispatchOutcome::Ret(0) }
        LibSym::Expf  => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.exp() as f64); DispatchOutcome::Ret(0) }
        LibSym::Fabsf => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.abs() as f64); DispatchOutcome::Ret(0) }
        LibSym::Floorf=> { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.floor() as f64); DispatchOutcome::Ret(0) }
        LibSym::Ceilf => { let f = read_f32(emu, esp+4); write_f64_st0(emu, f.ceil() as f64); DispatchOutcome::Ret(0) }

        // ── ObjC runtime stubs ────────────────────────────────────────────────
        LibSym::ObjcMsgSend | LibSym::ObjcMsgSendStret => DispatchOutcome::Ret(0),
        LibSym::ObjcGetClass | LibSym::ObjcLookUpClass => DispatchOutcome::Ret(0),
        LibSym::NSLog => {
            // NSLog(NSString *fmt, ...)
            // CFConstantString layout on i386: isa(4) flags(4) cStr(4) len(4)
            let cstr_ptr = read_u32(emu, a0 + 8);
            let fmt_str = if cstr_ptr != 0 { read_cstr(emu, cstr_ptr) } else { read_cstr(emu, a0) };
            let _ = fmt_printf_str(emu, fs, 2, &fmt_str, esp + 8);
            let _ = fs.write_bytes(2, b"\n");
            DispatchOutcome::Ret(0)
        }

        LibSym::Stub0 => DispatchOutcome::Ret(0),
    }
}

// ── printf helpers ────────────────────────────────────────────────────────────

fn fmt_printf(emu: &mut Unicorn<'_, ()>, fs: &mut VirtualFileSystem, fd: u32, fmt_ptr: u32, vararg_esp: u32) -> usize {
    let (text, _) = format_str(emu, fmt_ptr, vararg_esp);
    let n = text.len();
    let _ = fs.write_bytes(fd, text.as_bytes());
    n
}

fn fmt_printf_str(emu: &mut Unicorn<'_, ()>, fs: &mut VirtualFileSystem, fd: u32, fmt: &str, vararg_esp: u32) -> usize {
    let mut vae = vararg_esp;
    let text = do_format(emu, fmt, &mut vae);
    let n = text.len();
    let _ = fs.write_bytes(fd, text.as_bytes());
    n
}

/// Format a printf-style string from guest memory; returns (formatted, len).
fn format_str(emu: &mut Unicorn<'_, ()>, fmt_ptr: u32, vararg_esp: u32) -> (String, usize) {
    let fmt = read_cstr(emu, fmt_ptr);
    let mut vae = vararg_esp;
    let text = do_format(emu, &fmt, &mut vae);
    let n = text.len();
    (text, n)
}

fn do_format(emu: &Unicorn<'_, ()>, fmt: &str, vararg_esp: &mut u32) -> String {
    let mut out = String::with_capacity(fmt.len() + 16);
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' { out.push(bytes[i] as char); i += 1; continue; }
        i += 1;
        if i >= bytes.len() { break; }
        let zero_pad = bytes[i] == b'0';
        if zero_pad { i += 1; }
        let mut width = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() { width = width*10 + (bytes[i]-b'0') as usize; i += 1; }
        while i < bytes.len() && matches!(bytes[i], b'l'|b'h'|b'z'|b'j'|b't') { i += 1; }
        if i >= bytes.len() { break; }
        let spec = bytes[i]; i += 1;
        if spec == b'%' { out.push('%'); continue; }
        if spec == b'n' { continue; }
        let arg = read_u32(emu, *vararg_esp);
        *vararg_esp += 4;
        let frag = match spec {
            b'd'|b'i' => padf(format!("{}", arg as i32), width, if zero_pad{'0'}else{' '}, true),
            b'u'      => padf(format!("{}", arg),        width, if zero_pad{'0'}else{' '}, true),
            b'x'      => padf(format!("{:x}", arg),      width, if zero_pad{'0'}else{' '}, true),
            b'X'      => padf(format!("{:X}", arg),      width, if zero_pad{'0'}else{' '}, true),
            b'o'      => padf(format!("{:o}", arg),      width, if zero_pad{'0'}else{' '}, true),
            b'p'      => format!("0x{:x}", arg),
            b's'      => {
                let s = if arg == 0 { "(null)".to_string() } else { read_cstr(emu, arg) };
                padf(s, width, ' ', false)
            }
            b'c'      => (arg as u8 as char).to_string(),
            _         => { *vararg_esp -= 4; format!("%{}", spec as char) }
        };
        out.push_str(&frag);
    }
    out
}

fn padf(s: String, width: usize, pad: char, right: bool) -> String {
    if width <= s.len() { return s; }
    let padding: String = std::iter::repeat_n(pad, width - s.len()).collect();
    if right { format!("{}{}", padding, s) } else { format!("{}{}", s, padding) }
}

// ── FPU helpers ───────────────────────────────────────────────────────────────

fn read_f64(emu: &Unicorn<'_, ()>, addr: u32) -> f64 {
    let lo = read_u32(emu, addr) as u64;
    let hi = read_u32(emu, addr + 4) as u64;
    f64::from_bits(lo | (hi << 32))
}

fn read_f32(emu: &Unicorn<'_, ()>, addr: u32) -> f32 {
    f32::from_bits(read_u32(emu, addr))
}

/// Write an f64 result to ST(0) using the 80-bit x87 extended-precision format.
/// Converts from IEEE 754 double (64-bit) to x87 extended (80-bit) then writes
/// via Unicorn's reg_write to ST0.  On architectures where this works the FPU
/// will return the correct value; otherwise the caller sees 0 in EAX.
fn write_f64_st0(emu: &mut Unicorn<'_, ()>, value: f64) {
    let bytes = f64_to_x87(value);
    // Unicorn's reg_write for ST0 accepts the 10-byte extended value packed as two u64
    // via mem_write tricks if needed; for now write the lower 8 bytes as u64.
    let low8 = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or_default());
    let _ = emu.reg_write(RegisterX86::ST0, low8);
}

/// Convert IEEE 754 double to 80-bit x87 extended-precision (10 bytes, little-endian).
fn f64_to_x87(d: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if d == 0.0 { return out; }
    let bits = d.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    let exp64 = ((bits >> 52) & 0x7FF) as i32;
    let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
    if exp64 == 0x7FF {
        // Inf/NaN
        let exp80 = 0x7FFFu16 | sign;
        let sig: u64 = if mantissa == 0 { 0x8000_0000_0000_0000 } else { 0xC000_0000_0000_0000 };
        out[0..8].copy_from_slice(&sig.to_le_bytes());
        out[8..10].copy_from_slice(&exp80.to_le_bytes());
    } else {
        let exp80 = ((exp64 - 1023 + 16383) as u16) | sign;
        // Explicit integer bit + 63-bit fraction
        let sig = 0x8000_0000_0000_0000u64 | (mantissa << 11);
        out[0..8].copy_from_slice(&sig.to_le_bytes());
        out[8..10].copy_from_slice(&exp80.to_le_bytes());
    }
    out
}

// ── guest memory helpers ──────────────────────────────────────────────────────

fn read_u32(emu: &Unicorn<'_, ()>, addr: u32) -> u32 {
    let mut buf = [0u8; 4];
    let _ = emu.mem_read(addr as u64, &mut buf);
    u32::from_le_bytes(buf)
}

fn read_bytes(emu: &Unicorn<'_, ()>, addr: u32, len: usize) -> Vec<u8> {
    if len == 0 { return vec![]; }
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
        if emu.mem_read(addr as u64 + i as u64, &mut b).is_err() { break; }
        if b[0] == 0 { break; }
        bytes.push(b[0]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
