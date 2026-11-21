use std::io;

use crate::util;

pub const BLOCK_SIZE: u64 = 4096;
pub const ROOT_INODE: u64 = 1;
pub const MAGIC: &[u8; 8] = b"PELICAN5";

pub struct SuperBlock {
    pub block_size: u64,
    pub total_blocks: u64,
    pub inode_count: u64,
    pub inode_bitmap_at: u64,
    pub block_bitmap_at: u64,
    pub inode_table_at: u64,
    pub data_at: u64,
    pub free_blocks: u64,
    pub free_inodes: u64,
    pub root_inode: u64,
}

impl SuperBlock {
    pub fn to_bytes(&self) -> [u8; 4096] {
        let mut buf = [0u8; 4096];
        buf[0..8].copy_from_slice(MAGIC);
        util::put_u64(&mut buf, 8, self.block_size);
        util::put_u64(&mut buf, 16, self.total_blocks);
        util::put_u64(&mut buf, 24, self.inode_count);
        util::put_u64(&mut buf, 32, self.inode_bitmap_at);
        util::put_u64(&mut buf, 40, self.block_bitmap_at);
        util::put_u64(&mut buf, 48, self.inode_table_at);
        util::put_u64(&mut buf, 56, self.data_at);
        util::put_u64(&mut buf, 64, self.free_blocks);
        util::put_u64(&mut buf, 72, self.free_inodes);
        util::put_u64(&mut buf, 80, self.root_inode);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> io::Result<SuperBlock> {
        if &buf[0..8] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a pelican image (bad magic)",
            ));
        }

        Ok(SuperBlock {
            block_size: util::get_u64(buf, 8),
            total_blocks: util::get_u64(buf, 16),
            inode_count: util::get_u64(buf, 24),
            inode_bitmap_at: util::get_u64(buf, 32),
            block_bitmap_at: util::get_u64(buf, 40),
            inode_table_at: util::get_u64(buf, 48),
            data_at: util::get_u64(buf, 56),
            free_blocks: util::get_u64(buf, 64),
            free_inodes: util::get_u64(buf, 72),
            root_inode: util::get_u64(buf, 80),
        })
    }
}
