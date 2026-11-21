use std::{
    fs::OpenOptions,
    io::{self, Seek},
    os::unix::fs::{FileExt, MetadataExt},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::inode::{INODE_SIZE, INode};
use crate::superblock::{BLOCK_SIZE, ROOT_INODE, SuperBlock};

//max 128mb supported for now
pub fn mkfs(image: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(image)?;
    let size = file.seek(io::SeekFrom::End(0))?;
    let total_blocks = size / BLOCK_SIZE;

    if total_blocks < 68 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("image too small: {total_blocks} blocks, need at least 68"),
        ));
    }
    if total_blocks > 32768 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("image too big: {total_blocks} blocks, max 32768 (128 MB)"),
        ));
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

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let meta = file.metadata()?;

    let root = INode {
        kind: 0o040755,
        uid: meta.uid() as u64,
        gid: meta.gid() as u64,
        size: 0,
        atime: now,
        mtime: now,
        ctime: now,
        nlink: 2,
        dpointers: [0; 12],
        ipointer: 0,
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
