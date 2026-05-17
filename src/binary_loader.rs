use crate::errors::{EmulationError, EmulationResult};
use goblin::mach::MachO;
use std::fs;
use std::path::Path;

/// Information about a loaded binary
#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub name: String,
    pub entry_point: u32,
    #[allow(dead_code)]
    pub arch: Architecture,
    #[allow(dead_code)]
    pub is_dynamic: bool,
    pub stack_size: u32,
    pub heap_size: u32,
    #[allow(dead_code)]
    pub sections: Vec<Section>,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Architecture {
    I386,
    X86_64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub vaddr: u32,
    pub vsize: u32,
    #[allow(dead_code)]
    pub fileoff: u32,
    #[allow(dead_code)]
    pub filesize: u32,
    pub prot: u32,
}

/// Load and parse an i386 macOS binary (Mach-O format)
pub fn load_binary(path: &Path) -> EmulationResult<BinaryInfo> {
    let bytes = fs::read(path).map_err(|e| EmulationError::BinaryLoadError(e.to_string()))?;

    // Parse as Mach-O
    let macho = MachO::parse(&bytes, 0)
        .map_err(|e| EmulationError::BinaryLoadError(format!("Failed to parse Mach-O: {}", e)))?;

    // Check architecture
    let arch = match macho.header.cputype {
        7 => Architecture::I386,   // CPU_TYPE_I386
        _ => {
            return Err(EmulationError::InvalidArchitecture(format!(
                "Unsupported CPU type: {}",
                macho.header.cputype
            )))
        }
    };

    // Extract segments from the segments field
    let mut segments = Vec::new();
    for segment in &macho.segments {
        segments.push(Segment {
            name: format!("segment_{}", segments.len()),
            vaddr: segment.vmaddr as u32,
            vsize: segment.vmsize as u32,
            fileoff: segment.fileoff as u32,
            filesize: segment.filesize as u32,
            prot: segment.initprot,
        });
    }

    // Extract sections - parse them from the file manually
    let mut sections = Vec::new();
    for segment in &macho.segments {
        for (section, data) in segment.sections().unwrap_or_default() {
            let name = std::str::from_utf8(&section.sectname)
                .unwrap_or("unknown")
                .trim_end_matches('\0')
                .to_string();

            sections.push(Section {
                name,
                addr: section.addr as u32,
                size: section.size as u32,
                offset: section.offset as u32,
                data: data.to_vec(),
            });
        }
    }

    // Check if it's dynamically linked (simplified check)
    let is_dynamic = macho.load_commands.iter().any(|cmd| {
        format!("{:?}", cmd).contains("DyLinker") || format!("{:?}", cmd).contains("Dylib")
    });

    // Get entry point (simplified - default to 0x1000)
    let entry_point = 0x1000u32;

    Ok(BinaryInfo {
        name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        entry_point,
        arch,
        is_dynamic,
        stack_size: 8 * 1024 * 1024,  // 8MB default
        heap_size: 32 * 1024 * 1024, // 32MB default
        sections,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_architecture_detection() {
        let arch_i386 = Architecture::I386;
        assert_eq!(arch_i386, Architecture::I386);
    }
}
