use crate::dyld::DyldBindings;
use crate::errors::{EmulationError, EmulationResult};
use goblin::mach::load_command::CommandVariant;
use goblin::mach::MachO;
use std::fs;
use std::path::Path;

/// Information about a loaded i386 Mach-O executable.
#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub name: String,
    /// Guest virtual address of the process entry point.
    pub entry_point: u32,
    /// `true` when the entry point was derived from `LC_MAIN` and points
    /// directly at `main()` — the process setup must push a fake return address.
    pub entry_is_main: bool,
    pub arch: Architecture,
    /// `true` when the binary has dynamic library dependencies.
    pub is_dynamic: bool,
    pub sections: Vec<Section>,
    pub segments: Vec<Segment>,
    /// Raw file bytes — kept alive for in-place segment loading.
    pub raw: Vec<u8>,
    /// Import bindings parsed from LC_DYLD_INFO (or classic LC_DYSYMTAB).
    pub dyld_bindings: Option<DyldBindings>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Architecture {
    I386,
    X86_64,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub addr: u32,
    pub size: u32,
    pub offset: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub name: String,
    pub vaddr: u32,
    pub vsize: u32,
    pub fileoff: u32,
    pub filesize: u32,
}

/// Load and parse an i386 macOS Mach-O executable.
pub fn load_binary(path: &Path) -> EmulationResult<BinaryInfo> {
    let bytes = fs::read(path).map_err(|e| EmulationError::BinaryLoadError(e.to_string()))?;

    let macho = MachO::parse(&bytes, 0)
        .map_err(|e| EmulationError::BinaryLoadError(format!("Mach-O parse: {}", e)))?;

    // Only i386 (CPU_TYPE_I386 = 7)
    if macho.header.cputype != 7 {
        return Err(EmulationError::InvalidArchitecture(format!(
            "Unsupported CPU type {} (expected i386=7)",
            macho.header.cputype
        )));
    }

    // Only MH_EXECUTE (0x2)
    if macho.header.filetype != 0x2 {
        return Err(EmulationError::BinaryLoadError(format!(
            "Unsupported Mach-O file type {} (expected MH_EXECUTE=2)",
            macho.header.filetype
        )));
    }

    // ── segments ─────────────────────────────────────────────────────────────
    let segments: Vec<Segment> = macho
        .segments
        .iter()
        .map(|s| Segment {
            name: s.name().unwrap_or("?").trim_end_matches('\0').to_string(),
            vaddr: s.vmaddr as u32,
            vsize: s.vmsize as u32,
            fileoff: s.fileoff as u32,
            filesize: s.filesize as u32,
        })
        .collect();

    // ── sections ─────────────────────────────────────────────────────────────
    let mut sections: Vec<Section> = Vec::new();
    for segment in &macho.segments {
        for (sec, data) in segment.sections().unwrap_or_default() {
            let name = std::str::from_utf8(&sec.sectname)
                .unwrap_or("?")
                .trim_end_matches('\0')
                .to_string();
            sections.push(Section {
                name,
                addr: sec.addr as u32,
                size: sec.size as u32,
                offset: sec.offset,
                data: data.to_vec(),
            });
        }
    }

    // ── dynamic linking ───────────────────────────────────────────────────────
    let is_dynamic = macho.load_commands.iter().any(|cmd| {
        matches!(
            cmd.command,
            CommandVariant::LoadDylib(_)
                | CommandVariant::LoadWeakDylib(_)
                | CommandVariant::DyldInfoOnly(_)
                | CommandVariant::DyldInfo(_)
        )
    });

    let dyld_bindings = if is_dynamic {
        let b = DyldBindings::parse(&macho, &bytes);
        if !b.imports.is_empty() { Some(b) } else { None }
    } else {
        None
    };

    // ── entry point ───────────────────────────────────────────────────────────
    // Priority: LC_MAIN > LC_UNIXTHREAD EIP > __text section > first segment.
    let text_base: u32 = segments
        .iter()
        .find(|s| s.name.contains("TEXT"))
        .map(|s| s.vaddr)
        .unwrap_or(0x1000);

    let mut entry_point: Option<u32> = None;
    let mut entry_is_main = false;

    for cmd in &macho.load_commands {
        match cmd.command {
            CommandVariant::Main(ref ep) => {
                entry_point = Some(text_base + ep.entryoff as u32);
                entry_is_main = true;
                break;
            }
            _ => {
                // Parse LC_UNIXTHREAD thread state to extract EIP (index 10).
                let dump = format!("{:?}", cmd);
                if let Some(start) = dump.find("thread_state: [") {
                    let rest = &dump[start + "thread_state: [".len()..];
                    if let Some(end) = rest.find(']') {
                        let parts: Vec<&str> =
                            rest[..end].split(',').map(str::trim).collect();
                        if parts.len() > 10 {
                            if let Ok(eip) = parts[10].parse::<u32>() {
                                entry_point.get_or_insert(eip);
                            }
                        }
                    }
                }
            }
        }
    }

    let entry_point = entry_point
        .or_else(|| {
            sections
                .iter()
                .find(|s| s.name.to_lowercase().contains("text"))
                .map(|s| s.addr)
        })
        .or_else(|| segments.first().map(|s| s.vaddr))
        .unwrap_or(0x1000);

    Ok(BinaryInfo {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        entry_point,
        entry_is_main,
        arch: Architecture::I386,
        is_dynamic,
        sections,
        segments,
        raw: bytes,
        dyld_bindings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_architecture_variants() {
        assert_ne!(Architecture::I386, Architecture::X86_64);
    }
}
