use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::{File, OpenOptions},
    io,
    os::unix::{ffi::OsStrExt, fs::FileExt},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fuser::{
    Config, Errno, FileAttr,
    FileType::{self},
    Filesystem, INodeNo, MountOption, RenameFlags,
};

mod convert;
use convert::{errno, to_attr};

use crate::{
    dirent::{DIRENT_SIZE, DirEntry, check_name},
    inode::{INODE_SIZE, INode},
    superblock::{BLOCK_SIZE, ROOT_INODE, SuperBlock},
};

const REVISION_INTERVAL: u64 = 3600;
const LOG_BIT: u64 = 1 << 63;

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

    fn read_dir(&self, inode: &INode) -> io::Result<Vec<DirEntry>> {
        self.read_data(inode, 0, inode.size).map(|data| {
            data.chunks(DIRENT_SIZE as usize)
                .map(DirEntry::from_bytes)
                .filter(|e| e.inode != 0)
                .collect()
        })
    }

    fn read_data(&self, inode: &INode, offset: u64, size: u64) -> io::Result<Vec<u8>> {
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

    fn write_data(&self, inode: &mut INode, offset: u64, data: &[u8]) -> io::Result<usize> {
        let end = offset + data.len() as u64;
        if end > inode.dpointers.len() as u64 * BLOCK_SIZE {
            return Err(io::Error::from_raw_os_error(Errno::EFBIG.code()));
        }

        let mut pos = offset;

        let shared = if inode.revision_head != 0 {
            self.read_inode(inode.revision_head)?.dpointers
        } else {
            [0; 12]
        };

        while pos < end {
            let index = (pos / BLOCK_SIZE) as usize;
            let in_block = pos % BLOCK_SIZE;
            let n = (BLOCK_SIZE - in_block).min(end - pos);

            if inode.dpointers[index] == 0 {
                inode.dpointers[index] = self.alloc_block()?;
            } else if inode.dpointers[index] == shared[index] {
                inode.dpointers[index] = self.copy_block(inode.dpointers[index])?;
            }

            let block = inode.dpointers[index];

            let src = &data[(pos - offset) as usize..(pos - offset + n) as usize];
            self.file.write_all_at(src, block * BLOCK_SIZE + in_block)?;
            pos += n;
        }

        inode.size = inode.size.max(end);
        Ok(data.len())
    }

    fn alloc_block(&self) -> io::Result<u64> {
        let mut sb = self.sb.lock().unwrap();
        if sb.free_blocks == 0 {
            return Err(io::Error::from_raw_os_error(Errno::ENOSPC.code()));
        }

        let mut bitmap = [0u8; BLOCK_SIZE as usize];
        let bitmap_off = sb.block_bitmap_at * BLOCK_SIZE;
        self.file.read_exact_at(&mut bitmap, bitmap_off)?;

        let block = (sb.data_at..sb.total_blocks)
            .find(|&n| bitmap[(n / 8) as usize] & (1 << (n % 8)) == 0)
            .ok_or_else(|| io::Error::from_raw_os_error(Errno::ENOSPC.code()))?;

        bitmap[(block / 8) as usize] |= 1 << (block % 8);
        self.file.write_all_at(&bitmap, bitmap_off)?;

        sb.free_blocks -= 1;
        self.write_superblock(&sb)?;

        self.file
            .write_all_at(&[0u8; BLOCK_SIZE as usize], block * BLOCK_SIZE)?;
        Ok(block)
    }

    fn free_block(&self, block: u64) -> io::Result<()> {
        let mut sb = self.sb.lock().unwrap();

        let mut bitmap = [0u8; BLOCK_SIZE as usize];
        let bitmap_off = sb.block_bitmap_at * BLOCK_SIZE;
        self.file.read_exact_at(&mut bitmap, bitmap_off)?;

        bitmap[(block / 8) as usize] &= !(1 << (block % 8));
        self.file.write_all_at(&bitmap, bitmap_off)?;

        sb.free_blocks += 1;
        self.write_superblock(&sb)
    }

    fn alloc_inode(&self) -> io::Result<u64> {
        let mut sb = self.sb.lock().unwrap();
        if sb.free_inodes == 0 {
            return Err(io::Error::from_raw_os_error(Errno::ENOSPC.code()));
        }

        let mut bitmap = [0u8; BLOCK_SIZE as usize];
        let bitmap_off = sb.inode_bitmap_at * BLOCK_SIZE;
        self.file.read_exact_at(&mut bitmap, bitmap_off)?;

        let ino = (1..sb.inode_count)
            .find(|&n| bitmap[(n / 8) as usize] & (1 << (n % 8)) == 0)
            .ok_or_else(|| io::Error::from_raw_os_error(Errno::ENOSPC.code()))?;

        bitmap[(ino / 8) as usize] |= 1 << (ino % 8);
        self.file.write_all_at(&bitmap, bitmap_off)?;

        sb.free_inodes -= 1;
        self.write_superblock(&sb)?;
        Ok(ino)
    }

    fn add_entry(&self, parent_ino: u64, entry: &DirEntry) -> io::Result<()> {
        let mut parent = self.read_inode(parent_ino)?;
        let data = self.read_data(&parent, 0, parent.size)?;

        let mut free_slot = None;
        for (i, chunk) in data.chunks(DIRENT_SIZE as usize).enumerate() {
            let e = DirEntry::from_bytes(chunk);
            if e.inode == 0 {
                free_slot.get_or_insert(i);
            } else if e.name == entry.name {
                return Err(io::Error::from_raw_os_error(Errno::EEXIST.code()));
            }
        }

        let offset = match free_slot {
            Some(i) => i as u64 * DIRENT_SIZE,
            None => parent.size,
        };
        self.write_data(&mut parent, offset, &entry.to_bytes())?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        parent.mtime = now;
        parent.ctime = now;
        self.write_inode(parent_ino, &parent)
    }

    fn remove_entry(&self, parent_ino: u64, name: &[u8]) -> io::Result<DirEntry> {
        let mut parent = self.read_inode(parent_ino)?;
        let data = self.read_data(&parent, 0, parent.size)?;

        let (slot, entry) = data
            .chunks(DIRENT_SIZE as usize)
            .map(DirEntry::from_bytes)
            .enumerate()
            .find(|(_, e)| e.inode != 0 && e.name == name)
            .ok_or_else(|| io::Error::from_raw_os_error(Errno::ENOENT.code()))?;

        let empty = [0u8; DIRENT_SIZE as usize];
        self.write_data(&mut parent, slot as u64 * DIRENT_SIZE, &empty)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        parent.mtime = now;
        parent.ctime = now;
        self.write_inode(parent_ino, &parent)?;
        Ok(entry)
    }

    fn truncate(&self, inode: &mut INode, new_size: u64) -> io::Result<()> {
        if new_size > inode.dpointers.len() as u64 * BLOCK_SIZE {
            return Err(io::Error::from_raw_os_error(Errno::EFBIG.code()));
        }

        if new_size < inode.size {
            let shared = if inode.revision_head != 0 {
                self.read_inode(inode.revision_head)?.dpointers
            } else {
                [0; 12]
            };

            let keep = new_size.div_ceil(BLOCK_SIZE) as usize;
            for i in keep..inode.dpointers.len() {
                let block = inode.dpointers[i];
                if block != 0 && block != shared[i] {
                    self.free_block(block)?;
                }
                inode.dpointers[i] = 0;
            }

            let in_block = new_size % BLOCK_SIZE;
            if in_block != 0 && inode.dpointers[keep - 1] != 0 {
                if inode.dpointers[keep - 1] == shared[keep - 1] {
                    inode.dpointers[keep - 1] = self.copy_block(inode.dpointers[keep - 1])?;
                }
                let block = inode.dpointers[keep - 1];
                let tail = vec![0u8; (BLOCK_SIZE - in_block) as usize];
                self.file
                    .write_all_at(&tail, block * BLOCK_SIZE + in_block)?;
            }
        }

        inode.size = new_size;
        Ok(())
    }

    fn destroy_inode(&self, ino: u64, inode: &INode) -> io::Result<()> {
        let mut inodes = vec![ino];
        let mut blocks: HashSet<u64> = inode
            .dpointers
            .iter()
            .copied()
            .filter(|&b| b != 0)
            .collect();

        let mut next = inode.revision_head;
        while next != 0 {
            let rev = self.read_inode(next)?;
            blocks.extend(rev.dpointers.iter().copied().filter(|&b| b != 0));
            inodes.push(next);
            next = rev.revision_head;
        }

        let mut sb = self.sb.lock().unwrap();

        let mut bmap = [0u8; BLOCK_SIZE as usize];
        self.file
            .read_exact_at(&mut bmap, sb.block_bitmap_at * BLOCK_SIZE)?;
        for &b in &blocks {
            bmap[(b / 8) as usize] &= !(1 << (b % 8));
        }
        self.file
            .write_all_at(&bmap, sb.block_bitmap_at * BLOCK_SIZE)?;

        let mut imap = [0u8; BLOCK_SIZE as usize];
        self.file
            .read_exact_at(&mut imap, sb.inode_bitmap_at * BLOCK_SIZE)?;
        for &i in &inodes {
            imap[(i / 8) as usize] &= !(1 << (i % 8));
        }
        self.file
            .write_all_at(&imap, sb.inode_bitmap_at * BLOCK_SIZE)?;

        sb.free_blocks += blocks.len() as u64;
        sb.free_inodes += inodes.len() as u64;
        self.write_superblock(&sb)
    }

    fn find_entry(&self, dir: u64, name: &[u8]) -> io::Result<Option<DirEntry>> {
        let inode = self.read_inode(dir)?;
        Ok(self.read_dir(&inode)?.into_iter().find(|e| e.name == name))
    }

    fn is_ancestor(&self, ino: u64, mut dir: u64) -> io::Result<bool> {
        loop {
            if dir == ino {
                return Ok(true);
            }
            if dir == ROOT_INODE {
                return Ok(false);
            }
            dir = self
                .find_entry(dir, b"..")?
                .map(|e| e.inode)
                .unwrap_or(ROOT_INODE);
        }
    }

    fn add_nlink(&self, ino: u64, delta: i64) -> io::Result<()> {
        let mut inode = self.read_inode(ino)?;
        inode.nlink = (inode.nlink as i64 + delta) as u64;
        self.write_inode(ino, &inode)
    }

    fn snapshot(&self, inode: &mut INode, now: u64) -> io::Result<()> {
        let snap_ino = self.alloc_inode()?;

        let mut snap = inode.clone();
        snap.nlink = 0;
        self.write_inode(snap_ino, &snap)?;

        inode.revision_head = snap_ino;
        inode.rtime = now;
        Ok(())
    }

    fn copy_block(&self, old: u64) -> io::Result<u64> {
        let new = self.alloc_block()?;
        let mut buf = [0u8; BLOCK_SIZE as usize];
        self.file.read_exact_at(&mut buf, old * BLOCK_SIZE)?;
        self.file.write_all_at(&buf, new * BLOCK_SIZE)?;
        Ok(new)
    }

    fn revision_log(&self, file_ino: u64) -> io::Result<Vec<u8>> {
        let mut out = String::new();
        let mut next = file_ino;
        let mut index = 0;
        while next != 0 {
            let rev = self.read_inode(next)?;
            out.push_str(&format!("{index} -> {}\n", rev.mtime));
            next = rev.revision_head;
            index += 1
        }

        Ok(out.into_bytes())
    }

    fn log_attr(&self, file_ino: u64) -> io::Result<FileAttr> {
        let inode = self.read_inode(file_ino)?;
        let text = self.revision_log(file_ino)?;

        let mut attr = to_attr(LOG_BIT | file_ino, &inode);
        attr.kind = FileType::RegularFile;
        attr.size = text.len() as u64;
        attr.perm = 0o444;
        attr.nlink = 1;
        Ok(attr)
    }

    fn lookup_version(&self, parent: u64, base: &[u8], suffix: &[u8]) -> Result<FileAttr, Errno> {
        let file = self.find_entry(parent, base)?.ok_or(Errno::ENOENT)?;
        if file.kind == 2 {
            return Err(Errno::ENOENT);
        }

        if suffix == b"log" {
            return Ok(self.log_attr(file.inode)?);
        }

        // N: walk N steps down the revision chain
        let n: u64 = std::str::from_utf8(suffix)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or(Errno::ENOENT)?;

        let mut ino = file.inode;
        for _ in 0..n {
            ino = self.read_inode(ino)?.revision_head;
            if ino == 0 {
                return Err(Errno::ENOENT); // no such revision
            }
        }
        Ok(to_attr(ino, &self.read_inode(ino)?))
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

        if ino & LOG_BIT != 0 {
            return match self.log_attr(ino & !LOG_BIT) {
                Ok(attr) => reply.attr(&Duration::from_secs(1), &attr),
                Err(e) => reply.error(e.into()),
            };
        }

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
        let name = name.as_bytes();
        if let Some(at) = name.iter().rposition(|&b| b == b'@') {
            let (base, suffix) = (&name[..at], &name[at + 1..]);
            return match self.lookup_version(u64::from(parent), base, suffix) {
                Ok(attr) => reply.entry(&Duration::from_secs(1), &attr, fuser::Generation(0)),
                Err(e) => reply.error(e),
            };
        }

        let entries = match self
            .read_inode(u64::from(parent))
            .and_then(|inode| self.read_dir(&inode))
        {
            Ok(e) => e,
            Err(_) => {
                reply.error(fuser::Errno::EIO);
                return;
            }
        };

        let Some(entry) = entries.iter().find(|e| e.name == name) else {
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
        let entries = match self.read_inode(ino).and_then(|inode| self.read_dir(&inode)) {
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

        if ino & LOG_BIT != 0 {
            return match self.revision_log(ino & !LOG_BIT) {
                Ok(text) => {
                    let start = (offset as usize).min(text.len());
                    let end = (start + size as usize).min(text.len());
                    reply.data(&text[start..end])
                }
                Err(e) => reply.error(e.into()),
            };
        }

        let result = self
            .read_inode(ino)
            .and_then(|inode| self.read_data(&inode, offset, size as u64));

        match result {
            Ok(data) => reply.data(&data),
            Err(_) => reply.error(fuser::Errno::EIO),
        }
    }

    fn write(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: fuser::WriteFlags,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: fuser::ReplyWrite,
    ) {
        let ino = u64::from(ino);
        if ino & LOG_BIT != 0 {
            return reply.error(Errno::EROFS);
        }
        let result = self.read_inode(ino).and_then(|mut inode| {
            if inode.nlink == 0 {
                return Err(io::Error::from_raw_os_error(Errno::EROFS.code()));
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now.saturating_sub(inode.rtime) > REVISION_INTERVAL {
                self.snapshot(&mut inode, now)?;
            }

            let n = self.write_data(&mut inode, offset, data)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            inode.mtime = now;
            inode.ctime = now;
            self.write_inode(ino, &inode)?;
            Ok(n)
        });

        match result {
            Ok(n) => reply.written(n as u32),
            Err(e) => reply.error(e.into()),
        }
    }

    fn create(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        if let Err(e) = check_name(name.as_bytes()) {
            return reply.error(e);
        }

        let result = self.alloc_inode().and_then(|ino| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let inode = INode {
                kind: 0o100000 | (mode & !umask & 0o7777) as u64,
                uid: req.uid() as u64,
                gid: req.gid() as u64,
                size: 0,
                atime: now,
                mtime: now,
                ctime: now,
                nlink: 1,
                dpointers: [0; 12],
                ipointer: 0,
                revision_head: 0,
                rtime: now,
            };
            self.write_inode(ino, &inode)?;

            let entry = DirEntry {
                inode: ino,
                kind: 1,
                name: name.as_bytes().to_vec(),
            };
            self.add_entry(u64::from(parent), &entry)?;
            Ok((ino, inode))
        });

        match result {
            Ok((ino, inode)) => reply.created(
                &Duration::from_secs(1),
                &to_attr(ino, &inode),
                fuser::Generation(0),
                fuser::FileHandle(0),
                fuser::FopenFlags::empty(),
            ),
            Err(e) => reply.error(e.into()),
        }
    }

    fn setattr(
        &self,
        _req: &fuser::Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<fuser::TimeOrNow>,
        mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<fuser::FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: fuser::ReplyAttr,
    ) {
        let ino = u64::from(ino);
        if ino & LOG_BIT != 0 {
            return reply.error(Errno::EROFS);
        }
        let result = self.read_inode(ino).and_then(|mut inode| {
            if inode.nlink == 0 {
                return Err(io::Error::from_raw_os_error(Errno::EROFS.code()));
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let secs = |t: fuser::TimeOrNow| match t {
                fuser::TimeOrNow::Now => now,
                fuser::TimeOrNow::SpecificTime(st) => st
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            };

            if let Some(m) = mode {
                inode.kind = (inode.kind & 0o170000) | (m as u64 & 0o7777);
            }
            if let Some(u) = uid {
                inode.uid = u as u64;
            }
            if let Some(g) = gid {
                inode.gid = g as u64;
            }
            if let Some(sz) = size {
                if now.saturating_sub(inode.rtime) > REVISION_INTERVAL {
                    self.snapshot(&mut inode, now)?;
                }
                self.truncate(&mut inode, sz)?;
                inode.mtime = now;
            }
            if let Some(t) = atime {
                inode.atime = secs(t);
            }
            if let Some(t) = mtime {
                inode.mtime = secs(t);
            }
            inode.ctime = now;

            self.write_inode(ino, &inode)?;
            Ok(to_attr(ino, &inode))
        });

        match result {
            Ok(attr) => reply.attr(&Duration::from_secs(1), &attr),
            Err(e) => reply.error(e.into()),
        }
    }

    fn mkdir(
        &self,
        req: &fuser::Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: fuser::ReplyEntry,
    ) {
        if let Err(e) = check_name(name.as_bytes()) {
            return reply.error(e);
        }
        let parent = u64::from(parent);

        let result = self.alloc_inode().and_then(|ino| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let mut inode = INode {
                kind: 0o040000 | (mode & !umask & 0o7777) as u64,
                uid: req.uid() as u64,
                gid: req.gid() as u64,
                size: 0,
                atime: now,
                mtime: now,
                ctime: now,
                nlink: 2,
                dpointers: [0; 12],
                ipointer: 0,
                revision_head: 0,
                rtime: now,
            };

            let dot = DirEntry {
                inode: ino,
                kind: 2,
                name: b".".to_vec(),
            };
            let dotdot = DirEntry {
                inode: parent,
                kind: 2,
                name: b"..".to_vec(),
            };
            self.write_data(&mut inode, 0, &dot.to_bytes())?;
            self.write_data(&mut inode, DIRENT_SIZE, &dotdot.to_bytes())?;
            self.write_inode(ino, &inode)?;

            let entry = DirEntry {
                inode: ino,
                kind: 2,
                name: name.as_bytes().to_vec(),
            };
            self.add_entry(parent, &entry)?;

            let mut p = self.read_inode(parent)?;
            p.nlink += 1;
            self.write_inode(parent, &p)?;

            Ok((ino, inode))
        });

        match result {
            Ok((ino, inode)) => reply.entry(
                &Duration::from_secs(1),
                &to_attr(ino, &inode),
                fuser::Generation(0),
            ),
            Err(e) => reply.error(e.into()),
        }
    }

    fn unlink(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        let parent = u64::from(parent);
        let result = self
            .read_inode(parent)
            .and_then(|p| self.read_dir(&p))
            .and_then(|entries| {
                let entry = entries
                    .into_iter()
                    .find(|e| e.name == name.as_bytes())
                    .ok_or_else(|| io::Error::from_raw_os_error(Errno::ENOENT.code()))?;
                if entry.kind == 2 {
                    return Err(io::Error::from_raw_os_error(Errno::EISDIR.code()));
                }

                self.remove_entry(parent, name.as_bytes())?;

                let mut inode = self.read_inode(entry.inode)?;
                inode.nlink -= 1;
                if inode.nlink == 0 {
                    self.destroy_inode(entry.inode, &mut inode)
                } else {
                    inode.ctime = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    self.write_inode(entry.inode, &inode)
                }
            });

        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }

    fn rmdir(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        let parent = u64::from(parent);
        let result = self
            .read_inode(parent)
            .and_then(|p| self.read_dir(&p))
            .and_then(|entries| {
                let entry = entries
                    .into_iter()
                    .find(|e| e.name == name.as_bytes())
                    .ok_or_else(|| io::Error::from_raw_os_error(Errno::ENOENT.code()))?;
                if entry.kind != 2 {
                    return Err(io::Error::from_raw_os_error(Errno::ENOTDIR.code()));
                }

                let mut dir = self.read_inode(entry.inode)?;
                let children = self.read_dir(&dir)?;
                if children.iter().any(|c| c.name != b"." && c.name != b"..") {
                    return Err(io::Error::from_raw_os_error(Errno::ENOTEMPTY.code()));
                }

                self.remove_entry(parent, name.as_bytes())?;
                self.destroy_inode(entry.inode, &mut dir)?;

                let mut p = self.read_inode(parent)?;
                p.nlink -= 1;
                self.write_inode(parent, &p)
            });

        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }

    fn rename(
        &self,
        _req: &fuser::Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: fuser::RenameFlags,
        reply: fuser::ReplyEmpty,
    ) {
        if let Err(e) = check_name(name.as_bytes()) {
            return reply.error(e);
        }

        let (parent, newparent) = (u64::from(parent), u64::from(newparent));
        let (name, newname) = (name.as_bytes(), newname.as_bytes());

        let result = (|| -> io::Result<()> {
            let src = self
                .find_entry(parent, name)?
                .ok_or_else(|| errno(Errno::ENOENT))?;
            let src_is_dir = src.kind == 2;

            if src_is_dir && self.is_ancestor(src.inode, newparent)? {
                return Err(errno(Errno::EINVAL));
            }

            if let Some(dst) = self.find_entry(newparent, newname)? {
                if dst.inode == src.inode {
                    return Ok(());
                }
                if flags.contains(RenameFlags::RENAME_NOREPLACE) {
                    return Err(errno(Errno::EEXIST));
                }
                let dst_is_dir = dst.kind == 2;
                match (src_is_dir, dst_is_dir) {
                    (false, true) => return Err(errno(Errno::EISDIR)),
                    (true, false) => return Err(errno(Errno::ENOTDIR)),
                    _ => {}
                }

                let mut dst_inode = self.read_inode(dst.inode)?;
                if dst_is_dir {
                    let children = self.read_dir(&dst_inode)?;
                    if children.iter().any(|c| c.name != b"." && c.name != b"..") {
                        return Err(errno(Errno::ENOTEMPTY));
                    }
                    self.remove_entry(newparent, newname)?;
                    self.destroy_inode(dst.inode, &mut dst_inode)?;
                    self.add_nlink(newparent, -1)?;
                } else {
                    self.remove_entry(newparent, newname)?;
                    dst_inode.nlink -= 1;
                    if dst_inode.nlink == 0 {
                        self.destroy_inode(dst.inode, &mut dst_inode)?;
                    } else {
                        self.write_inode(dst.inode, &dst_inode)?;
                    }
                }
            }

            self.remove_entry(parent, name)?;
            let moved = DirEntry {
                inode: src.inode,
                kind: src.kind,
                name: newname.to_vec(),
            };
            self.add_entry(newparent, &moved)?;

            if src_is_dir && parent != newparent {
                self.remove_entry(src.inode, b"..")?;
                let dotdot = DirEntry {
                    inode: newparent,
                    kind: 2,
                    name: b"..".to_vec(),
                };
                self.add_entry(src.inode, &dotdot)?;
                self.add_nlink(parent, -1)?;
                self.add_nlink(newparent, 1)?;
            }

            let mut inode = self.read_inode(src.inode)?;
            inode.ctime = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            self.write_inode(src.inode, &inode)
        })();

        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }
}

pub fn mount(image: &str, mountpoint: &str) -> io::Result<()> {
    let file = OpenOptions::new().read(true).write(true).open(image)?;

    let mut buf = [0u8; BLOCK_SIZE as usize];
    file.read_exact_at(&mut buf, 0)?;
    let sb = Mutex::new(SuperBlock::from_bytes(&buf)?);

    let mut config = Config::default();
    config.mount_options = vec![MountOption::RW, MountOption::FSName("pelican".to_string())];
    fuser::mount(Pelican { file, sb }, mountpoint, &config)?;
    Ok(())
}
