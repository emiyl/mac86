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
#[allow(non_snake_case)]
mod CoreFoundation;
mod cf_handle_table;
mod dispatch;
mod fts;
mod math;
mod mem;
mod printf;
mod symbols;
mod trampoline;

pub use dispatch::{handle_libcall, DispatchOutcome, LibCallOutcome};
pub use symbols::{known_symbol, LibSym};
pub use trampoline::{
    Trampoline, CF_APPLY_SENTINEL_ADDR, DEFAULT_RUNE_LOCALE_ADDR, FDOPEN_FILE_BASE,
    OPTARG_STORAGE_ADDR, OPTIND_STORAGE_ADDR, SIGNAL_RETURN_ADDR, STACK_CHK_GUARD_ADDR,
    STDERRP_STORAGE, STDERR_FILE_PTR, STDINP_STORAGE, STDIN_FILE_PTR, STDOUTP_STORAGE,
    STDOUT_FILE_PTR, THREAD_SENTINEL_ADDR, TRAMPOLINE_BASE,
};
