/// Mach-O dynamic-linker binding parser.
///
/// Produces a list of `(symbol_name, ptr_slot_vaddr)` pairs — one for every
/// pointer slot that dyld would fill at load time.  Two strategies are tried:
///
/// 1. **LC_DYLD_INFO / LC_DYLD_INFO_ONLY** bind + lazy-bind opcode streams
///    (authoritative for binaries built with macOS 10.6+ toolchains).
/// 2. **Fallback**: LC_DYSYMTAB indirect-symbol table + `__nl_symbol_ptr` /
///    `__la_symbol_ptr` sections parsed directly from the raw binary because
///    goblin 0.8 does not expose `section_32.reserved1` in its `Section` type.
use goblin::mach::load_command::CommandVariant;
use goblin::mach::MachO;

#[derive(Debug, Clone)]
pub struct ImportBinding {
    /// Symbol name as it appears in the binary (e.g. `_write`).
    pub name: String,
    /// Guest virtual address of the pointer slot to fill.
    pub ptr_addr: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DyldBindings {
    pub imports: Vec<ImportBinding>,
}

struct SegInfo {
    vmaddr: u32,
}

impl DyldBindings {
    pub fn parse(macho: &MachO, raw: &[u8]) -> Self {
        let segments: Vec<SegInfo> = macho
            .segments
            .iter()
            .map(|s| SegInfo {
                vmaddr: s.vmaddr as u32,
            })
            .collect();

        let mut imports = Vec::new();

        // ── Strategy 1: LC_DYLD_INFO opcodes ─────────────────────────────
        for cmd in &macho.load_commands {
            let info = match cmd.command {
                CommandVariant::DyldInfoOnly(ref i) | CommandVariant::DyldInfo(ref i) => i,
                _ => continue,
            };

            if info.bind_size > 0 {
                let s = info.bind_off as usize;
                let e = s + info.bind_size as usize;
                if e <= raw.len() {
                    parse_bind_opcodes(&raw[s..e], &segments, &mut imports);
                }
            }
            if info.lazy_bind_size > 0 {
                let s = info.lazy_bind_off as usize;
                let e = s + info.lazy_bind_size as usize;
                if e <= raw.len() {
                    parse_bind_opcodes(&raw[s..e], &segments, &mut imports);
                }
            }

            dedup(&mut imports);
            return Self { imports };
        }

        // ── Strategy 2: classic LC_DYSYMTAB ──────────────────────────────
        parse_classic(macho, raw, &mut imports);
        dedup(&mut imports);
        Self { imports }
    }
}

// ── bind opcode interpreter ───────────────────────────────────────────────────

fn parse_bind_opcodes(
    opcodes: &[u8],
    segments: &[SegInfo],
    out: &mut Vec<ImportBinding>,
) {
    let mut pos = 0;
    let mut sym_name = String::new();
    let mut seg_idx: usize = 0;
    let mut seg_offset: u64 = 0;

    while pos < opcodes.len() {
        let byte = opcodes[pos];
        let imm = (byte & 0x0F) as usize;
        let opcode = byte & 0xF0;
        pos += 1;

        match opcode {
            0x00 => break, // DONE
            0x10 => {}     // SET_DYLIB_ORDINAL_IMM
            0x20 => {
                read_uleb128(opcodes, &mut pos);
            }
            0x30 => {} // SET_DYLIB_SPECIAL_IMM
            0x40 => {
                // SET_SYMBOL_TRAILING_FLAGS_IMM — null-terminated name follows
                let start = pos;
                while pos < opcodes.len() && opcodes[pos] != 0 {
                    pos += 1;
                }
                sym_name = String::from_utf8_lossy(&opcodes[start..pos]).into_owned();
                if pos < opcodes.len() {
                    pos += 1;
                }
            }
            0x50 => {} // SET_TYPE_IMM
            0x60 => {
                read_sleb128(opcodes, &mut pos);
            }
            0x70 => {
                seg_idx = imm;
                seg_offset = read_uleb128(opcodes, &mut pos);
            }
            0x80 => {
                let delta = read_uleb128(opcodes, &mut pos);
                seg_offset = seg_offset.wrapping_add(delta);
            }
            0x90 => {
                emit(segments, seg_idx, seg_offset, &sym_name, out);
                seg_offset = seg_offset.wrapping_add(4);
            }
            0xA0 => {
                emit(segments, seg_idx, seg_offset, &sym_name, out);
                let delta = read_uleb128(opcodes, &mut pos);
                seg_offset = seg_offset.wrapping_add(4 + delta);
            }
            0xB0 => {
                emit(segments, seg_idx, seg_offset, &sym_name, out);
                seg_offset = seg_offset.wrapping_add(4 + imm as u64 * 4);
            }
            0xC0 => {
                let count = read_uleb128(opcodes, &mut pos);
                let skip = read_uleb128(opcodes, &mut pos);
                for _ in 0..count {
                    emit(segments, seg_idx, seg_offset, &sym_name, out);
                    seg_offset = seg_offset.wrapping_add(4 + skip);
                }
            }
            _ => break,
        }
    }
}

fn emit(
    segments: &[SegInfo],
    seg_idx: usize,
    seg_offset: u64,
    sym_name: &str,
    out: &mut Vec<ImportBinding>,
) {
    if seg_idx < segments.len() {
        out.push(ImportBinding {
            name: sym_name.to_string(),
            ptr_addr: segments[seg_idx].vmaddr.wrapping_add(seg_offset as u32),
        });
    }
}

// ── classic LC_DYSYMTAB fallback ──────────────────────────────────────────────
//
// goblin 0.8 does not expose `section_32.reserved1` through its Section type,
// so we parse LC_SEGMENT section headers directly from the raw binary bytes.
// section_32 layout (68 bytes per header):
//   0   sectname[16]  16  segname[16]  32  addr(u32)  36  size(u32)
//  40   offset(u32)   44  align(u32)   48  reloff(u32) 52  nreloc(u32)
//  56   flags(u32)    60  reserved1(u32)  64  reserved2(u32)
// segment_command layout: cmd(4) cmdsize(4) segname(16) vmaddr(4) vmsize(4)
//   fileoff(4) filesize(4) maxprot(4) initprot(4) nsects(4) flags(4) = 56 bytes

fn parse_classic(macho: &MachO, raw: &[u8], out: &mut Vec<ImportBinding>) {
    // Find DysymtabCommand
    let dysymtab = macho.load_commands.iter().find_map(|cmd| {
        if let CommandVariant::Dysymtab(ref d) = cmd.command {
            Some(d)
        } else {
            None
        }
    });
    let dysymtab = match dysymtab {
        Some(d) => d,
        None => return,
    };

    let ind_off = dysymtab.indirectsymoff as usize;
    let ind_count = dysymtab.nindirectsyms as usize;
    if ind_off == 0 || ind_count == 0 || ind_off + ind_count * 4 > raw.len() {
        return;
    }

    let indirect: Vec<u32> = (0..ind_count)
        .map(|i| {
            let s = ind_off + i * 4;
            u32::from_le_bytes(raw[s..s + 4].try_into().unwrap_or_default())
        })
        .collect();

    // Collect symbol names. macho.symbols() returns SymbolIterator directly
    // (never Option); each item is Result<(&str, Nlist), Error>.
    let sym_names: Vec<String> = macho
        .symbols()
        .filter_map(|r| r.ok().map(|sym| sym.0.to_string()))
        .collect();

    const LC_SEGMENT: u32 = 0x1;
    const SEG_HDR: usize = 56;
    const SEC_SZ: usize = 68;
    const S_NON_LAZY: u32 = 6;
    const S_LAZY: u32 = 7;
    const INDIRECT_SYMBOL_LOCAL: u32 = 0x8000_0000;
    const INDIRECT_SYMBOL_ABS: u32 = 0x4000_0000;

    for cmd in &macho.load_commands {
        let cmd_off = cmd.offset;
        if cmd_off + SEG_HDR > raw.len() {
            continue;
        }
        let cmd_type =
            u32::from_le_bytes(raw[cmd_off..cmd_off + 4].try_into().unwrap_or_default());
        if cmd_type != LC_SEGMENT {
            continue;
        }

        let nsects = u32::from_le_bytes(
            raw[cmd_off + 48..cmd_off + 52]
                .try_into()
                .unwrap_or_default(),
        ) as usize;

        for i in 0..nsects {
            let sh = cmd_off + SEG_HDR + i * SEC_SZ;
            if sh + SEC_SZ > raw.len() {
                break;
            }

            let flags =
                u32::from_le_bytes(raw[sh + 56..sh + 60].try_into().unwrap_or_default());
            let sec_type = flags & 0xFF;
            if sec_type != S_NON_LAZY && sec_type != S_LAZY {
                continue;
            }

            let addr =
                u32::from_le_bytes(raw[sh + 32..sh + 36].try_into().unwrap_or_default());
            let size =
                u32::from_le_bytes(raw[sh + 36..sh + 40].try_into().unwrap_or_default());
            let reserved1 =
                u32::from_le_bytes(raw[sh + 60..sh + 64].try_into().unwrap_or_default())
                    as usize;

            let n_ptrs = size as usize / 4;
            for j in 0..n_ptrs {
                let ii = reserved1 + j;
                if ii >= indirect.len() {
                    break;
                }
                let sym_idx = indirect[ii];
                if sym_idx == INDIRECT_SYMBOL_LOCAL || sym_idx == INDIRECT_SYMBOL_ABS {
                    continue;
                }
                let si = sym_idx as usize;
                if si < sym_names.len() {
                    out.push(ImportBinding {
                        name: sym_names[si].clone(),
                        ptr_addr: addr + j as u32 * 4,
                    });
                }
            }
        }
    }
}

// ── ULEB128 / SLEB128 ────────────────────────────────────────────────────────

fn read_uleb128(data: &[u8], pos: &mut usize) -> u64 {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    while *pos < data.len() {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            break;
        }
    }
    result
}

