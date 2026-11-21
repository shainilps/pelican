use std::{
    io,
    time::{Duration, UNIX_EPOCH},
};

use fuser::{Errno, FileAttr, FileType, INodeNo};

use crate::{inode::INode, superblock::BLOCK_SIZE};

// fuser error code -> io::Error, for helpers that return io::Result
pub fn errno(e: Errno) -> io::Error {
    io::Error::from_raw_os_error(e.code())
}

// on-disk inode -> the attributes FUSE reports to the kernel
pub fn to_attr(ino: u64, inode: &INode) -> FileAttr {
    let kind = if inode.kind & 0o170000 == 0o040000 {
        FileType::Directory
    } else {
        FileType::RegularFile
    };
    let time = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);

    FileAttr {
        ino: INodeNo(ino),
        size: inode.size,
        blocks: (inode.size + 511) / 512,
        atime: time(inode.atime),
        mtime: time(inode.mtime),
        ctime: time(inode.ctime),
        crtime: time(inode.ctime),
        kind,
        perm: (inode.kind & 0o7777) as u16,
        nlink: inode.nlink as u32,
        uid: inode.uid as u32,
        gid: inode.gid as u32,
        rdev: 0,
        blksize: BLOCK_SIZE as u32,
        flags: 0,
    }
}
