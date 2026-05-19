use core::ffi::c_void;
use std::cell::RefCell;
use std::collections::HashMap;

struct Inner {
    next: u32,
    to_ptr: HashMap<u32, *mut c_void>,
    to_handle: HashMap<usize, u32>,
    pending_applies: Vec<(u32, u32, u32)>, // (applier_addr, elem_value, context)
}

// SAFETY: The emulator is single-threaded; thread_local prevents any sharing.
unsafe impl Send for Inner {}

thread_local! {
    static TABLE: RefCell<Inner> = RefCell::new(Inner {
        next: 1,
        to_ptr: HashMap::new(),
        to_handle: HashMap::new(),
        pending_applies: Vec::new(),
    });
}

/// Intern a new 64-bit host pointer, returning a stable 32-bit handle.
/// Deduplicates: the same pointer always returns the same handle.
/// NULL → 0.
pub fn host_intern(ptr: *const c_void) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    let key = ptr as usize;
    let existing = TABLE.with(|t| t.borrow().to_handle.get(&key).copied());
    if let Some(h) = existing {
        return h;
    }
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(&h) = t.to_handle.get(&key) {
            return h;
        }
        let h = t.next;
        t.next += 1;
        t.to_ptr.insert(h, ptr as *mut c_void);
        t.to_handle.insert(key, h);
        h
    })
}

/// Resolve a value returned from a host function back to a 32-bit guest value.
///
/// - Already in the handle table → return its handle.
/// - Fits in 32 bits (was a raw i386 value stored verbatim by the host) → return as-is.
/// - New 64-bit host pointer not yet tracked → intern and return a new handle.
pub fn host_result(ptr: *const c_void) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    let raw = ptr as usize;
    let existing = TABLE.with(|t| t.borrow().to_handle.get(&raw).copied());
    if let Some(h) = existing {
        return h;
    }
    if raw <= 0xFFFF_FFFF {
        // Raw i386 value stored in a host container; return it directly.
        return raw as u32;
    }
    host_intern(ptr)
}

/// Resolve a 32-bit guest value to a host const pointer.
///
/// - 0 → NULL.
/// - Known handle → real host pointer.
/// - Unknown (raw i386 address) → zero-extended to 64 bits.
pub fn host_arg(v: u32) -> *const c_void {
    if v == 0 {
        return std::ptr::null();
    }
    TABLE.with(|t| {
        t.borrow()
            .to_ptr
            .get(&v)
            .copied()
            .map(|p| p as *const c_void)
            .unwrap_or(v as usize as *const c_void)
    })
}

/// Mutable variant of [`host_arg`].
pub fn host_arg_mut(v: u32) -> *mut c_void {
    host_arg(v) as *mut c_void
}

/// Push a pending (applier_addr, element_value, context) onto the CF apply queue.
/// Items are pushed in reverse order of desired execution so that `pop()` gives
/// forward array order (elem[1], elem[2], …, elem[n-1]).
pub fn cf_apply_push(applier_addr: u32, elem: u32, context: u32) {
    TABLE.with(|t| t.borrow_mut().pending_applies.push((applier_addr, elem, context)));
}

/// Pop the next pending CF apply item, or `None` if the queue is empty.
pub fn cf_apply_pop() -> Option<(u32, u32, u32)> {
    TABLE.with(|t| t.borrow_mut().pending_applies.pop())
}
