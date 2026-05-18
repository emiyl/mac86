use std::collections::HashMap;

use crate::dyld::DyldBindings;

use super::symbols::{known_data_symbol, known_symbol, LibSym};

pub const TRAMPOLINE_BASE: u32 = 0x5000_0000;
pub const THREAD_SENTINEL_ADDR: u32 = TRAMPOLINE_BASE + 4;
pub const SIGNAL_RETURN_ADDR: u32 = TRAMPOLINE_BASE + 8;
pub const OPTIND_STORAGE_ADDR: u32 = 0x5000_F000;
pub const OPTARG_STORAGE_ADDR: u32 = 0x5000_F004;
pub const ERRNO_STORAGE_ADDR: u32 = 0x5000_F008;
pub const STACK_CHK_GUARD_ADDR: u32 = 0x5003_0000;
pub const DEFAULT_RUNE_LOCALE_ADDR: u32 = 0x5003_1000;

// Fake FILE* identity values for stdin/stdout/stderr.
pub const STDIN_FILE_PTR: u32 = 0x5001_0000;
pub const STDOUT_FILE_PTR: u32 = 0x5001_0004;
pub const STDERR_FILE_PTR: u32 = 0x5001_0008;

// Storage locations for __stdinp/__stdoutp/__stderrp globals.
// The binary's __nl_symbol_ptr slot is patched to point here; reading the
// slot gives the fake FILE* value above.
pub const STDINP_STORAGE: u32 = 0x5001_F000;
pub const STDOUTP_STORAGE: u32 = 0x5001_F004;
pub const STDERRP_STORAGE: u32 = 0x5001_F008;

// fdopen-created FILE* range: FILE* = FDOPEN_FILE_BASE + fd * 4 (fd 0..255).
pub const FDOPEN_FILE_BASE: u32 = 0x5002_0000;

pub struct Trampoline {
    pub dispatch: HashMap<u32, LibSym>,
    name_to_addr: HashMap<String, u32>,
    pub slot_count: u32,
}

impl Trampoline {
    pub fn build(bindings: &DyldBindings) -> Self {
        let mut dispatch: HashMap<u32, LibSym> = HashMap::new();
        let mut name_to_addr: HashMap<String, u32> = HashMap::new();
        let mut sym_to_addr: HashMap<LibSym, u32> = HashMap::new();

        dispatch.insert(TRAMPOLINE_BASE, LibSym::Exit);
        sym_to_addr.insert(LibSym::Exit, TRAMPOLINE_BASE);
        dispatch.insert(THREAD_SENTINEL_ADDR, LibSym::ThreadSentinel);
        sym_to_addr.insert(LibSym::ThreadSentinel, THREAD_SENTINEL_ADDR);
        dispatch.insert(SIGNAL_RETURN_ADDR, LibSym::SignalReturn);
        sym_to_addr.insert(LibSym::SignalReturn, SIGNAL_RETURN_ADDR);

        let mut slot = 3u32;
        for imp in &bindings.imports {
            if let Some(data_addr) = known_data_symbol(&imp.name) {
                name_to_addr.insert(imp.name.clone(), data_addr);
                continue;
            }

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
        for (name, &addr) in name_to_addr.iter() {
            log::debug!("trampoline mapping: {:?} -> 0x{:x}", name, addr);
        }
        Trampoline {
            dispatch,
            name_to_addr,
            slot_count: slot,
        }
    }

    pub fn exit_addr(&self) -> u32 {
        TRAMPOLINE_BASE
    }

    pub fn addr_for_binding(&self, name: &str) -> Option<u32> {
        self.name_to_addr.get(name).copied()
    }

    pub fn region_end(&self) -> u32 {
        TRAMPOLINE_BASE + (self.slot_count + 1) * 4
    }

    /// Return a name→address map for dlsym lookups (leading `_` stripped).
    pub fn symbol_map(&self) -> HashMap<String, u32> {
        self.name_to_addr
            .iter()
            .map(|(k, &v)| (k.trim_start_matches('_').to_string(), v))
            .collect()
    }
}
