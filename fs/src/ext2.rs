extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::{BlockDevice, SECTOR_SIZE};
use crate::vfs::{DirEntry, FileSystem, VfsError, Vnode, VnodeType};

// ── On-disk structures (RFC 3010 / Ext2 spec) ────────────────────────────────
//
// These packed views are retained for the Ext4 driver, which reuses the Ext2
// superblock / group-descriptor / inode layouts.  The Ext2 driver itself works
// on raw little-endian byte buffers so that reads and writes never depend on
// struct alignment.

/// Ext2 superblock — lives at byte offset 1024 in the partition image.
#[repr(C, packed)]
pub struct Superblock {
    pub s_inodes_count: u32,
    pub s_blocks_count: u32,
    pub s_r_blocks_count: u32,
    pub s_free_blocks_count: u32,
    pub s_free_inodes_count: u32,
    pub s_first_data_block: u32,
    pub s_log_block_size: u32,
    pub s_log_frag_size: u32,
    pub s_blocks_per_group: u32,
    pub s_frags_per_group: u32,
    pub s_inodes_per_group: u32,
    pub s_mtime: u32,
    pub s_wtime: u32,
    pub s_mnt_count: u16,
    pub s_max_mnt_count: u16,
    pub s_magic: u16,
}

/// Ext2 block group descriptor (32 bytes each).
#[repr(C, packed)]
pub struct BlockGroupDescriptor {
    pub bg_block_bitmap: u32,
    pub bg_inode_bitmap: u32,
    pub bg_inode_table: u32,
    pub bg_free_blocks_count: u16,
    pub bg_free_inodes_count: u16,
    pub bg_used_dirs_count: u16,
    pub bg_pad: u16,
    pub bg_reserved: [u32; 3],
}

/// Ext2 inode (128 bytes).
#[repr(C, packed)]
pub struct Inode {
    pub i_mode: u16,
    pub i_uid: u16,
    pub i_size: u32,
    pub i_atime: u32,
    pub i_ctime: u32,
    pub i_mtime: u32,
    pub i_dtime: u32,
    pub i_gid: u16,
    pub i_links_count: u16,
    pub i_blocks: u32,
    pub i_flags: u32,
    pub i_osd1: u32,
    pub i_block: [u32; 15],
    pub i_generation: u32,
    pub i_file_acl: u32,
    pub i_dir_acl: u32,
    pub i_faddr: u32,
    pub i_osd2: [u8; 12],
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Ext2 superblock magic value.
pub const EXT2_MAGIC: u16 = 0xEF53;
/// Root directory inode number.
pub const EXT2_ROOT_INO: u64 = 2;

const EXT2_SUPER_OFFSET: u64 = 1024;
const EXT2_SUPER_SIZE: usize = 1024;

const S_IFMT: u16 = 0xF000;
const S_IFREG: u16 = 0x8000;
const S_IFDIR: u16 = 0x4000;

const FT_REGULAR: u8 = 1;
const FT_DIR: u8 = 2;
const FT_SYMLINK: u8 = 7;

// Superblock byte offsets.
const SB_INODES_COUNT: usize = 0x00;
const SB_BLOCKS_COUNT: usize = 0x04;
const SB_FREE_BLOCKS: usize = 0x0C;
const SB_FREE_INODES: usize = 0x10;
const SB_FIRST_DATA_BLOCK: usize = 0x14;
const SB_LOG_BLOCK_SIZE: usize = 0x18;
const SB_BLOCKS_PER_GROUP: usize = 0x20;
const SB_INODES_PER_GROUP: usize = 0x28;
const SB_MAGIC: usize = 0x38;
const SB_STATE: usize = 0x3A;
const SB_REV_LEVEL: usize = 0x4C;
const SB_FIRST_INO: usize = 0x54;
const SB_INODE_SIZE: usize = 0x58;

// Group-descriptor byte offsets.
const GD_BLOCK_BITMAP: usize = 0x00;
const GD_INODE_BITMAP: usize = 0x04;
const GD_INODE_TABLE: usize = 0x08;
const GD_FREE_BLOCKS: usize = 0x0C;
const GD_FREE_INODES: usize = 0x0E;
const GD_USED_DIRS: usize = 0x10;

// Inode byte offsets.
const IN_MODE: usize = 0x00;
const IN_SIZE: usize = 0x04;
const IN_LINKS: usize = 0x1A;
const IN_BLOCKS: usize = 0x1C;
const IN_BLOCK: usize = 0x28; // [u32; 15]

// ── Little-endian helpers ────────────────────────────────────────────────────

#[inline]
fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
fn wr16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn wr32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

// Read-modify-write helpers: these avoid the borrow conflict that arises from
// `wr32(buf, o, rd32(buf, o) + 1)`.
#[inline]
fn inc32(b: &mut [u8], o: usize) {
    wr32(b, o, rd32(b, o).wrapping_add(1));
}
#[inline]
fn dec32(b: &mut [u8], o: usize) {
    wr32(b, o, rd32(b, o).saturating_sub(1));
}
#[inline]
fn inc16(b: &mut [u8], o: usize) {
    wr16(b, o, rd16(b, o).wrapping_add(1));
}
#[inline]
fn dec16(b: &mut [u8], o: usize) {
    wr16(b, o, rd16(b, o).saturating_sub(1));
}

fn ft_to_vtype(ft: u8) -> VnodeType {
    match ft {
        FT_DIR => VnodeType::Directory,
        FT_SYMLINK => VnodeType::Symlink,
        _ => VnodeType::File,
    }
}

/// Write a directory entry into `data` at `off` with `rec_len` bytes.
fn set_dir_entry(data: &mut [u8], off: usize, rec_len: u16, inode: u32, name: &[u8], ft: u8) {
    wr32(data, off, inode);
    wr16(data, off + 4, rec_len);
    data[off + 6] = name.len() as u8;
    data[off + 7] = ft;
    data[off + 8..off + 8 + name.len()].copy_from_slice(name);
    let used = align4(8 + name.len());
    for b in data[off + used..off + rec_len as usize].iter_mut() {
        *b = 0;
    }
}

/// Parse the visible entries of a directory block image.
fn parse_dir_entries(data: &[u8]) -> Vec<DirEntry> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let rec = rd16(data, off + 4) as usize;
        if rec == 0 || off + rec > data.len() {
            break;
        }
        let nlen = data[off + 6] as usize;
        let ino = rd32(data, off);
        if ino != 0 && 8 + nlen <= rec {
            if let Ok(n) = core::str::from_utf8(&data[off + 8..off + 8 + nlen]) {
                if n != "." && n != ".." {
                    out.push(DirEntry {
                        name: String::from(n),
                        inode: ino as u64,
                        vtype: ft_to_vtype(data[off + 7]),
                    });
                }
            }
        }
        off += rec;
    }
    out
}

