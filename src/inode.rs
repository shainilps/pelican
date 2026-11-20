use crate::util;

pub struct INode {
    pub kind: u64,
    pub uid: u64,
    pub gid: u64,
    pub size: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub d_pointers: [u64; 12],
    pub i_pointer: u64,
    pub revision_head: u64,
    //rest padding
}

impl INode {
    pub fn to_bytes(&self) -> [u8; 256] {
        let mut buf = [0u8; 256];
        util::put_u64(&mut buf, 0, self.kind);
        util::put_u64(&mut buf, 8, self.uid);
        util::put_u64(&mut buf, 16, self.gid);
        util::put_u64(&mut buf, 24, self.size);
        util::put_u64(&mut buf, 32, self.atime);
        util::put_u64(&mut buf, 40, self.mtime);
        util::put_u64(&mut buf, 48, self.ctime);
        util::put_u64_array(&mut buf, 56, &self.d_pointers); // 56..152
        util::put_u64(&mut buf, 152, self.i_pointer);
        util::put_u64(&mut buf, 160, self.revision_head);
        buf
    }
}
