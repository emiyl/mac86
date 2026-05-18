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
        let zero_pad = bytes[i] == b'0';
        if zero_pad {
            i += 1;
        }
        let mut width = 0usize;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            width = width * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }
        while i < bytes.len() && matches!(bytes[i], b'l' | b'h' | b'z' | b'j' | b't') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let spec = bytes[i];
        i += 1;
        if spec == b'%' {
            out.push('%');
            continue;
        }
        if spec == b'n' {
            continue;
        }
        let arg = read_u32(emu, *vararg_esp);
        *vararg_esp += 4;
        let frag = match spec {
            b'd' | b'i' => padf(
                format!("{}", arg as i32),
                width,
                if zero_pad { '0' } else { ' ' },
                true,
            ),
            b'u' => padf(
                format!("{}", arg),
                width,
                if zero_pad { '0' } else { ' ' },
                true,
            ),
            b'x' => padf(
                format!("{:x}", arg),
                width,
                if zero_pad { '0' } else { ' ' },
                true,
            ),
            b'X' => padf(
                format!("{:X}", arg),
                width,
                if zero_pad { '0' } else { ' ' },
                true,
            ),
            b'o' => padf(
                format!("{:o}", arg),
                width,
                if zero_pad { '0' } else { ' ' },
                true,
            ),
            b'p' => format!("0x{:x}", arg),
            b's' => {
                let s = if arg == 0 {
                    "(null)".to_string()
                } else {
                    read_cstr(emu, arg)
                };
                padf(s, width, ' ', false)
            }
            b'c' => (arg as u8 as char).to_string(),
            _ => {
                *vararg_esp -= 4;
                format!("%{}", spec as char)
            }
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