/// Find `name` inside a directory block image.
fn find_dir_entry(data: &[u8], name: &str) -> Option<(u32, u8, usize)> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let rec = rd16(data, off + 4) as usize;
        if rec == 0 || off + rec > data.len() {
            break;
        }
        let nlen = data[off + 6] as usize;
        let ino = rd32(data, off);
        if ino != 0 && 8 + nlen <= rec {
            if let Ok(n) = core::str::from_utf8(&data[off + 8..off + 8 + nlen]) {
                if n == name {
                    return Some((ino, data[off + 7], off));
                }
            }
        }
        off += rec;
    }
    None
}

// ── Ext2Fs ───────────────────────────────────────────────────────────────────

/// A writable Ext2 filesystem backed by a [`BlockDevice`].
///
/// Supports direct and single/double/triple-indirect block mapping, file and
/// directory creation, unlinking and data writes.
pub struct Ext2Fs {
    dev: Arc<dyn BlockDevice>,
    block_size: u32,
    inode_size: u32,
    inodes_per_group: u32,
    inodes_count: u32,
    blocks_count: u32,
    groups: u32,
}

impl Ext2Fs {
    /// Open an existing Ext2 filesystem on `dev`.
    pub fn from_device(dev: Arc<dyn BlockDevice>) -> Result<Self, VfsError> {
        let sb = dev.read_exact(EXT2_SUPER_OFFSET, EXT2_SUPER_SIZE)?;
        if rd16(&sb, SB_MAGIC) != EXT2_MAGIC {
            return Err(VfsError::IOError);
        }
        let block_size = 1024u32 << rd32(&sb, SB_LOG_BLOCK_SIZE);
        let inode_size = {
            let rev = rd32(&sb, SB_REV_LEVEL);
            if rev == 0 {
                128
            } else {
                let s = rd16(&sb, SB_INODE_SIZE);
                if s == 0 {
                    128
                } else {
                    s as u32
                }
            }
        };
        let inodes_count = rd32(&sb, SB_INODES_COUNT);
        let blocks_count = rd32(&sb, SB_BLOCKS_COUNT);
        let inodes_per_group = rd32(&sb, SB_INODES_PER_GROUP).max(1);
        let groups = inodes_count.div_ceil(inodes_per_group).max(1);
        Ok(Self {
            dev,
            block_size,
            inode_size,
            inodes_per_group,
            inodes_count,
            blocks_count,
            groups,
        })
    }

    /// Convenience: open an Ext2 image already held in memory.
    pub fn new(image: Vec<u8>) -> Self {
        let dev = Arc::new(crate::block::MemBlockDevice::from_image(image));
        Self::from_device(dev).expect("Ext2Fs::new: invalid Ext2 image")
    }

    fn read_super(&self) -> Result<Vec<u8>, VfsError> {
        self.dev.read_exact(EXT2_SUPER_OFFSET, EXT2_SUPER_SIZE)
    }

    fn write_super(&self, sb: &[u8]) -> Result<(), VfsError> {
        self.dev.write_at(EXT2_SUPER_OFFSET, &sb[..EXT2_SUPER_SIZE])
    }

    fn gdt_offset(&self) -> u64 {
        if self.block_size == 1024 {
            2048
        } else {
            self.block_size as u64
        }
    }

    fn read_gd(&self, group: u32) -> Result<[u8; 32], VfsError> {
        let off = self.gdt_offset() + group as u64 * 32;
        let mut buf = [0u8; 32];
        self.dev.read_at(off, &mut buf)?;
        Ok(buf)
    }

