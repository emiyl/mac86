/// libSystem trampoline — load-time dynamic symbol resolution for Phase 3+.
///
/// ## Trampoline region layout (0x5000_0000)
///
/// | Slot | Address      | Purpose                                 |
/// |------|------------- |-----------------------------------------|
/// |  0   | 0x5000_0000  | Exit (always, main's fake return addr)  |
/// |  1   | 0x5000_0004  | ThreadSentinel (eager thread completion)|
/// |  2+  | 0x5000_0008+ | Imported symbols from DyldBindings      |
///
/// ## Threading model
///
/// Threads run *eagerly* at `pthread_create` time using a context-switch
/// approach: the current register state (the "calling thread") is saved on a
/// continuations stack, the CPU is pointed at the new thread's entry function,
/// and when the thread returns to `THREAD_SENTINEL_ADDR` the saved state is
/// popped and the calling thread continues from the `pthread_create` return site.
/// This is cooperative (not concurrent) but correct for the common create→join
/// pattern.
use crate::dyld::DyldBindings;
use crate::filesystem::VirtualFileSystem;
use crate::threads::ThreadContinuation;
use std::collections::HashMap;
use unicorn_engine::{RegisterX86, Unicorn};

pub const TRAMPOLINE_BASE: u32 = 0x5000_0000;
/// Address the thread function returns to when it finishes.
pub const THREAD_SENTINEL_ADDR: u32 = TRAMPOLINE_BASE + 4;

// ── symbol table ─────────────────────────────────────────────────────────────

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
    // string / memory
    Strlen, Strcmp, Strncmp, Strcpy, Strncpy, Strcat, Strchr, Strdup,
    Memcpy, Memmove, Memset, Memcmp,
    // env
    Getenv,
    // Phase 5 — pthread
    PthreadCreate,
    PthreadJoin,
    PthreadSelf,
    PthreadMutexInit,
    PthreadMutexLock,
    PthreadMutexUnlock,
    PthreadMutexTrylock,
    PthreadMutexDestroy,
    PthreadRwlockInit,
    PthreadRwlockRdlock,
    PthreadRwlockWrlock,
    PthreadRwlockUnlock,
    PthreadRwlockDestroy,
    PthreadCondInit,
    PthreadCondWait,
    PthreadCondTimedwait,
    PthreadCondSignal,
    PthreadCondBroadcast,
    PthreadCondDestroy,
    PthreadAttrInit,
    PthreadAttrSetdetachstate,
    PthreadAttrSetstacksize,
    PthreadAttrDestroy,
    PthreadOnce,
    PthreadKeyCreate,
    PthreadKeyDelete,
    PthreadGetspecific,
    PthreadSetspecific,
    // Phase 5 — setjmp / longjmp
    Setjmp,
    Longjmp,
    // Internal
    ThreadSentinel,
    // Silent no-op stubs (return 0)
    Stub0,
}

fn base_name(raw: &str) -> &str {
    let s = raw.trim_start_matches('_');
    if let Some(i) = s.find('$') { &s[..i] } else { s }
}

