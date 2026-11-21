use fuser::Errno;

use crate::util;

pub const DIRENT_SIZE: u64 = 64;
pub const MAX_NAME: usize = 54;

pub fn check_name(name: &[u8]) -> Result<(), Errno> {
    if name.len() > MAX_NAME {
        return Err(Errno::ENAMETOOLONG);
    }
    if name.contains(&b'@') {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

pub struct DirEntry {
    pub inode: u64,
    pub kind: u8, //1 for file, 2 for dir
    pub name: Vec<u8>,
}

impl DirEntry {
    pub fn to_bytes(&self) -> [u8; DIRENT_SIZE as usize] {
        let mut buf = [0u8; DIRENT_SIZE as usize];
        util::put_u64(&mut buf, 0, self.inode);
        buf[8] = self.name.len() as u8;
        buf[9] = self.kind;
        buf[10..10 + self.name.len()].copy_from_slice(&self.name);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> DirEntry {
        let len = buf[8] as usize;
        DirEntry {
            inode: util::get_u64(buf, 0),
            kind: buf[9],
            name: buf[10..10 + len].to_vec(),
        }
    }
}