    fn write_gd(&self, group: u32, gd: &[u8; 32]) -> Result<(), VfsError> {
        let off = self.gdt_offset() + group as u64 * 32;
        self.dev.write_at(off, gd)
    }

    fn read_block(&self, block: u32) -> Result<Vec<u8>, VfsError> {
        self.dev.read_exact(block as u64 * self.block_size as u64, self.block_size as usize)
    }

    fn write_block(&self, block: u32, buf: &[u8]) -> Result<(), VfsError> {
        self.dev.write_at(block as u64 * self.block_size as u64, &buf[..self.block_size as usize])
    }

    fn zero_block(&self, block: u32) -> Result<(), VfsError> {
        let zeros = vec![0u8; self.block_size as usize];
        self.write_block(block, &zeros)
    }

    fn inode_offset(&self, ino: u64) -> Result<u64, VfsError> {
        let group = ((ino - 1) / self.inodes_per_group as u64) as u32;
        let index = ((ino - 1) % self.inodes_per_group as u64) as u64;
        if group >= self.groups {
            return Err(VfsError::FileNotFound);
        }
        let gd = self.read_gd(group)?;
        let table = rd32(&gd, GD_INODE_TABLE) as u64;
        Ok(table * self.block_size as u64 + index * self.inode_size as u64)
    }

    fn read_inode(&self, ino: u64) -> Result<Vec<u8>, VfsError> {
        if ino == 0 || ino > self.inodes_count as u64 {
            return Err(VfsError::FileNotFound);
        }
        let off = self.inode_offset(ino)?;
        self.dev.read_exact(off, self.inode_size as usize)
    }

    fn write_inode(&self, ino: u64, inode: &[u8]) -> Result<(), VfsError> {
        let off = self.inode_offset(ino)?;
        self.dev.write_at(off, &inode[..self.inode_size as usize])
    }

    fn inode_is_dir(&self, inode: &[u8]) -> bool {
        rd16(inode, IN_MODE) & S_IFMT == S_IFDIR
    }

    fn inode_size_of(&self, inode: &[u8]) -> u64 {
        rd32(inode, IN_SIZE) as u64
    }

    // ── block mapping ────────────────────────────────────────────────────────

    fn ppb(&self) -> u64 {
        self.block_size as u64 / 4
    }

    /// Descend `level` levels of indirection inside `table_block`, allocating
    /// each table on the way down when `allocate` is true.
    fn map_indirect(
        &self,
        table_block: u32,
        level: u32,
        idx: u64,
        allocate: bool,
    ) -> Result<u32, VfsError> {
        let ppb = self.ppb();
        let mut buf = self.read_block(table_block)?;
        let entry = (idx / ppb.pow(level)) as usize;
        let mut blk = rd32(&buf, entry * 4);
        if blk == 0 {
            if !allocate {
                return Ok(0);
            }
            blk = self.alloc_block()?;
            self.zero_block(blk)?;
            wr32(&mut buf, entry * 4, blk);
            self.write_block(table_block, &buf)?;
        }
        if level == 0 {
            Ok(blk)
        } else {
            let sub = idx % ppb.pow(level);
            self.map_indirect(blk, level - 1, sub, allocate)
        }
    }

    /// Map a logical block to a physical block, optionally allocating it.
    /// Returns 0 for an unallocated hole when `allocate` is false.
    fn map_block(&self, ino: u64, logical: u64, allocate: bool) -> Result<u32, VfsError> {
        let ppb = self.ppb();
        let mut inode = self.read_inode(ino)?;
        let mut dirty = false;

        let indirect = |slot: usize,
                        level: u32,
                        idx: u64,
                        inode: &mut Vec<u8>,
                        dirty: &mut bool|
         -> Result<u32, VfsError> {
            let mut t = rd32(inode, slot);
            if t == 0 {
                if !allocate {
                    return Ok(0);
                }
                t = self.alloc_block()?;
                self.zero_block(t)?;
                wr32(inode, slot, t);
                *dirty = true;
            }
            self.map_indirect(t, level, idx, allocate)
        };

        let data = if logical < 12 {
            let slot = IN_BLOCK + logical as usize * 4;
            let b = rd32(&inode, slot);
            if b == 0 {
                if !allocate {
                    return Ok(0);
                }
                let nb = self.alloc_block()?;
                self.zero_block(nb)?;
                wr32(&mut inode, slot, nb);
                dirty = true;
                nb
            } else {
                b
            }
        } else if logical < 12 + ppb {
            indirect(IN_BLOCK + 12 * 4, 0, logical - 12, &mut inode, &mut dirty)?
        } else if logical < 12 + ppb + ppb * ppb {
            indirect(IN_BLOCK + 13 * 4, 1, logical - 12 - ppb, &mut inode, &mut dirty)?
        } else if logical < 12 + ppb + ppb * ppb + ppb * ppb * ppb {
            indirect(IN_BLOCK + 14 * 4, 2, logical - 12 - ppb - ppb * ppb, &mut inode, &mut dirty)?
        } else {
            return Err(VfsError::NoSpace);
        };

        if dirty {
            self.write_inode(ino, &inode)?;
        }
        Ok(data)
    }