pub fn known_symbol(name: &str) -> Option<LibSym> {
    match base_name(name) {
        "write" | "write_nocancel" | "pwrite" => Some(LibSym::Write),
        "read"  | "read_nocancel"  | "pread"  => Some(LibSym::Read),
        "open"  | "open_nocancel"             => Some(LibSym::Open),
        "close" | "close_nocancel"            => Some(LibSym::Close),
        "exit"  | "_exit" | "quick_exit"      => Some(LibSym::Exit),
        "abort"                                => Some(LibSym::Abort),
        "malloc" | "malloc_zone_malloc"        => Some(LibSym::Malloc),
        "free"   | "malloc_zone_free"          => Some(LibSym::Free),
        "calloc" | "malloc_zone_calloc"        => Some(LibSym::Calloc),
        "realloc"| "malloc_zone_realloc"       => Some(LibSym::Realloc),
        "puts"                                 => Some(LibSym::Puts),
        "printf" | "__printf_chk" | "printf_chk" => Some(LibSym::Printf),
        "fprintf"| "__fprintf_chk"             => Some(LibSym::Fprintf),
        "vprintf"| "__vprintf_chk"             => Some(LibSym::Vprintf),
        "fputs"                                => Some(LibSym::Fputs),
        "fflush"                               => Some(LibSym::Fflush),
        "strlen"                               => Some(LibSym::Strlen),
        "strcmp"                               => Some(LibSym::Strcmp),
        "strncmp"                              => Some(LibSym::Strncmp),
        "strcpy" | "__strcpy_chk"              => Some(LibSym::Strcpy),
        "strncpy"| "__strncpy_chk"             => Some(LibSym::Strncpy),
        "strcat" | "__strcat_chk"              => Some(LibSym::Strcat),
        "strchr"                               => Some(LibSym::Strchr),
        "strdup"                               => Some(LibSym::Strdup),
        "memcpy" | "__memcpy_chk"              => Some(LibSym::Memcpy),
        "memmove"| "__memmove_chk"             => Some(LibSym::Memmove),
        "memset" | "__memset_chk"              => Some(LibSym::Memset),
        "memcmp"                               => Some(LibSym::Memcmp),
        "getenv"                               => Some(LibSym::Getenv),
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
        "pthread_key_create"                   => Some(LibSym::PthreadKeyCreate),
        "pthread_key_delete"                   => Some(LibSym::PthreadKeyDelete),
        "pthread_getspecific"                  => Some(LibSym::PthreadGetspecific),
        "pthread_setspecific"                  => Some(LibSym::PthreadSetspecific),
        // setjmp / longjmp (several aliases)
        "setjmp" | "_setjmp" | "sigsetjmp"    => Some(LibSym::Setjmp),
        "longjmp"| "_longjmp"| "siglongjmp"   => Some(LibSym::Longjmp),
        // No-op stubs
        "atexit" | "__cxa_atexit" | "__cxa_finalize" | "__cxa_thread_atexit"
        | "setlocale" | "bindtextdomain" | "textdomain" | "tzset"
        | "__pthread_sigmask" | "pthread_atfork"
        | "mach_init_routine" | "__dyld_func_lookup"
        | "dyld_stub_binding_helper" | "__keymgr_dwarf2_register_sections"
        | "pthread_sigmask" => Some(LibSym::Stub0),
        _ => None,
    }
}

// ── trampoline table ──────────────────────────────────────────────────────────

pub struct Trampoline {
    pub dispatch: HashMap<u32, LibSym>,
    name_to_addr: HashMap<String, u32>,
    #[allow(dead_code)]
    sym_to_addr: HashMap<LibSym, u32>,
    pub slot_count: u32,
}

impl Trampoline {
    pub fn build(bindings: &DyldBindings) -> Self {
        let mut dispatch: HashMap<u32, LibSym> = HashMap::new();
        let mut name_to_addr: HashMap<String, u32> = HashMap::new();
        let mut sym_to_addr: HashMap<LibSym, u32> = HashMap::new();

        // Fixed slots (always present regardless of imports)
        dispatch.insert(TRAMPOLINE_BASE, LibSym::Exit);
        sym_to_addr.insert(LibSym::Exit, TRAMPOLINE_BASE);
        dispatch.insert(THREAD_SENTINEL_ADDR, LibSym::ThreadSentinel);
        sym_to_addr.insert(LibSym::ThreadSentinel, THREAD_SENTINEL_ADDR);

        let mut slot = 2u32; // imported symbols start at slot 2

        for imp in &bindings.imports {
            let Some(sym) = known_symbol(&imp.name) else { continue };
            let addr = *sym_to_addr.entry(sym).or_insert_with(|| {
                let a = TRAMPOLINE_BASE + slot * 4;
                slot += 1;
                dispatch.insert(a, sym);
                a
            });
            name_to_addr.insert(imp.name.clone(), addr);
        }

        Trampoline { dispatch, name_to_addr, sym_to_addr, slot_count: slot }
    }

    pub fn exit_addr(&self) -> u32 { TRAMPOLINE_BASE }
    pub fn addr_for_binding(&self, name: &str) -> Option<u32> {
        self.name_to_addr.get(name).copied()
    }
    pub fn region_end(&self) -> u32 {
        TRAMPOLINE_BASE + (self.slot_count + 1) * 4
    }
}

