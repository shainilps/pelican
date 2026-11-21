use std::{
    error::Error,
    ffi::{FromBytesUntilNulError, OsStr},
    fs::{File, OpenOptions},
    io,
    os::unix::{ffi::OsStrExt, fs::FileExt},
    sync::Mutex,
    time::{Duration, UNIX_EPOCH},
};

use fuser::{
    Config, FileAttr,
    FileType::{self, BlockDevice},
    Filesystem, INodeNo, MountOption,
};

use crate::{
    dirent::{DIRENT_SIZE, DirEntry},
    inode::{INODE_SIZE, INode},
    superblock::{BLOCK_SIZE, SuperBlock},
};

struct Pelican {
    file: File,
    sb: Mutex<SuperBlock>,
}

impl Pelican {
    fn read_inode(&self, ino: u64) -> io::Result<INode> {
        let mut buf = [0u8; INODE_SIZE as usize];
        let table = self.sb.lock().unwrap().inode_table_at;
        let off = table * BLOCK_SIZE + ino * INODE_SIZE;
        self.file.read_exact_at(&mut buf, off)?;
        Ok(INode::from_bytes(&buf))
    }

    fn write_inode(&self, ino: u64, inode: &INode) -> io::Result<()> {
        let table = self.sb.lock().unwrap().inode_table_at;
        let off = table * BLOCK_SIZE + ino * INODE_SIZE;
        self.file.write_all_at(&inode.to_bytes(), off)
    }

    fn write_superblock(&self, sb: &SuperBlock) -> io::Result<()> {
        self.file.write_all_at(&sb.to_bytes(), 0)
    }

    fn read_dir(&self, ino: u64) -> io::Result<Vec<DirEntry>> {
        let inode = self.read_inode(ino)?;
        let mut entries = Vec::new();
        let mut remaining = inode.size;

        for &block in inode.dpointers.iter() {
            if block == 0 || remaining == 0 {
                break;
            }

            let mut buf = [0u8; BLOCK_SIZE as usize];
            self.file.read_exact_at(&mut buf, block * BLOCK_SIZE)?;

            let used = remaining.min(BLOCK_SIZE);

            for i in 0..(used / DIRENT_SIZE) as usize {
                let off = i * DIRENT_SIZE as usize;
                let e = DirEntry::from_bytes(&buf[off..off + DIRENT_SIZE as usize]);
                if e.inode != 0 {
                    entries.push(e);
                }
            }
            remaining -= used;
        }

        Ok(entries)
    }

    fn read(&self, ino: u64, offset: u64, size: u64) -> io::Result<Vec<u8>> {
        let inode = self.read_inode(ino)?;

        if offset >= inode.size {
            return Ok(Vec::new());
        }
        let end = (offset + size).min(inode.size);
        let mut out = Vec::with_capacity((end - offset) as usize);
        let mut pos = offset;

        while pos < end {
            let index = (pos / BLOCK_SIZE) as usize;
            let in_block = pos % BLOCK_SIZE;
            let n = (BLOCK_SIZE - in_block).min(end - pos);

            if index >= inode.dpointers.len() {
                break;
            }
            let block = inode.dpointers[index];

            let mut chunk = vec![0u8; n as usize];
            if block != 0 {
                self.file
                    .read_exact_at(&mut chunk, block * BLOCK_SIZE + in_block)?;
            }
            out.extend_from_slice(&chunk);
            pos += n;
        }

        Ok(out)
    }
}

impl Filesystem for Pelican {
    fn getattr(
        &self,
        _req: &fuser::Request,
        ino: fuser::INodeNo,
        _fh: Option<fuser::FileHandle>,
        reply: fuser::ReplyAttr,
    ) {
        let ino = u64::from(ino);
        match self.read_inode(ino) {
            Ok(inode) => reply.attr(&Duration::from_secs(1), &to_attr(ino, &inode)),
            Err(_) => reply.error(fuser::Errno::EIO),
        }
    }

    fn lookup(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let entries = match self.read_dir(u64::from(parent)) {
            Ok(e) => e,
            Err(_) => {
                reply.error(fuser::Errno::EIO);
                return;
            }
        };

        let Some(entry) = entries.iter().find(|e| e.name == name.as_bytes()) else {
            reply.error(fuser::Errno::ENOENT);
            return;
        };

        match self.read_inode(entry.inode) {
            Ok(inode) => reply.entry(
                &Duration::from_secs(1),
                &to_attr(entry.inode, &inode),
                fuser::Generation(0),
            ),
            Err(_) => reply.error(fuser::Errno::EIO),
        }
    }

    fn readdir(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        mut reply: fuser::ReplyDirectory,
    ) {
        let ino = u64::from(ino);
        let entries = match self.read_dir(ino) {
            Ok(e) => e,
            Err(_) => {
                reply.error(fuser::Errno::EIO);
                return;
            }
        };

        for (i, e) in entries.iter().enumerate().skip(offset as usize) {
            let kind = if e.kind == 2 {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            if reply.add(
                INodeNo(e.inode),
                (i + 1) as u64,
                kind,
                OsStr::from_bytes(&e.name),
            ) {
                break;
            }
        }
        reply.ok();
    }

    fn read(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        size: u32,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: fuser::ReplyData,
    ) {
        let ino = u64::from(ino);
        let data = match self.read(ino, offset, size as u64) {
            Ok(e) => e,
            Err(_) => {
                reply.error(fuser::Errno::EIO);
                return;
            }
        };
        reply.data(&data);
    }
}

fn to_attr(ino: u64, inode: &INode) -> FileAttr {
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

pub fn mount(image: &str, mountpoint: &str) -> Result<(), Box<dyn Error>> {
    let file = OpenOptions::new().read(true).write(true).open(image)?;

    let mut buf = [0u8; BLOCK_SIZE as usize];
    file.read_exact_at(&mut buf, 0)?;
    let sb = Mutex::new(SuperBlock::from_bytes(&buf)?);

    let mut config = Config::default();
    config.mount_options = vec![MountOption::RW, MountOption::FSName("pelican".to_string())];
    fuser::mount(Pelican { file, sb }, mountpoint, &config)?;
    Ok(())
}
