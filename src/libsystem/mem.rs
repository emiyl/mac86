use unicorn_engine::Unicorn;

pub(super) fn read_u32(emu: &Unicorn<'_, ()>, addr: u32) -> u32 {
    let mut buf = [0u8; 4];
    let _ = emu.mem_read(addr as u64, &mut buf);
    u32::from_le_bytes(buf)
}

pub(super) fn write_u32(emu: &mut Unicorn<'_, ()>, addr: u32, value: u32) {
    let _ = emu.mem_write(addr as u64, &value.to_le_bytes());
}

pub(super) fn read_bytes(emu: &Unicorn<'_, ()>, addr: u32, len: usize) -> Vec<u8> {
    if len == 0 {
        return vec![];
    }
    let mut buf = vec![0u8; len];
    let _ = emu.mem_read(addr as u64, &mut buf);
    buf
}

pub(super) fn read_cstr(emu: &Unicorn<'_, ()>, addr: u32) -> String {
    read_cstr_max(emu, addr, 65_536)
}

pub(super) fn read_cstr_max(emu: &Unicorn<'_, ()>, addr: u32, max: usize) -> String {
    let mut bytes = Vec::new();
    for i in 0..max {
        let mut b = [0u8; 1];
        if emu.mem_read(addr as u64 + i as u64, &mut b).is_err() {
            break;
        }
        if b[0] == 0 {
            break;
        }
        bytes.push(b[0]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