    // ── allocation ───────────────────────────────────────────────────────────

    fn alloc_block(&self) -> Result<u32, VfsError> {
        let gd = self.read_gd(0)?;
        let bitmap_block = rd32(&gd, GD_BLOCK_BITMAP);
        if bitmap_block == 0 {
            return Err(VfsError::IOError);
        }
        let mut bitmap = self.read_block(bitmap_block)?;
        for i in 0..self.blocks_count as usize {
            let byte = i / 8;
            if byte >= bitmap.len() {
                break;
            }
            let bit = i % 8;
            if bitmap[byte] & (1 << bit) == 0 {
                bitmap[byte] |= 1 << bit;
                self.write_block(bitmap_block, &bitmap)?;

                let mut sb = self.read_super()?;
                dec32(&mut sb, SB_FREE_BLOCKS);
                self.write_super(&sb)?;

                let mut gd2 = self.read_gd(0)?;
                dec16(&mut gd2, GD_FREE_BLOCKS);
                self.write_gd(0, &gd2)?;
                return Ok(i as u32);
            }
        }
        Err(VfsError::NoSpace)
    }

    fn free_block(&self, block: u32) -> Result<(), VfsError> {
        if block == 0 {
            return Ok(());
        }
        let gd = self.read_gd(0)?;
        let bitmap_block = rd32(&gd, GD_BLOCK_BITMAP);
        let mut bitmap = self.read_block(bitmap_block)?;
        let i = block as usize;
        let byte = i / 8;
        let bit = i % 8;
        if byte < bitmap.len() && bitmap[byte] & (1 << bit) != 0 {
            bitmap[byte] &= !(1 << bit);
            self.write_block(bitmap_block, &bitmap)?;

            let mut sb = self.read_super()?;
            inc32(&mut sb, SB_FREE_BLOCKS);
            self.write_super(&sb)?;

            let mut gd2 = self.read_gd(0)?;
            inc16(&mut gd2, GD_FREE_BLOCKS);
            self.write_gd(0, &gd2)?;
        }
        Ok(())
    }

    fn alloc_inode(&self) -> Result<u64, VfsError> {
        let gd = self.read_gd(0)?;
        let bitmap_block = rd32(&gd, GD_INODE_BITMAP);
        if bitmap_block == 0 {
            return Err(VfsError::IOError);
        }
        let mut bitmap = self.read_block(bitmap_block)?;
        for n in 1..=self.inodes_count as usize {
            let i = n - 1;
            let byte = i / 8;
            if byte >= bitmap.len() {
                break;
            }
            let bit = i % 8;
            if bitmap[byte] & (1 << bit) == 0 {
                bitmap[byte] |= 1 << bit;
                self.write_block(bitmap_block, &bitmap)?;

                let mut sb = self.read_super()?;
                dec32(&mut sb, SB_FREE_INODES);
                self.write_super(&sb)?;

                let mut gd2 = self.read_gd(0)?;
                dec16(&mut gd2, GD_FREE_INODES);
                self.write_gd(0, &gd2)?;

                return Ok(n as u64);
            }
        }
        Err(VfsError::OutOfInodes)
    }

    fn free_inode(&self, ino: u64) -> Result<(), VfsError> {
        let gd = self.read_gd(0)?;
        let bitmap_block = rd32(&gd, GD_INODE_BITMAP);
        let mut bitmap = self.read_block(bitmap_block)?;
        let i = (ino - 1) as usize;
        let byte = i / 8;
        let bit = i % 8;
        if byte < bitmap.len() && bitmap[byte] & (1 << bit) != 0 {
            bitmap[byte] &= !(1 << bit);
            self.write_block(bitmap_block, &bitmap)?;

            let mut sb = self.read_super()?;
            inc32(&mut sb, SB_FREE_INODES);
            self.write_super(&sb)?;

            let mut gd2 = self.read_gd(0)?;
            inc16(&mut gd2, GD_FREE_INODES);
            self.write_gd(0, &gd2)?;
        }
        let zeros = vec![0u8; self.inode_size as usize];
        self.write_inode(ino, &zeros)?;
        Ok(())
    }

    /// Free every data block referenced by an inode (direct + indirect).
    fn free_inode_blocks(&self, inode: &[u8]) -> Result<(), VfsError> {
        let ppb = self.ppb() as usize;
        for i in 0..12 {
            let b = rd32(inode, IN_BLOCK + i * 4);
            if b != 0 {
                self.free_block(b)?;
            }
        }
        let s = rd32(inode, IN_BLOCK + 12 * 4);
        if s != 0 {
            let buf = self.read_block(s)?;
            for i in 0..ppb {
                let b = rd32(&buf, i * 4);
                if b != 0 {
                    self.free_block(b)?;
                }
            }
            self.free_block(s)?;
        }
        let d = rd32(inode, IN_BLOCK + 13 * 4);
        if d != 0 {
            let dbuf = self.read_block(d)?;
            for i in 0..ppb {
                let t = rd32(&dbuf, i * 4);
                if t != 0 {
                    let tbuf = self.read_block(t)?;
                    for j in 0..ppb {
                        let b = rd32(&tbuf, j * 4);
                        if b != 0 {
                            self.free_block(b)?;
                        }
                    }
                    self.free_block(t)?;
                }
            }
            self.free_block(d)?;
        }
        let t3 = rd32(inode, IN_BLOCK + 14 * 4);
        if t3 != 0 {
            let l1 = self.read_block(t3)?;
            for i in 0..ppb {
                let d2 = rd32(&l1, i * 4);
                if d2 != 0 {
                    let l2 = self.read_block(d2)?;
                    for j in 0..ppb {
                        let t = rd32(&l2, j * 4);
                        if t != 0 {
                            let l3 = self.read_block(t)?;
                            for k in 0..ppb {
                                let b = rd32(&l3, k * 4);
                                if b != 0 {
                                    self.free_block(b)?;
                                }
                            }
                            self.free_block(t)?;
                        }
                    }
                    self.free_block(d2)?;
                }
            }
            self.free_block(t3)?;
        }
        Ok(())
    }

