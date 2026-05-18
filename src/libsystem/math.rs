use unicorn_engine::{RegisterX86, Unicorn};

use super::mem::read_u32;

pub(super) fn read_f64(emu: &Unicorn<'_, ()>, addr: u32) -> f64 {
    let lo = read_u32(emu, addr) as u64;
    let hi = read_u32(emu, addr + 4) as u64;
    f64::from_bits(lo | (hi << 32))
}

pub(super) fn read_f32(emu: &Unicorn<'_, ()>, addr: u32) -> f32 {
    f32::from_bits(read_u32(emu, addr))
}

/// Write an f64 result to ST(0) using the 80-bit x87 extended-precision format.
pub(super) fn write_f64_st0(emu: &mut Unicorn<'_, ()>, value: f64) {
    let bytes = f64_to_x87(value);
    let low8 = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or_default());
    let _ = emu.reg_write(RegisterX86::ST0, low8);
}

/// Convert IEEE 754 double to 80-bit x87 extended-precision (10 bytes, little-endian).
fn f64_to_x87(d: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if d == 0.0 {
        return out;
    }
    let bits = d.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    let exp64 = ((bits >> 52) & 0x7FF) as i32;
    let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
    if exp64 == 0x7FF {
        let exp80 = 0x7FFFu16 | sign;
        let sig: u64 = if mantissa == 0 {
            0x8000_0000_0000_0000
        } else {
            0xC000_0000_0000_0000
        };
        out[0..8].copy_from_slice(&sig.to_le_bytes());
        out[8..10].copy_from_slice(&exp80.to_le_bytes());
    } else {
        let exp80 = ((exp64 - 1023 + 16383) as u16) | sign;
        let sig = 0x8000_0000_0000_0000u64 | (mantissa << 11);
        out[0..8].copy_from_slice(&sig.to_le_bytes());
        out[8..10].copy_from_slice(&exp80.to_le_bytes());
    }
    out
}
