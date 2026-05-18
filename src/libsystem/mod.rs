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

mod dispatch;
mod fts;
mod math;
mod mem;
mod printf;
mod symbols;
mod trampoline;

pub use dispatch::{DispatchOutcome, LibCallOutcome, handle_libcall};
pub use symbols::{known_symbol, LibSym};
pub use trampoline::{
    Trampoline, OPTARG_STORAGE_ADDR, OPTIND_STORAGE_ADDR, SIGNAL_RETURN_ADDR,
    THREAD_SENTINEL_ADDR, TRAMPOLINE_BASE,
};