    // ── directory helpers ────────────────────────────────────────────────────

    fn read_dir_data(&self, inode: u64) -> Result<Vec<u8>, VfsError> {
        let node = self.read_inode(inode)?;
        let size = self.inode_size_of(&node);
        let mut data = vec![0u8; size as usize];
        if size > 0 {
            self.read(inode, &mut data, 0)?;
        }
        Ok(data)
    }

    /// Write `buf` at `offset` into `inode` without the directory check.
    /// Used both for regular files and to update directory contents.
    fn write_data(&self, inode: u64, buf: &[u8], offset: u64) -> Result<usize, VfsError> {
        let bs = self.block_size as u64;
        let mut done = 0usize;
        while done < buf.len() {
            let cur = offset + done as u64;
            let logical = cur / bs;
            let in_blk = (cur % bs) as usize;
            let chunk = (buf.len() - done).min(bs as usize - in_blk);
            let block = self.map_block(inode, logical, true)?;
            let disk = block as u64 * bs + in_blk as u64;
            self.dev.write_at(disk, &buf[done..done + chunk])?;
            done += chunk;
        }
        // Re-read: `map_block` may have persisted new block pointers above.
        let end = offset + buf.len() as u64;
        let mut node = self.read_inode(inode)?;
        if end > self.inode_size_of(&node) {
            wr32(&mut node, IN_SIZE, end as u32);
            wr32(&mut node, IN_BLOCKS, (end / SECTOR_SIZE as u64) as u32);
            self.write_inode(inode, &node)?;
        }
        Ok(done)
    }

    /// Add `name` → `inode` to the directory `parent`.
    fn dir_add(&self, parent: u64, name: &str, inode: u32, ft: u8) -> Result<(), VfsError> {
        if name.is_empty() || name.len() > 255 {
            return Err(VfsError::InvalidPath);
        }
        let mut data = self.read_dir_data(parent)?;
        let need = align4(8 + name.len());
        let mut off = 0usize;
        while off + 8 <= data.len() {
            let rec = rd16(&data, off + 4) as usize;
            if rec == 0 {
                break;
            }
            let nlen = data[off + 6] as usize;
            let used = align4(8 + nlen);
            let ino_field = rd32(&data, off);
            if ino_field == 0 && rec >= need {
                set_dir_entry(&mut data, off, rec as u16, inode, name.as_bytes(), ft);
                self.write_data(parent, &data, 0)?;
                return Ok(());
            }
            if rec >= used + need {
                wr16(&mut data, off + 4, used as u16);
                let new_off = off + used;
                set_dir_entry(&mut data, new_off, (rec - used) as u16, inode, name.as_bytes(), ft);
                self.write_data(parent, &data, 0)?;
                return Ok(());
            }
            off += rec;
        }

        // No room: append a fresh block.
        let bs = self.block_size as usize;
        let mut block = vec![0u8; bs];
        set_dir_entry(&mut block, 0, bs as u16, inode, name.as_bytes(), ft);
        let size = self.inode_size_of(&self.read_inode(parent)?);
        self.write_data(parent, &block, size)?;
        Ok(())
    }

    /// Remove `name` from directory `parent`.
    fn dir_remove(&self, parent: u64, name: &str) -> Result<(), VfsError> {
        let mut data = self.read_dir_data(parent)?;
        let mut prev: Option<usize> = None;
        let mut off = 0usize;
        while off + 8 <= data.len() {
            let rec = rd16(&data, off + 4) as usize;
            if rec == 0 {
                break;
            }
            let nlen = data[off + 6] as usize;
            let ino = rd32(&data, off);
            if ino != 0 && 8 + nlen <= rec {
                if let Ok(n) = core::str::from_utf8(&data[off + 8..off + 8 + nlen]) {
                    if n == name {
                        if let Some(p) = prev {
                            let pre_rec = rd16(&data, p + 4) as usize;
                            wr16(&mut data, p + 4, (pre_rec + rec) as u16);
                        } else {
                            wr32(&mut data, off, 0);
                        }
                        self.write_data(parent, &data, 0)?;
                        return Ok(());
                    }
                }
            }
            prev = Some(off);
            off += rec;
        }
        Err(VfsError::FileNotFound)
    }

