use std::{
    error::Error,
    fs::OpenOptions,
    io::{self, Seek},
    os::unix::fs::{FileExt, MetadataExt},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{inode, util};

const BLOCK_SIZE: u64 = 4096;
const INODE_SIZE: u64 = 256;
const ROOT_INODE: u64 = 1;
const MAGIC: &[u8; 8] = b"PELICAN5";

struct SuperBlock {
    block_size: u64,
    total_blocks: u64,
    inode_count: u64,
    inode_bitmap_at: u64,
    block_bitmap_at: u64,
    inode_table_at: u64,
    data_at: u64,
    free_blocks: u64,
    free_inodes: u64,
    root_inode: u64,
}

impl SuperBlock {
    fn to_bytes(&self) -> [u8; 4096] {
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
}

//max 128mb supported for now
pub fn mkfs(image: &str) -> Result<(), Box<dyn Error>> {
    let mut file = OpenOptions::new().read(true).write(true).open(image)?;
    let size = file.seek(io::SeekFrom::End(0))?;
    let total_blocks = size / BLOCK_SIZE;

    if total_blocks < 68 {
        return Err(format!("image too small: {total_blocks} blocks, need at least 68").into());
    }
    if total_blocks > 32768 {
        return Err(format!("image too big: {total_blocks} blocks, max 32768 (128 MB)").into());
    }

    let total_inode: u64 = 1024;
    let inode_block_count = (INODE_SIZE * total_inode) / BLOCK_SIZE;

    let sb = SuperBlock {
        block_size: BLOCK_SIZE,
        total_blocks: total_blocks,
        inode_count: total_inode,
        block_bitmap_at: 1,
        inode_bitmap_at: 2,
        inode_table_at: 3,
        data_at: 3 + inode_block_count,
        free_blocks: total_blocks - (3 + inode_block_count),
        free_inodes: (total_inode - 2),
        root_inode: ROOT_INODE,
    };

    let mut block_bitmap = [0u8; BLOCK_SIZE as usize];

    for n in 0..sb.data_at {
        block_bitmap[(n / 8) as usize] |= 1 << (n % 8);
    }

    for n in total_blocks..(BLOCK_SIZE * 8) {
        block_bitmap[(n / 8) as usize] |= 1 << (n % 8);
    }

    let mut inode_bitmap = [0u8; BLOCK_SIZE as usize];
    inode_bitmap[0] = 0b11;

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let meta = file.metadata()?;

    let root = inode::INode {
        kind: 0o040755,
        uid: meta.uid() as u64,
        gid: meta.gid() as u64,
        size: 0,
        atime: now,
        mtime: now,
        ctime: now,
        d_pointers: [0; 12],
        i_pointer: 0,
        revision_head: 0,
    };

    file.write_all_at(&block_bitmap, sb.block_bitmap_at * BLOCK_SIZE)?;
    file.write_all_at(&inode_bitmap, sb.inode_bitmap_at * BLOCK_SIZE)?;
    file.write_all_at(
        &root.to_bytes(),
        sb.inode_table_at * BLOCK_SIZE + ROOT_INODE * INODE_SIZE,
    )?;
    file.write_all_at(&sb.to_bytes(), 0)?;

    file.sync_all()?;
    Ok(())
}
