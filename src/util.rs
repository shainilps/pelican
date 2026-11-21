pub fn put_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

pub fn get_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

pub fn get_u64_array<const N: usize>(buf: &[u8], off: usize) -> [u64; N] {
    let mut out = [0u64; N];
    for i in 0..N {
        out[i] = get_u64(buf, off + i * 8);
    }
    out
}

pub fn put_u64_array(buf: &mut [u8], off: usize, v: &[u64]) {
    for (i, &x) in v.iter().enumerate() {
        put_u64(buf, off + i * 8, x);
    }
}