    // ── format ───────────────────────────────────────────────────────────────

    /// Create a fresh Ext2 filesystem on `dev` using 4096-byte blocks and a
    /// single block group.
    pub fn format(dev: Arc<dyn BlockDevice>, inodes_count: u32) -> Result<Self, VfsError> {
        const BS: u32 = 4096;
        let blocks_count = (dev.num_bytes() / BS as u64) as u32;
        if blocks_count < 24 {
            return Err(VfsError::NoSpace);
        }
        let inodes_count = inodes_count.max(16);
        let inode_size = 128u32;
        let inode_table_blocks = (inodes_count * inode_size).div_ceil(BS);
        let block_bitmap = 2u32;
        let inode_bitmap = 3u32;
        let inode_table = 4u32;
        let root_dir_block = inode_table + inode_table_blocks;
        let used_blocks = root_dir_block + 1;
        if used_blocks >= blocks_count {
            return Err(VfsError::NoSpace);
        }
        let free_blocks = blocks_count - used_blocks;
        const FIRST_INO: u32 = 11;
        let free_inodes = inodes_count.saturating_sub(FIRST_INO - 1);

        let fs = Self {
            dev,
            block_size: BS,
            inode_size,
            inodes_per_group: inodes_count,
            inodes_count,
            blocks_count,
            groups: 1,
        };

        // Superblock.
        let mut sb = vec![0u8; EXT2_SUPER_SIZE];
        wr32(&mut sb, SB_INODES_COUNT, inodes_count);
        wr32(&mut sb, SB_BLOCKS_COUNT, blocks_count);
        wr32(&mut sb, SB_FREE_BLOCKS, free_blocks);
        wr32(&mut sb, SB_FREE_INODES, free_inodes);
        wr32(&mut sb, SB_FIRST_DATA_BLOCK, 0);
        wr32(&mut sb, SB_LOG_BLOCK_SIZE, 2); // 1024 << 2 = 4096
        wr32(&mut sb, 0x1C, 2); // s_log_frag_size
        wr32(&mut sb, SB_BLOCKS_PER_GROUP, blocks_count);
        wr32(&mut sb, 0x24, blocks_count); // s_frags_per_group
        wr32(&mut sb, SB_INODES_PER_GROUP, inodes_count);
        wr16(&mut sb, 0x36, 0xFFFF); // s_max_mnt_count
        wr16(&mut sb, SB_MAGIC, EXT2_MAGIC);
        wr16(&mut sb, SB_STATE, 1); // cleanly unmounted
        wr16(&mut sb, 0x3C, 1); // s_errors = continue
        wr32(&mut sb, SB_REV_LEVEL, 1);
        wr32(&mut sb, SB_FIRST_INO, FIRST_INO);
        wr16(&mut sb, SB_INODE_SIZE, inode_size as u16);
        wr32(&mut sb, 0x60, 0x2); // s_feature_incompat = FILETYPE
        fs.write_super(&sb)?;

        // Group descriptor.
        let mut gd = [0u8; 32];
        wr32(&mut gd, GD_BLOCK_BITMAP, block_bitmap);
        wr32(&mut gd, GD_INODE_BITMAP, inode_bitmap);
        wr32(&mut gd, GD_INODE_TABLE, inode_table);
        wr16(&mut gd, GD_FREE_BLOCKS, free_blocks as u16);
        wr16(&mut gd, GD_FREE_INODES, free_inodes as u16);
        wr16(&mut gd, GD_USED_DIRS, 1);
        fs.write_gd(0, &gd)?;

        // Block bitmap: blocks 0..=root_dir_block are metadata/root.
        let mut bmap = vec![0u8; BS as usize];
        for b in 0..used_blocks {
            bmap[(b / 8) as usize] |= 1 << (b % 8);
        }
        fs.write_block(block_bitmap, &bmap)?;

        // Inode bitmap: inodes 1..=10 reserved.
        let mut imap = vec![0u8; BS as usize];
        for n in 0..(FIRST_INO - 1) {
            imap[(n / 8) as usize] |= 1 << (n % 8);
        }
        fs.write_block(inode_bitmap, &imap)?;

        for b in 0..inode_table_blocks {
            fs.zero_block(inode_table + b)?;
        }

        // Root inode (inode 2 → table index 1).
        let mut root = vec![0u8; inode_size as usize];
        wr16(&mut root, IN_MODE, S_IFDIR | 0o755);
        wr32(&mut root, IN_SIZE, BS);
        wr32(&mut root, IN_LINKS, 2);
        wr32(&mut root, IN_BLOCKS, BS / SECTOR_SIZE as u32);
        wr32(&mut root, IN_BLOCK, root_dir_block);
        let root_off = inode_table as u64 * BS as u64 + inode_size as u64;
        fs.dev.write_at(root_off, &root)?;

        // Root directory data.
        let mut dir = vec![0u8; BS as usize];
        set_dir_entry(&mut dir, 0, 12, EXT2_ROOT_INO as u32, b".", FT_DIR);
        set_dir_entry(&mut dir, 12, (BS as usize - 12) as u16, EXT2_ROOT_INO as u32, b"..", FT_DIR);
        fs.write_block(root_dir_block, &dir)?;
        fs.dev.flush()?;
        Ok(fs)
    }
}

