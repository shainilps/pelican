pub fn put_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

pub fn put_u64_array(buf: &mut [u8], off: usize, v: &[u64]) {
    for (i, &x) in v.iter().enumerate() {
        put_u64(buf, off + i * 8, x);
    }
}
