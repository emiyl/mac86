/// Thread state for the emulated process.
///
/// We use an **eager cooperative** model: a new thread starts executing
/// immediately when `pthread_create` is called, by switching the running
/// Unicorn context to the thread's entry function.  When the thread returns
/// (to `THREAD_SENTINEL_ADDR`) the saved main-thread context is restored and
/// `pthread_create` returns 0.  This stack of continuations supports nested
/// `pthread_create` calls.
///
/// Because the single Unicorn instance runs everything sequentially there is
/// no real concurrency; mutexes, condition variables, and rwlocks are all
/// no-ops.
use std::collections::{HashMap, HashSet};

/// Saved CPU state for the thread that called `pthread_create`.
#[derive(Debug, Clone)]
pub struct ThreadContinuation {
    /// Address to return to when the created thread finishes.
    pub ret_addr: u32,
    /// TID of the thread that is about to run.
    pub tid: u32,
    // Callee-saved registers at the pthread_create call site.
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    /// ESP at the pthread_create call (before the trampoline popped anything).
    pub esp: u32,
}

pub struct ThreadTable {
    next_tid: u32,
    /// Finished threads and their return values.
    results: HashMap<u32, u32>,
    /// Stack of saved contexts waiting for created threads to finish.
    pub continuations: Vec<ThreadContinuation>,
    /// Thread-local storage — simplified: single global key→value table.
    tls: HashMap<u32, u32>,
    next_key: u32,
    /// Addresses of `pthread_once_t` objects that have already been run.
    once_done: HashSet<u32>,
    /// Guest signal handlers registered via sigaction: signal → handler_addr.
    pub signal_handlers: HashMap<u32, u32>,
}

impl ThreadTable {
    pub fn new() -> Self {
        ThreadTable {
            next_tid: 2, // 1 = main thread
            results: HashMap::new(),
            continuations: Vec::new(),
            tls: HashMap::new(),
            next_key: 1,
            once_done: HashSet::new(),
            signal_handlers: HashMap::new(),
        }
    }

    pub fn alloc_tid(&mut self) -> u32 {
        let tid = self.next_tid;
        self.next_tid += 1;
        tid
    }

    pub fn store_result(&mut self, tid: u32, retval: u32) {
        self.results.insert(tid, retval);
    }

    pub fn get_result(&self, tid: u32) -> Option<u32> {
        self.results.get(&tid).copied()
    }

    /// Allocate a new TLS key.
    pub fn create_key(&mut self) -> u32 {
        let k = self.next_key;
        self.next_key += 1;
        k
    }

    pub fn set_tls(&mut self, key: u32, value: u32) {
        self.tls.insert(key, value);
    }

    pub fn get_tls(&self, key: u32) -> u32 {
        self.tls.get(&key).copied().unwrap_or(0)
    }

    /// Returns `true` if this is the first call for `once_addr` (run the fn).
    pub fn once_check_and_set(&mut self, once_addr: u32) -> bool {
        self.once_done.insert(once_addr) // returns true if newly inserted
    }
}

impl Default for ThreadTable {
    fn default() -> Self {
        Self::new()
    }
}