impl FileSystem for Ext2Fs {
    fn read(&self, inode: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        let node = self.read_inode(inode)?;
        let size = self.inode_size_of(&node);
        if offset >= size {
            return Ok(0);
        }
        let bs = self.block_size as u64;
        let read_len = buf.len().min((size - offset) as usize);
        let mut done = 0usize;
        while done < read_len {
            let cur = offset + done as u64;
            let logical = cur / bs;
            let in_blk = (cur % bs) as usize;
            let chunk = (read_len - done).min(bs as usize - in_blk);
            let block = self.map_block(inode, logical, false)?;
            if block == 0 {
                for b in buf[done..done + chunk].iter_mut() {
                    *b = 0;
                }
            } else {
                let disk = block as u64 * bs + in_blk as u64;
                self.dev.read_at(disk, &mut buf[done..done + chunk])?;
            }
            done += chunk;
        }
        Ok(done)
    }

    fn write(&self, inode: u64, buf: &[u8], offset: u64) -> Result<usize, VfsError> {
        let node = self.read_inode(inode)?;
        if self.inode_is_dir(&node) {
            return Err(VfsError::IsADirectory);
        }
        self.write_data(inode, buf, offset)
    }

    fn lookup(
        &self,
        parent_inode: u64,
        name: &str,
        fs_arc: Arc<dyn FileSystem + Send + Sync>,
    ) -> Result<Vnode, VfsError> {
        let data = self.read_dir_data(parent_inode)?;
        match find_dir_entry(&data, name) {
            Some((ino, ft, _)) => {
                let size = self.inode_size_of(&self.read_inode(ino as u64)?);
                Ok(Vnode { inode: ino as u64, size, vtype: ft_to_vtype(ft), fs: fs_arc })
            }
            None => Err(VfsError::FileNotFound),
        }
    }

    fn readdir(&self, inode: u64) -> Result<Vec<DirEntry>, VfsError> {
        let data = self.read_dir_data(inode)?;
        Ok(parse_dir_entries(&data))
    }

    fn create(&self, parent_inode: u64, name: &str) -> Result<u64, VfsError> {
        let parent = self.read_inode(parent_inode)?;
        if !self.inode_is_dir(&parent) {
            return Err(VfsError::NotADirectory);
        }
        let data = self.read_dir_data(parent_inode)?;
        if find_dir_entry(&data, name).is_some() {
            return Err(VfsError::AlreadyExists);
        }

        let ino = self.alloc_inode()?;
        let mut node = vec![0u8; self.inode_size as usize];
        wr16(&mut node, IN_MODE, S_IFREG | 0o644);
        wr32(&mut node, IN_LINKS, 1);
        self.write_inode(ino, &node)?;
        self.dir_add(parent_inode, name, ino as u32, FT_REGULAR)?;
        Ok(ino)
    }

    fn mkdir(&self, parent_inode: u64, name: &str) -> Result<u64, VfsError> {
        let parent = self.read_inode(parent_inode)?;
        if !self.inode_is_dir(&parent) {
            return Err(VfsError::NotADirectory);
        }
        let data = self.read_dir_data(parent_inode)?;
        if find_dir_entry(&data, name).is_some() {
            return Err(VfsError::AlreadyExists);
        }

        let ino = self.alloc_inode()?;
        let block = self.alloc_block()?;
        let bs = self.block_size as usize;
        let mut dir = vec![0u8; bs];
        set_dir_entry(&mut dir, 0, 12, ino as u32, b".", FT_DIR);
        set_dir_entry(&mut dir, 12, (bs - 12) as u16, parent_inode as u32, b"..", FT_DIR);
        self.write_block(block, &dir)?;

        let mut node = vec![0u8; self.inode_size as usize];
        wr16(&mut node, IN_MODE, S_IFDIR | 0o755);
        wr32(&mut node, IN_SIZE, self.block_size);
        wr32(&mut node, IN_LINKS, 2);
        wr32(&mut node, IN_BLOCKS, self.block_size / SECTOR_SIZE as u32);
        wr32(&mut node, IN_BLOCK, block);
        self.write_inode(ino, &node)?;

        self.dir_add(parent_inode, name, ino as u32, FT_DIR)?;
        let mut parent = self.read_inode(parent_inode)?;
        inc32(&mut parent, IN_LINKS);
        self.write_inode(parent_inode, &parent)?;

        let mut gd = self.read_gd(0)?;
        inc16(&mut gd, GD_USED_DIRS);
        self.write_gd(0, &gd)?;
        Ok(ino)
    }

    fn unlink(&self, parent_inode: u64, name: &str) -> Result<(), VfsError> {
        let data = self.read_dir_data(parent_inode)?;
        let (ino, ft, _) = find_dir_entry(&data, name).ok_or(VfsError::FileNotFound)?;
        let ino = ino as u64;

        if ft == FT_DIR {
            if !self.readdir(ino)?.is_empty() {
                return Err(VfsError::NotSupported);
            }
            let mut parent = self.read_inode(parent_inode)?;
            dec32(&mut parent, IN_LINKS);
            self.write_inode(parent_inode, &parent)?;
            let mut gd = self.read_gd(0)?;
            dec16(&mut gd, GD_USED_DIRS);
            self.write_gd(0, &gd)?;
        }

        let node = self.read_inode(ino)?;
        self.free_inode_blocks(&node)?;
        self.free_inode(ino)?;
        self.dir_remove(parent_inode, name)?;
        Ok(())
    }

