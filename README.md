# Pelican

A time-machine filesystem in userspace (FUSE). It keeps old versions of your files.

```
$ cat notes.txt@log
3 versions

  @0   2026-11-21 14:05:37   (current)
  @1   2026-11-21 12:40:09
  @2   2026-11-20 18:10:44

$ cat notes.txt@2        # how notes.txt looked on 2026-11-20
```

## run

```bash
truncate -s 100M disc.img          # 272 KB min, 128 MB max
cargo run -- mkfs disc.img
cargo run -- mount disc.img ./tmp
fusermount3 -u ./tmp               # unmount
```

## revisions

- a file gets a new revision when it's edited and its last revision is older than 1 hour
- `file@log` lists versions, `file@N` opens one (`@0` is the current file)
- revisions are read-only
- `@` can't be used in names
- `rm` deletes the file and its history
- directories aren't versioned

For testing, set the interval in seconds: `PELICAN_REVISION_INTERVAL=5 cargo run -- mount disc.img ./tmp`

## disk layout

4096-byte blocks.

| block |                                                                 |
| ----- | --------------------------------------------------------------- |
| 0     | superblock (magic `PELICAN5`, counts, where each region starts) |
| 1     | block bitmap                                                    |
| 2     | inode bitmap                                                    |
| 3–66  | inode table, 1024 inodes × 256 bytes                            |
| 67+   | data                                                            |

inode (256 bytes):

| offset     | field                                        |
| ---------- | -------------------------------------------- |
| 0          | kind (type + permissions, e.g. `0o100644`)   |
| 8, 16      | uid, gid                                     |
| 24         | size                                         |
| 32, 40, 48 | atime, mtime, ctime                          |
| 56         | 12 direct block pointers                     |
| 152        | indirect pointer (unused)                    |
| 160        | revision_head: inode of the previous version |
| 168        | nlink                                        |
| 176        | rtime: when the last revision was made       |

directory entry (64 bytes): inode `u64`, name length `u8`, kind `u8` (1 file, 2 dir), name (54 bytes max).

A revision is a copy of the inode (same block pointers) linked through `revision_head`. Writes to the current file copy a shared block before changing it, so old versions stay as they were.

## limits

- files up to 48 KB (12 direct blocks)
- 54-byte names, 768 entries per directory
- no symlinks or hard links

## todo

- make it a distributed filesystem