fn read_sleb128(data: &[u8], pos: &mut usize) -> i64 {
    let mut result: i64 = 0;
    let mut shift = 0u32;
    let mut last = 0u8;
    while *pos < data.len() {
        last = data[*pos];
        *pos += 1;
        result |= ((last & 0x7F) as i64) << shift;
        shift += 7;
        if last & 0x80 == 0 {
            break;
        }
    }
    if shift < 64 && (last & 0x40) != 0 {
        result |= !0i64 << shift;
    }
    result
}

// ── rebase opcodes ────────────────────────────────────────────────────────────
//
// Rebase adjusts internal pointers by a slide value.  Since we always load at
// the binary's preferred address (slide = 0), this is a no-op at runtime.
// We parse the opcodes anyway so we can report unrecognised formats and collect
// the pointer addresses for future slide support.
#[derive(Debug, Clone)]
pub struct RebaseEntry {
    /// Guest virtual address of the pointer to rebase.
    pub ptr_addr: u32,
}

pub fn parse_rebases(macho: &MachO, raw: &[u8]) -> Vec<RebaseEntry> {
    let segments: Vec<SegInfo> = macho
        .segments
        .iter()
        .map(|s| SegInfo { vmaddr: s.vmaddr as u32 })
        .collect();

    let mut entries = Vec::new();

    for cmd in &macho.load_commands {
        let info = match cmd.command {
            CommandVariant::DyldInfoOnly(ref i) | CommandVariant::DyldInfo(ref i) => i,
            _ => continue,
        };

        if info.rebase_size == 0 {
            return entries;
        }
        let start = info.rebase_off as usize;
        let end   = start + info.rebase_size as usize;
        if end > raw.len() {
            return entries;
        }
        parse_rebase_opcodes(&raw[start..end], &segments, &mut entries);
        return entries;
    }
    entries
}