    fn get_root_inode(&self) -> u64 {
        EXT2_ROOT_INO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemBlockDevice;

    fn fresh() -> Arc<Ext2Fs> {
        // 8 MiB device → 2048 blocks of 4096 bytes.
        let dev = Arc::new(MemBlockDevice::new(8 * 1024 * 1024 / SECTOR_SIZE));
        Arc::new(Ext2Fs::format(dev, 128).unwrap())
    }

    #[test]
    fn format_and_root() {
        let fs = fresh();
        assert_eq!(fs.get_root_inode(), EXT2_ROOT_INO);
        let entries = fs.readdir(EXT2_ROOT_INO).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn create_write_read() {
        let fs = fresh();
        let ino = fs.create(EXT2_ROOT_INO, "hello.txt").unwrap();
        fs.write(ino, b"Hello, Ext2!\n", 0).unwrap();
        let mut buf = [0u8; 32];
        let n = fs.read(ino, &mut buf, 0).unwrap();
        assert_eq!(&buf[..n], b"Hello, Ext2!\n");
    }

    #[test]
    fn lookup_and_readdir() {
        let fs = fresh();
        let ino = fs.create(EXT2_ROOT_INO, "a.txt").unwrap();
        fs.write(ino, b"abc", 0).unwrap();
        let entries = fs.readdir(EXT2_ROOT_INO).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].inode, ino);

        let arc: Arc<dyn FileSystem + Send + Sync> = fs.clone();
        let vnode = fs.lookup(EXT2_ROOT_INO, "a.txt", arc).unwrap();
        assert_eq!(vnode.inode, ino);
        assert_eq!(vnode.size, 3);
        assert_eq!(vnode.vtype, VnodeType::File);
        assert_eq!(
            fs.lookup(EXT2_ROOT_INO, "missing", fs.clone()).err(),
            Some(VfsError::FileNotFound)
        );
    }

    #[test]
    fn large_file_uses_indirect_blocks() {
        let fs = fresh();
        let ino = fs.create(EXT2_ROOT_INO, "big.bin").unwrap();
        // 5 MiB spans direct (48 KiB) + single indirect (4 MiB) + double.
        let mut data = vec![0u8; 5 * 1024 * 1024];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        fs.write(ino, &data, 0).unwrap();
        let mut out = vec![0u8; data.len()];
        let n = fs.read(ino, &mut out, 0).unwrap();
        assert_eq!(n, data.len());
        assert_eq!(out, data);
    }

    #[test]
    fn sparse_holes_read_zero() {
        let fs = fresh();
        let ino = fs.create(EXT2_ROOT_INO, "sparse").unwrap();
        fs.write(ino, b"tail", 3 * 4096 + 5).unwrap();
        let mut buf = [1u8; 16];
        let n = fs.read(ino, &mut buf, 0).unwrap();
        assert_eq!(n, 16);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn mkdir_and_unlink() {
        let fs = fresh();
        let dir = fs.mkdir(EXT2_ROOT_INO, "sub").unwrap();
        let file = fs.create(dir, "inner.txt").unwrap();
        fs.write(file, b"inner", 0).unwrap();
        assert_eq!(fs.readdir(dir).unwrap().len(), 1);

        // Non-empty directory cannot be unlinked.
        assert_eq!(fs.unlink(EXT2_ROOT_INO, "sub"), Err(VfsError::NotSupported));
        fs.unlink(dir, "inner.txt").unwrap();
        assert!(fs.readdir(dir).unwrap().is_empty());
        fs.unlink(EXT2_ROOT_INO, "sub").unwrap();
        assert!(fs.readdir(EXT2_ROOT_INO).unwrap().is_empty());
    }

    #[test]
    fn duplicate_create_errors() {
        let fs = fresh();
        fs.create(EXT2_ROOT_INO, "dup").unwrap();
        assert_eq!(fs.create(EXT2_ROOT_INO, "dup"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn persistence_across_reopen() {
        let dev: Arc<dyn BlockDevice> =
            Arc::new(MemBlockDevice::new(8 * 1024 * 1024 / SECTOR_SIZE));
        {
            let fs = Ext2Fs::format(dev.clone(), 128).unwrap();
            let ino = fs.create(EXT2_ROOT_INO, "persist.txt").unwrap();
            fs.write(ino, b"durable", 0).unwrap();
        }
        let fs = Ext2Fs::from_device(dev).unwrap();
        let data = fs.read_dir_data(EXT2_ROOT_INO).unwrap();
        let (ino, _, _) = find_dir_entry(&data, "persist.txt").unwrap();
        let mut buf = [0u8; 16];
        let n = fs.read(ino as u64, &mut buf, 0).unwrap();
        assert_eq!(&buf[..n], b"durable");
    }
}
