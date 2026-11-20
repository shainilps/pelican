# Pelican

A Time-Machine File-System in userspace (FUSE).

```bash
RUST_LOG=debug cargo run -- /dev/loop0 ./tmp

```

4096 bytes blocksize

=> for 100mb
100 * 1024 * 1024 = 104857600

=> res/4096
blocks number: 25,600

super block: 100bytes (block0)
magic = "PELICAN5" is this our disk?
block_size = 4096
total_blocks = 25600
inode_count = 1024 how many inode slots exist
inode_bitmap_at = 2 block numbers where each region starts
block_bitmap_at = 1
inode_table_at = 3
data_at = 67
free_blocks = ... counts, so you don't have to scan
free_inodes = ...
root_inode = 1

bitmap:
block 1: 8 * 4096 = 32k (can cover upto 128mb)
block 1: 8 * 4096 = 32k (can cover upto 128mb)

Inode:

kind (file/dir) + perm 4 bytes
uid, gid 8
nlink 4
size in bytes 8
atime, mtime, ctime 24
direct block pointers 12 x 8 = 96 "my data is in blocks 70, 71, 85..."
indirect pointer 8
revision head //it stores the inode
spare for later rest
--------------------------------


total 256 bytes

- 16 inode per block

rest of the blocks data block