fn parse_rebase_opcodes(opcodes: &[u8], segments: &[SegInfo], out: &mut Vec<RebaseEntry>) {
    let mut pos = 0usize;
    let mut seg_idx: usize = 0;
    let mut seg_offset: u64 = 0;

    while pos < opcodes.len() {
        let byte = opcodes[pos];
        let imm  = (byte & 0x0F) as usize;
        let op   = byte & 0xF0;
        pos += 1;

        match op {
            0x00 => break,  // REBASE_OPCODE_DONE
            0x10 => {},     // SET_TYPE_IMM
            0x20 => {       // SET_SEGMENT_AND_OFFSET_ULEB
                seg_idx = imm;
                seg_offset = read_uleb128(opcodes, &mut pos);
            }
            0x30 => {       // ADD_ADDR_ULEB
                seg_offset = seg_offset.wrapping_add(read_uleb128(opcodes, &mut pos));
            }
            0x40 => {       // ADD_ADDR_IMM_SCALED
                seg_offset = seg_offset.wrapping_add((imm * 4) as u64);
            }
            0x50 => {       // DO_REBASE_IMM_TIMES
                for _ in 0..imm {
                    if seg_idx < segments.len() {
                        out.push(RebaseEntry {
                            ptr_addr: segments[seg_idx].vmaddr.wrapping_add(seg_offset as u32),
                        });
                    }
                    seg_offset = seg_offset.wrapping_add(4);
                }
            }
            0x60 => {       // DO_REBASE_ULEB_TIMES
                let count = read_uleb128(opcodes, &mut pos);
                for _ in 0..count {
                    if seg_idx < segments.len() {
                        out.push(RebaseEntry {
                            ptr_addr: segments[seg_idx].vmaddr.wrapping_add(seg_offset as u32),
                        });
                    }
                    seg_offset = seg_offset.wrapping_add(4);
                }
            }
            0x70 => {       // DO_REBASE_ADD_ADDR_ULEB
                if seg_idx < segments.len() {
                    out.push(RebaseEntry {
                        ptr_addr: segments[seg_idx].vmaddr.wrapping_add(seg_offset as u32),
                    });
                }
                seg_offset = seg_offset.wrapping_add(4 + read_uleb128(opcodes, &mut pos));
            }
            0x80 => {       // DO_REBASE_ULEB_TIMES_SKIPPING_ULEB
                let count = read_uleb128(opcodes, &mut pos);
                let skip  = read_uleb128(opcodes, &mut pos);
                for _ in 0..count {
                    if seg_idx < segments.len() {
                        out.push(RebaseEntry {
                            ptr_addr: segments[seg_idx].vmaddr.wrapping_add(seg_offset as u32),
                        });
                    }
                    seg_offset = seg_offset.wrapping_add(4 + skip);
                }
            }
            _ => break,
        }
    }
}

fn dedup(imports: &mut Vec<ImportBinding>) {
    imports.sort_by_key(|i| i.ptr_addr);
    imports.dedup_by_key(|i| i.ptr_addr);
}
