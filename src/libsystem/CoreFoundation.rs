use core::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CFRange {
    pub location: isize,
    pub length: isize,
}

pub type CFArrayRef = *const c_void;
pub type CFMutableArrayRef = *mut c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFNumberRef = *const c_void;

#[repr(C)]
pub struct CFArrayCallBacks {
    _opaque: [u8; 0],
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kCFTypeArrayCallBacks: CFArrayCallBacks;
    pub fn CFAbsoluteTimeGetCurrent() -> f64;
    pub fn CFArrayAppendArray(array: CFMutableArrayRef, newValues: CFArrayRef, range: CFRange);
    pub fn CFArrayAppendValue(array: CFMutableArrayRef, value: *const c_void);
    pub fn CFArrayContainsValue(array: CFArrayRef, range: CFRange, value: *const c_void) -> bool;
    pub fn CFArrayCreateCopy(allocator: CFAllocatorRef, array: CFArrayRef) -> CFArrayRef;
    pub fn CFArrayCreateMutable(
        allocator: CFAllocatorRef,
        capacity: isize,
        callbacks: *const c_void,
    ) -> CFMutableArrayRef;
    pub fn CFArrayCreateMutableCopy(
        allocator: CFAllocatorRef,
        capacity: isize,
        array: CFArrayRef,
    ) -> CFMutableArrayRef;
    pub fn CFArrayGetCount(array: CFArrayRef) -> isize;
    pub fn CFArrayGetTypeID() -> u64;
    pub fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> *const c_void;
    pub fn CFArrayInsertValueAtIndex(array: CFMutableArrayRef, index: isize, value: *const c_void);
    pub fn CFArrayRemoveAllValues(array: CFMutableArrayRef);
    pub fn CFArrayRemoveValueAtIndex(array: CFMutableArrayRef, index: isize);
    pub fn CFNumberCreate(allocator: CFAllocatorRef, theType: isize, valuePtr: *const c_void) -> CFNumberRef;
    pub fn CFNumberGetTypeID() -> u64;
    pub fn CFNumberGetValue(number: CFNumberRef, theType: isize, valuePtr: *mut c_void) -> bool;
}
