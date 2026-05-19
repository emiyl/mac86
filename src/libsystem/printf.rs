use unicorn_engine::Unicorn;

use crate::filesystem::VirtualFileSystem;

use super::mem::{read_cstr, read_u32};

pub(super) fn fmt_printf(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    fd: u32,
    fmt_ptr: u32,
    vararg_esp: u32,
) -> usize {
    let (text, _) = format_str(emu, fmt_ptr, vararg_esp);
    let n = text.len();
    let _ = fs.write_bytes(fd, text.as_bytes());
    n
}

pub(super) fn fmt_printf_str(
    emu: &mut Unicorn<'_, ()>,
    fs: &mut VirtualFileSystem,
    fd: u32,
    fmt: &str,
    vararg_esp: u32,
) -> usize {
    let mut vae = vararg_esp;
    let text = do_format(emu, fmt, &mut vae);
    let n = text.len();
    let _ = fs.write_bytes(fd, text.as_bytes());
    n
}

pub(super) fn format_str(
    emu: &mut Unicorn<'_, ()>,
    fmt_ptr: u32,
    vararg_esp: u32,
) -> (String, usize) {
    let fmt = read_cstr(emu, fmt_ptr);
    let mut vae = vararg_esp;
    let text = do_format(emu, &fmt, &mut vae);
    let n = text.len();
    (text, n)
}

fn next32(emu: &Unicorn<'_, ()>, esp: &mut u32) -> u32 {
    let v = read_u32(emu, *esp);
    *esp += 4;
    v
}

fn next64(emu: &Unicorn<'_, ()>, esp: &mut u32) -> u64 {
    // i386 cdecl pushes the low word first, so it lands at the higher address.
    // The high word is pushed last and sits at the lower address ([esp]).
    let hi = read_u32(emu, *esp) as u64;
    let lo = read_u32(emu, *esp + 4) as u64;
    *esp += 8;
    (hi << 32) | lo
}

pub(super) fn do_format(emu: &Unicorn<'_, ()>, fmt: &str, vararg_esp: &mut u32) -> String {
    let mut out = String::with_capacity(fmt.len() + 16);
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            break;
        }

        // ── Flags ─────────────────────────────────────────────────────────
        let mut left_align = false;
        let mut zero_pad = false;
        loop {
            match bytes.get(i) {
                Some(b'-') => { left_align = true; i += 1; }
                Some(b'0') => { zero_pad = true;   i += 1; }
                Some(b'+') | Some(b' ') | Some(b'#') => { i += 1; }
                _ => break,
            }
        }

        // ── Width  (literal digits, or * = next arg) ───────────────────────
        let mut width = 0usize;
        if bytes.get(i) == Some(&b'*') {
            let w = next32(emu, vararg_esp) as i32;
            if w < 0 { left_align = true; width = (-w) as usize; }
            else      { width = w as usize; }
            i += 1;
        } else {
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                width = width * 10 + (bytes[i] - b'0') as usize;
                i += 1;
            }
        }

        // ── Precision  (.digits or .*)  ────────────────────────────────────
        // We don't use precision for formatting yet; just consume it.
        if bytes.get(i) == Some(&b'.') {
            i += 1;
            if bytes.get(i) == Some(&b'*') {
                next32(emu, vararg_esp);
                i += 1;
            } else {
                while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
            }
        }

        // ── Length modifiers ───────────────────────────────────────────────
        // On i386: long = 4 bytes, long long / intmax_t / quad = 8 bytes.
        let mut wide = false; // true → read 8 bytes from stack
        let mut l_count = 0u8;
        loop {
            match bytes.get(i) {
                Some(b'l') => { l_count += 1; if l_count >= 2 { wide = true; } i += 1; }
                Some(b'q') => { wide = true; i += 1; } // BSD %qu / %qd
                Some(b'j') => { wide = true; i += 1; } // intmax_t (64-bit on i386)
                Some(b'h') | Some(b'z') | Some(b't') => { i += 1; }
                _ => break,
            }
        }

        if i >= bytes.len() { break; }
        let spec = bytes[i];
        i += 1;

        if spec == b'%' { out.push('%'); continue; }
        if spec == b'n' { next32(emu, vararg_esp); continue; }

        let right    = !left_align;
        let pad_char = if zero_pad { '0' } else { ' ' };

        let frag: String = match spec {
            b'd' | b'i' => {
                let s = if wide {
                    format!("{}", next64(emu, vararg_esp) as i64)
                } else {
                    format!("{}", next32(emu, vararg_esp) as i32)
                };
                padf(s, width, pad_char, right)
            }
            b'u' => {
                let s = if wide {
                    format!("{}", next64(emu, vararg_esp))
                } else {
                    format!("{}", next32(emu, vararg_esp))
                };
                padf(s, width, pad_char, right)
            }
            b'x' => {
                let s = if wide {
                    format!("{:x}", next64(emu, vararg_esp))
                } else {
                    format!("{:x}", next32(emu, vararg_esp))
                };
                padf(s, width, pad_char, right)
            }
            b'X' => {
                let s = if wide {
                    format!("{:X}", next64(emu, vararg_esp))
                } else {
                    format!("{:X}", next32(emu, vararg_esp))
                };
                padf(s, width, pad_char, right)
            }
            b'o' => {
                let s = if wide {
                    format!("{:o}", next64(emu, vararg_esp))
                } else {
                    format!("{:o}", next32(emu, vararg_esp))
                };
                padf(s, width, pad_char, right)
            }
            b'p' => format!("0x{:x}", next32(emu, vararg_esp)),
            b's' => {
                let ptr = next32(emu, vararg_esp);
                let s = if ptr == 0 { "(null)".to_string() } else { read_cstr(emu, ptr) };
                padf(s, width, ' ', right)
            }
            b'c' => (next32(emu, vararg_esp) as u8 as char).to_string(),
            b'f' | b'e' | b'E' | b'g' | b'G' => {
                // double is always 8 bytes on i386 stack (promoted from float)
                let bits = next64(emu, vararg_esp);
                format!("{}", f64::from_bits(bits))
            }
            _ => format!("%{}", spec as char),
        };
        out.push_str(&frag);
    }
    out
}

fn padf(s: String, width: usize, pad: char, right: bool) -> String {
    if width <= s.len() {
        return s;
    }
    let padding: String = std::iter::repeat_n(pad, width - s.len()).collect();
    if right {
        format!("{}{}", padding, s)
    } else {
        format!("{}{}", s, padding)
    }
}
