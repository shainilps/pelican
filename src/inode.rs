use crate::util;

pub const INODE_SIZE: u64 = 256;

#[derive(Clone)]
pub struct INode {
    pub kind: u64,
    pub uid: u64,
    pub gid: u64,
    pub size: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub nlink: u64,
    pub dpointers: [u64; 12],
    pub ipointer: u64,
    pub revision_head: u64,
    pub rtime: u64,
    //rest padding
}

impl INode {
    pub fn to_bytes(&self) -> [u8; INODE_SIZE as usize] {
        let mut buf = [0u8; INODE_SIZE as usize];
        util::put_u64(&mut buf, 0, self.kind);
        util::put_u64(&mut buf, 8, self.uid);
        util::put_u64(&mut buf, 16, self.gid);
        util::put_u64(&mut buf, 24, self.size);
        util::put_u64(&mut buf, 32, self.atime);
        util::put_u64(&mut buf, 40, self.mtime);
        util::put_u64(&mut buf, 48, self.ctime);
        util::put_u64_array(&mut buf, 56, &self.dpointers);
        util::put_u64(&mut buf, 152, self.ipointer);
        util::put_u64(&mut buf, 160, self.revision_head);
        util::put_u64(&mut buf, 168, self.nlink);
        util::put_u64(&mut buf, 176, self.rtime);
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> INode {
        INode {
            kind: util::get_u64(buf, 0),
            uid: util::get_u64(buf, 8),
            gid: util::get_u64(buf, 16),
            size: util::get_u64(buf, 24),
            atime: util::get_u64(buf, 32),
            mtime: util::get_u64(buf, 40),
            ctime: util::get_u64(buf, 48),
            dpointers: util::get_u64_array(buf, 56),
            ipointer: util::get_u64(buf, 152),
            revision_head: util::get_u64(buf, 160),
            nlink: util::get_u64(buf, 168),
            rtime: util::get_u64(buf, 176),
        }
    }
}