// ── dispatch outcome ──────────────────────────────────────────────────────────

/// What `handle_libcall` should do after `dispatch()` returns.
pub enum DispatchOutcome {
    /// Normal return: advance ESP past return addr, set PC to ret_addr, set EAX.
    Ret(u64),
    /// Stop the emulation (exit / abort / thread sentinel with no continuation).
    Exit,
    /// Caller has already set PC/ESP/EAX directly; skip the standard RET simulation.
    StateSet,
}

// ── call entry point ─────────────────────────────────────────────────────────

pub enum LibCallOutcome { Continue, Exit }

pub fn handle_libcall(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    sym: LibSym,
) -> LibCallOutcome {
    let esp     = emu.reg_read(RegisterX86::ESP).unwrap_or(0) as u32;
    let ret_addr = read_u32(emu, esp);
    let a0 = read_u32(emu, esp + 4);
    let a1 = read_u32(emu, esp + 8);
    let a2 = read_u32(emu, esp + 12);
    let a3 = read_u32(emu, esp + 16);

    log::debug!("[libsystem] {:?}({:#x}, {:#x}, {:#x}, {:#x})", sym, a0, a1, a2, a3);

    let outcome = dispatch(emu, fs, sym, a0, a1, a2, a3, esp, ret_addr);

    match outcome {
        DispatchOutcome::Ret(retval) => {
            let _ = emu.reg_write(RegisterX86::ESP, (esp + 4) as u64);
            let _ = emu.set_pc(ret_addr as u64);
            let _ = emu.reg_write(RegisterX86::EAX, retval & 0xFFFF_FFFF);
            let _ = emu.reg_write(RegisterX86::EDX, retval >> 32);
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
    esp: u32,
    ret_addr: u32,
) -> DispatchOutcome {
    match sym {
        // ── I/O ──────────────────────────────────────────────────────────────
        LibSym::Write => {
            let data = read_bytes(emu, a1, a2 as usize);
            let n = fs.write_bytes(a0, &data).unwrap_or(0);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Read => {
            let data = fs.read_bytes(a0, a2 as usize).unwrap_or_default();
            if !data.is_empty() { let _ = emu.mem_write(a1 as u64, &data); }
            DispatchOutcome::Ret(data.len() as u64)
        }
        LibSym::Open => {
            let path = read_cstr(emu, a0);
            let writable = (a1 & 0x3) != 0;
            match fs.open(std::path::Path::new(&path), writable) {
                Ok(fd) => DispatchOutcome::Ret(fd as u64),
                Err(_) => DispatchOutcome::Ret(u32::MAX as u64),
            }
        }
        LibSym::Close => {
            let _ = fs.close(a0);
            DispatchOutcome::Ret(0)
        }
        LibSym::Exit | LibSym::Abort => DispatchOutcome::Exit,

        // ── Memory ───────────────────────────────────────────────────────────
        LibSym::Malloc => {
            let sz = if a0 == 0 { 4 } else { (a0 + 15) & !15 };
            let addr = fs.mmap_anon(sz).unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Free => DispatchOutcome::Ret(0),
        LibSym::Calloc => {
            let total = a0.saturating_mul(a1).max(4);
            let addr = fs.mmap_anon((total + 15) & !15).unwrap_or(0);
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Realloc => {
            let sz = if a1 == 0 { 4 } else { (a1 + 15) & !15 };
            let new_addr = fs.mmap_anon(sz).unwrap_or(0);
            if a0 != 0 && new_addr != 0 {
                let old = read_bytes(emu, a0, a1 as usize);
                let _ = emu.mem_write(new_addr as u64, &old);
            }
            DispatchOutcome::Ret(new_addr as u64)
        }

        // ── stdio ─────────────────────────────────────────────────────────────
        LibSym::Puts => {
            let s = read_cstr(emu, a0);
            let mut out = s.into_bytes();
            out.push(b'\n');
            let n = out.len();
            let _ = fs.write_bytes(1, &out);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Printf => {
            let n = fmt_printf(emu, fs, 1, a0, esp + 8);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Fprintf => {
            let fd = if a0 <= 2 { a0 } else { 1 };
            let n = fmt_printf(emu, fs, fd, a1, esp + 12);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Vprintf => DispatchOutcome::Ret(0),
        LibSym::Fputs => {
            let s = read_cstr(emu, a0);
            let fd = if a1 <= 2 { a1 } else { 1 };
            let n = fs.write_bytes(fd, s.as_bytes()).unwrap_or(0);
            DispatchOutcome::Ret(n as u64)
        }
        LibSym::Fflush => DispatchOutcome::Ret(0),

        // ── string / memory ───────────────────────────────────────────────────
        LibSym::Strlen => DispatchOutcome::Ret(read_cstr(emu, a0).len() as u64),
        LibSym::Strcmp => {
            let s1 = read_cstr(emu, a0);
            let s2 = read_cstr(emu, a1);
            let r = s1.as_bytes().cmp(s2.as_bytes()) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strncmp => {
            let n = a2 as usize;
            let s1 = read_cstr_max(emu, a0, n);
            let s2 = read_cstr_max(emu, a1, n);
            let r = s1.as_bytes()[..s1.len().min(n)]
                .cmp(&s2.as_bytes()[..s2.len().min(n)]) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Strcpy => {
            let s = read_cstr(emu, a1);
            let mut b = s.into_bytes(); b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strncpy => {
            let n = a2 as usize;
            let s = read_cstr_max(emu, a1, n);
            let mut b = s.into_bytes();
            b.truncate(n);
            while b.len() < n { b.push(0); }
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strcat => {
            let dest = read_cstr(emu, a0);
            let src  = read_cstr(emu, a1);
            let mut b = dest.into_bytes();
            b.extend_from_slice(src.as_bytes());
            b.push(0);
            let _ = emu.mem_write(a0 as u64, &b);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Strchr => {
            let s = read_cstr(emu, a0);
            let c = (a1 & 0xFF) as u8;
            match s.as_bytes().iter().position(|&b| b == c) {
                Some(i) => DispatchOutcome::Ret(a0 as u64 + i as u64),
                None    => DispatchOutcome::Ret(0),
            }
        }
        LibSym::Strdup => {
            let s = read_cstr(emu, a0);
            let mut b = s.into_bytes(); b.push(0);
            let len = b.len() as u32;
            let addr = fs.mmap_anon((len + 15) & !15).unwrap_or(0);
            if addr != 0 { let _ = emu.mem_write(addr as u64, &b); }
            DispatchOutcome::Ret(addr as u64)
        }
        LibSym::Memcpy | LibSym::Memmove => {
            let data = read_bytes(emu, a1, a2 as usize);
            let _ = emu.mem_write(a0 as u64, &data);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Memset => {
            let buf = vec![(a1 & 0xFF) as u8; a2 as usize];
            let _ = emu.mem_write(a0 as u64, &buf);
            DispatchOutcome::Ret(a0 as u64)
        }
        LibSym::Memcmp => {
            let b1 = read_bytes(emu, a0, a2 as usize);
            let b2 = read_bytes(emu, a1, a2 as usize);
            let r = b1.cmp(&b2) as i8 as i32;
            DispatchOutcome::Ret(r as u32 as u64)
        }
        LibSym::Getenv => DispatchOutcome::Ret(0),

        // ── Phase 5: pthreads ─────────────────────────────────────────────────

        LibSym::PthreadCreate => {
            // pthread_create(pthread_t *tid_out, attr, start_fn, arg)
            let tid_out  = a0;
            let start_fn = a2;
            let arg      = a3;

            let tid = fs.threads.alloc_tid();

            // Write the new tid to *tid_out
            let _ = emu.mem_write(tid_out as u64, &tid.to_le_bytes());

            // Allocate a 64 KB stack for the thread.
            let stack_size: u32 = 0x1_0000;
            let stack_base = fs.mmap_anon(stack_size).unwrap_or(0);
            // Stack top, 16-byte aligned.
            let mut tsp = (stack_base + stack_size) & !0xF;

            // Push arg (the thread function's single argument).
            tsp -= 4;
            let _ = emu.mem_write(tsp as u64, &arg.to_le_bytes());
            // Push THREAD_SENTINEL_ADDR as the fake return address for start_fn.
            tsp -= 4;
            let _ = emu.mem_write(tsp as u64, &THREAD_SENTINEL_ADDR.to_le_bytes());

            // Save the calling thread's context so we can restore it when the
            // new thread finishes.
            let cont = ThreadContinuation {
                ret_addr,
                tid,
                ebx: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
                ecx: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
                edx: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
                esi: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
                edi: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
                ebp: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
                esp,
            };
            fs.threads.continuations.push(cont);

            // Switch the CPU to the thread's entry point and stack.
            let _ = emu.reg_write(RegisterX86::ESP, tsp as u64);
            let _ = emu.reg_write(RegisterX86::EBP, 0u64);
            let _ = emu.reg_write(RegisterX86::EAX, 0u64);
            let _ = emu.reg_write(RegisterX86::EBX, 0u64);
            let _ = emu.reg_write(RegisterX86::ECX, 0u64);
            let _ = emu.reg_write(RegisterX86::EDX, 0u64);
            let _ = emu.reg_write(RegisterX86::ESI, 0u64);
            let _ = emu.reg_write(RegisterX86::EDI, 0u64);
            let _ = emu.set_pc(start_fn as u64);

            // Do NOT simulate RET: Unicorn will continue at start_fn.
            DispatchOutcome::StateSet
        }

        LibSym::ThreadSentinel => {
            // The thread function has returned.  EAX holds its return value.
            let thread_retval = emu.reg_read(RegisterX86::EAX).unwrap_or(0) as u32;

            if let Some(cont) = fs.threads.continuations.pop() {
                // Store the thread's result.
                fs.threads.store_result(cont.tid, thread_retval);

                // Restore the calling thread's register state.
                let _ = emu.reg_write(RegisterX86::EBX, cont.ebx as u64);
                let _ = emu.reg_write(RegisterX86::ECX, cont.ecx as u64);
                let _ = emu.reg_write(RegisterX86::EDX, cont.edx as u64);
                let _ = emu.reg_write(RegisterX86::ESI, cont.esi as u64);
                let _ = emu.reg_write(RegisterX86::EDI, cont.edi as u64);
                let _ = emu.reg_write(RegisterX86::EBP, cont.ebp as u64);
                // Caller's ESP: past the return address that was on its stack.
                let _ = emu.reg_write(RegisterX86::ESP, (cont.esp + 4) as u64);
                // pthread_create returns 0 (success).
                let _ = emu.reg_write(RegisterX86::EAX, 0u64);
                let _ = emu.set_pc(cont.ret_addr as u64);

                DispatchOutcome::StateSet
            } else {
                // No saved continuation — treat like exit().
                DispatchOutcome::Exit
            }
        }

        LibSym::PthreadJoin => {
            // pthread_join(tid, void **retval)
            let tid        = a0;
            let retval_ptr = a1;
            if let Some(result) = fs.threads.get_result(tid) {
                if retval_ptr != 0 {
                    // Store the result as a pointer-sized value.
                    let _ = emu.mem_write(retval_ptr as u64, &(result as u64).to_le_bytes());
                }
            }
            // If the thread hasn't run yet (shouldn't happen with eager model), return 0.
            DispatchOutcome::Ret(0)
        }

        LibSym::PthreadSelf => DispatchOutcome::Ret(1), // main thread = 1

        // Mutexes — no-ops (single-threaded cooperative model, no contention)
        LibSym::PthreadMutexInit
        | LibSym::PthreadMutexLock
        | LibSym::PthreadMutexUnlock
        | LibSym::PthreadMutexTrylock
        | LibSym::PthreadMutexDestroy => DispatchOutcome::Ret(0),

        // RW locks — no-ops
        LibSym::PthreadRwlockInit
        | LibSym::PthreadRwlockRdlock
        | LibSym::PthreadRwlockWrlock
        | LibSym::PthreadRwlockUnlock
        | LibSym::PthreadRwlockDestroy => DispatchOutcome::Ret(0),

        // Condition variables — no-ops
        LibSym::PthreadCondInit
        | LibSym::PthreadCondWait
        | LibSym::PthreadCondTimedwait
        | LibSym::PthreadCondSignal
        | LibSym::PthreadCondBroadcast
        | LibSym::PthreadCondDestroy => DispatchOutcome::Ret(0),

        // Thread attributes — no-ops
        LibSym::PthreadAttrInit
        | LibSym::PthreadAttrSetdetachstate
        | LibSym::PthreadAttrSetstacksize
        | LibSym::PthreadAttrDestroy => DispatchOutcome::Ret(0),

        LibSym::PthreadOnce => {
            // pthread_once(once_t *ctl, void (*init_fn)(void))
            // a0 = once_t*, a1 = init_fn
            if fs.threads.once_check_and_set(a0) && a1 != 0 {
                // Run init_fn() inline: set up a call frame and switch to it.
                // We save a continuation so ThreadSentinel can return here.
                let mut tsp = esp; // reuse caller's stack area (above the return addr)
                // Push THREAD_SENTINEL_ADDR as the return address for init_fn.
                tsp -= 4;
                let _ = emu.mem_write(tsp as u64, &THREAD_SENTINEL_ADDR.to_le_bytes());

                let cont = ThreadContinuation {
                    ret_addr,
                    tid: 1, // main thread
                    ebx: emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32,
                    ecx: emu.reg_read(RegisterX86::ECX).unwrap_or(0) as u32,
                    edx: emu.reg_read(RegisterX86::EDX).unwrap_or(0) as u32,
                    esi: emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32,
                    edi: emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32,
                    ebp: emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32,
                    esp,
                };
                fs.threads.continuations.push(cont);

                let _ = emu.reg_write(RegisterX86::ESP, tsp as u64);
                let _ = emu.set_pc(a1 as u64);
                DispatchOutcome::StateSet
            } else {
                DispatchOutcome::Ret(0)
            }
        }

        LibSym::PthreadKeyCreate => {
            // pthread_key_create(key_t *key, destructor)
            let key = fs.threads.create_key();
            if a0 != 0 {
                let _ = emu.mem_write(a0 as u64, &key.to_le_bytes());
            }
            DispatchOutcome::Ret(0)
        }
        LibSym::PthreadKeyDelete => DispatchOutcome::Ret(0),
        LibSym::PthreadGetspecific => {
            DispatchOutcome::Ret(fs.threads.get_tls(a0) as u64)
        }
        LibSym::PthreadSetspecific => {
            fs.threads.set_tls(a0, a1);
            DispatchOutcome::Ret(0)
        }

        // ── Phase 5: setjmp / longjmp ─────────────────────────────────────────
        //
        // Our jmp_buf layout (32 bytes, 8 × u32 at jmp_buf[0..]):
        //   [0] EBX  [1] ESI  [2] EDI  [3] EBP
        //   [4] ESP (caller's, = esp + 8 at setjmp call — past ret_addr + jmp_buf arg)
        //   [5] EIP (the return address = setjmp return site)
        //   [6,7] reserved

        LibSym::Setjmp => {
            // setjmp(jmp_buf *env)
            // At entry: [esp] = ret_addr, [esp+4] = jmp_buf*
            let jbuf = a0;
            let saved_esp = esp + 8; // caller's ESP after setjmp fully returns

            let mut buf = [0u8; 32];
            let write32 = |b: &mut [u8; 32], off: usize, v: u32| {
                b[off..off + 4].copy_from_slice(&v.to_le_bytes());
            };
            write32(&mut buf, 0, emu.reg_read(RegisterX86::EBX).unwrap_or(0) as u32);
            write32(&mut buf, 4, emu.reg_read(RegisterX86::ESI).unwrap_or(0) as u32);
            write32(&mut buf, 8, emu.reg_read(RegisterX86::EDI).unwrap_or(0) as u32);
            write32(&mut buf, 12, emu.reg_read(RegisterX86::EBP).unwrap_or(0) as u32);
            write32(&mut buf, 16, saved_esp);
            write32(&mut buf, 20, ret_addr);
            let _ = emu.mem_write(jbuf as u64, &buf);

            // Return 0 from setjmp via standard RET simulation.
            DispatchOutcome::Ret(0)
        }

        LibSym::Longjmp => {
            // longjmp(jmp_buf *env, int val)
            // Restore saved state and jump back to the setjmp call site.
            let jbuf = a0;
            let val  = if a1 == 0 { 1 } else { a1 };

            let mut buf = [0u8; 32];
            let _ = emu.mem_read(jbuf as u64, &mut buf);
            let read32 = |b: &[u8; 32], off: usize| -> u32 {
                u32::from_le_bytes(b[off..off + 4].try_into().unwrap_or_default())
            };
            let r_ebx = read32(&buf, 0);
            let r_esi = read32(&buf, 4);
            let r_edi = read32(&buf, 8);
            let r_ebp = read32(&buf, 12);
            let r_esp = read32(&buf, 16);
            let r_eip = read32(&buf, 20);

            let _ = emu.reg_write(RegisterX86::EBX, r_ebx as u64);
            let _ = emu.reg_write(RegisterX86::ESI, r_esi as u64);
            let _ = emu.reg_write(RegisterX86::EDI, r_edi as u64);
            let _ = emu.reg_write(RegisterX86::EBP, r_ebp as u64);
            let _ = emu.reg_write(RegisterX86::ESP, r_esp as u64);
            let _ = emu.reg_write(RegisterX86::EAX, val as u64);
            let _ = emu.set_pc(r_eip as u64);

            DispatchOutcome::StateSet // we already set everything
        }

        LibSym::Stub0 => DispatchOutcome::Ret(0),
    }
}

// ── printf ────────────────────────────────────────────────────────────────────

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
        if bytes[i] != b'%' { out.push(bytes[i]); i += 1; continue; }
        i += 1;
        if i >= bytes.len() { break; }

        let zero_pad = bytes[i] == b'0';
        if zero_pad { i += 1; }
        let mut width = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            width = width * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }
        while i < bytes.len() && matches!(bytes[i], b'l' | b'h' | b'z' | b'j' | b't') {
            i += 1;
        }
        if i >= bytes.len() { break; }

        let spec = bytes[i]; i += 1;
        if spec == b'%' { out.push(b'%'); continue; }
        if spec == b'n' { continue; }

        let arg = read_u32(emu, vararg_esp);
        vararg_esp += 4;

        let frag: Vec<u8> = match spec {
            b'd' | b'i' => pad(format!("{}", arg as i32).into_bytes(), width, if zero_pad { b'0' } else { b' ' }, true),
            b'u' =>        pad(format!("{}", arg).into_bytes(),         width, if zero_pad { b'0' } else { b' ' }, true),
            b'x' =>        pad(format!("{:x}", arg).into_bytes(),       width, if zero_pad { b'0' } else { b' ' }, true),
            b'X' =>        pad(format!("{:X}", arg).into_bytes(),       width, if zero_pad { b'0' } else { b' ' }, true),
            b'o' =>        pad(format!("{:o}", arg).into_bytes(),       width, if zero_pad { b'0' } else { b' ' }, true),
            b'p' => format!("0x{:x}", arg).into_bytes(),
            b's' => {
                let s = if arg == 0 { b"(null)".to_vec() } else { read_cstr(emu, arg).into_bytes() };
                pad(s, width, b' ', false)
            }
            b'c' => vec![arg as u8],
            _ => { vararg_esp -= 4; vec![b'%', spec] }
        };
        out.extend_from_slice(&frag);
    }

    let n = out.len();
    let _ = fs.write_bytes(fd, &out);
    n
}

fn pad(mut b: Vec<u8>, width: usize, pad_char: u8, right_align: bool) -> Vec<u8> {
    if width <= b.len() { return b; }
    let padding = vec![pad_char; width - b.len()];
    if right_align { let mut out = padding; out.append(&mut b); out }
    else            { b.extend_from_slice(&padding); b }
}

// ── guest memory helpers ──────────────────────────────────────────────────────

fn read_u32(emu: &Unicorn<'_, ()>, addr: u32) -> u32 {
    let mut buf = [0u8; 4];
    let _ = emu.mem_read(addr as u64, &mut buf);
    u32::from_le_bytes(buf)
}

fn read_bytes(emu: &Unicorn<'_, ()>, addr: u32, len: usize) -> Vec<u8> {
    if len == 0 { return Vec::new(); }
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
