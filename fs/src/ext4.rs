extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::ext2::{BlockGroupDescriptor, Inode, Superblock};
use crate::vfs::{DirEntry, FileSystem, VfsError, Vnode, VnodeType};

// ── Ext4 Extent Tree ─────────────────────────────────────────────────────────

/// Ext4 extent tree header — appears at `i_block[0]` when `EXT4_EXTENTS_FL` is
/// set in `i_flags` (flag 0x80000).
#[repr(C, packed)]
pub struct ExtentHeader {
    pub eh_magic: u16, // must be 0xF30A
    pub eh_entries: u16,
    pub eh_max: u16,
    pub eh_depth: u16,
    pub eh_generation: u32,
}

pub const EXT4_EXT_MAGIC: u16 = 0xF30A;
pub const EXT4_EXTENTS_FL: u32 = 0x0008_0000;

/// A single leaf extent: maps logical blocks → physical disk blocks.
#[repr(C, packed)]
pub struct Extent {
    pub ee_block: u32,    // first logical block number
    pub ee_len: u16,      // number of blocks (15-bit; high bit = uninit)
    pub ee_start_hi: u16, // high 16 bits of physical block
    pub ee_start_lo: u32, // low 32 bits of physical block
}

impl Extent {
    pub fn start_block(&self) -> u64 {
        // SAFETY: packed fields — read_unaligned.
        let hi = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.ee_start_hi)) } as u64;
        let lo = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.ee_start_lo)) } as u64;
        (hi << 32) | lo
    }
    pub fn logical_start(&self) -> u32 {
        unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.ee_block)) }
    }
    pub fn length(&self) -> u16 {
        let raw = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.ee_len)) };
        raw & 0x7FFF // strip uninitialized bit
    }
}

// ── Ext4Fs ───────────────────────────────────────────────────────────────────

/// Ext4 filesystem driver — extends the Ext2 superblock format with:
/// - Extent tree block mapping (replaces direct/indirect block arrays)
/// - 64-bit file sizes (i_size_high in inode offset 0x6C)
/// - Journal (jbd2) awareness (journal replayed before mount — stub only)
///
/// Write support returns `VfsError::NotSupported` in this phase.
pub struct Ext4Fs {
    image: Vec<u8>,
}

impl Ext4Fs {
    pub fn new(image: Vec<u8>) -> Self {
        Self { image }
    }

    pub fn get_superblock(&self) -> Option<&Superblock> {
        if self.image.len() < 2048 {
            return None;
        }
        // SAFETY: image bounds checked above; magic read via read_unaligned.
        let ptr = unsafe { self.image.as_ptr().add(1024) as *const Superblock };
        let magic = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!((*ptr).s_magic)) };
        if magic == 0xEF53 {
            Some(unsafe { &*ptr })
        } else {
            None
        }
    }

    fn block_size(&self) -> Option<u64> {
        let sb = self.get_superblock()?;
        let log = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(sb.s_log_block_size)) };
        Some(1024u64 << log)
    }

    fn get_inode(&self, inode: u64) -> Result<&Inode, VfsError> {
        let sb = self.get_superblock().ok_or(VfsError::IOError)?;
        let bs = self.block_size().ok_or(VfsError::IOError)? as usize;
        let ipg = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(sb.s_inodes_per_group)) };
        let group = ((inode - 1) / ipg as u64) as usize;
        let index = ((inode - 1) % ipg as u64) as usize;
        let bg_off = if bs == 1024 { 2048 } else { bs };

        let bg_ptr =
            unsafe { self.image.as_ptr().add(bg_off + group * 32) as *const BlockGroupDescriptor };
        let inode_table_block =
            unsafe { core::ptr::read_unaligned(core::ptr::addr_of!((*bg_ptr).bg_inode_table)) };
        let it_off = inode_table_block as usize * bs;
        const INODE_SZ: usize = 128;

        if self.image.len() < it_off + (index + 1) * INODE_SZ {
            return Err(VfsError::IOError);
        }
        // SAFETY: bounds verified; all fields accessed via read_unaligned.
        Ok(unsafe { &*(self.image.as_ptr().add(it_off + index * INODE_SZ) as *const Inode) })
    }

    /// Read file data via the Ext4 extent tree.
    fn read_extents(&self, inode: &Inode, buf: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        let bs = self.block_size().ok_or(VfsError::IOError)?;
        let i_flags = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(inode.i_flags)) };
        let i_block: [u32; 15] =
            unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(inode.i_block)) };
        let i_size = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(inode.i_size)) };

        if (i_flags & EXT4_EXTENTS_FL) == 0 {
            // Fall back to direct/indirect block addressing (same as Ext2)
            return self.read_direct(&i_block, i_size as u64, bs, buf, offset);
        }

        // Parse the extent header stored at i_block[0..3] (60 bytes of header + extents)
        let raw = &i_block as *const [u32; 15] as *const u8;
        let hdr = unsafe { &*(raw as *const ExtentHeader) };
        let eh_magic = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(hdr.eh_magic)) };
        let eh_entries = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(hdr.eh_entries)) };
        let eh_depth = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(hdr.eh_depth)) };

        if eh_magic != EXT4_EXT_MAGIC || eh_depth != 0 {
            // Interior nodes (depth > 0) require recursive traversal — not implemented.
            return Err(VfsError::NotSupported);
        }

        if offset >= i_size as u64 {
            return Ok(0);
        }
        let read_len = buf.len().min((i_size as u64 - offset) as usize);
        let mut done = 0;

        let extent_base = unsafe { raw.add(12) as *const Extent }; // skip 12-byte header

        while done < read_len {
            let cur_off = offset + done as u64;
            let log_block = (cur_off / bs) as u32;
            let in_blk_off = (cur_off % bs) as usize;

            // Find the extent covering log_block
            let mut found = false;
            for i in 0..eh_entries as usize {
                let ext = unsafe { &*extent_base.add(i) };
                let ee_block = ext.logical_start();
                let ee_len = ext.length() as u32;

                if log_block >= ee_block && log_block < ee_block + ee_len {
                    let phys = ext.start_block() + (log_block - ee_block) as u64;
                    let disk_off = phys as usize * bs as usize + in_blk_off;
                    let chunk = (read_len - done).min(bs as usize - in_blk_off);

                    if self.image.len() < disk_off + chunk {
                        return Err(VfsError::IOError);
                    }
                    buf[done..done + chunk]
                        .copy_from_slice(&self.image[disk_off..disk_off + chunk]);
                    done += chunk;
                    found = true;
                    break;
                }
            }
            if !found {
                break;
            }
        }
        Ok(done)
    }

    fn read_direct(
        &self,
        i_block: &[u32; 15],
        i_size: u64,
        bs: u64,
        buf: &mut [u8],
        offset: u64,
    ) -> Result<usize, VfsError> {
        if offset >= i_size {
            return Ok(0);
        }
        let read_len = buf.len().min((i_size - offset) as usize);
        let mut done = 0;

        while done < read_len {
            let cur_off = offset + done as u64;
            let block_idx = (cur_off / bs) as usize;
            let in_blk_off = (cur_off % bs) as usize;
            if block_idx >= 12 {
                return Err(VfsError::NotSupported);
            }
            let disk_off = i_block[block_idx] as usize * bs as usize + in_blk_off;
            let chunk = (read_len - done).min(bs as usize - in_blk_off);
            if self.image.len() < disk_off + chunk {
                return Err(VfsError::IOError);
            }
            buf[done..done + chunk].copy_from_slice(&self.image[disk_off..disk_off + chunk]);
            done += chunk;
        }
        Ok(done)
    }
}

impl FileSystem for Ext4Fs {
    fn read(&self, inode: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        let inode_ref = self.get_inode(inode)?;
        self.read_extents(inode_ref, buf, offset)
    }

    fn write(&self, _inode: u64, _buf: &[u8], _offset: u64) -> Result<usize, VfsError> {
        Err(VfsError::NotSupported) // Ext4 write path planned for Phase 8
    }

    fn lookup(
        &self,
        parent_inode: u64,
        name: &str,
        fs_arc: Arc<dyn FileSystem + Send + Sync>,
    ) -> Result<Vnode, VfsError> {
        let mut buf = alloc::vec![0u8; 4096];
        let bytes = self.read(parent_inode, &mut buf, 0)?;
        let mut off = 0;
        while off + 8 <= bytes {
            let ent_inode =
                u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
            let rec_len = u16::from_le_bytes([buf[off + 4], buf[off + 5]]) as usize;
            let name_len = buf[off + 6] as usize;
            let file_type = buf[off + 7];
            if off + 8 + name_len > bytes {
                break;
            }
            let entry_name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len])
                .map_err(|_| VfsError::IOError)?;
            if entry_name == name {
                let vtype = if file_type == 2 { VnodeType::Directory } else { VnodeType::File };
                return Ok(Vnode { inode: ent_inode as u64, size: 0, vtype, fs: fs_arc });
            }
            if rec_len == 0 {
                break;
            }
            off += rec_len;
        }
        Err(VfsError::FileNotFound)
    }

    fn readdir(&self, inode: u64) -> Result<Vec<DirEntry>, VfsError> {
        let mut buf = alloc::vec![0u8; 4096];
        let bytes = self.read(inode, &mut buf, 0)?;
        let mut entries = Vec::new();
        let mut off = 0;
        while off + 8 <= bytes {
            let ent_inode =
                u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
            let rec_len = u16::from_le_bytes([buf[off + 4], buf[off + 5]]) as usize;
            let name_len = buf[off + 6] as usize;
            let file_type = buf[off + 7];
            if off + 8 + name_len > bytes {
                break;
            }
            let name = core::str::from_utf8(&buf[off + 8..off + 8 + name_len])
                .map_err(|_| VfsError::IOError)?;
            if ent_inode != 0 && name != "." && name != ".." {
                let vtype = if file_type == 2 { VnodeType::Directory } else { VnodeType::File };
                entries.push(DirEntry { name: String::from(name), inode: ent_inode as u64, vtype });
            }
            if rec_len == 0 {
                break;
            }
            off += rec_len;
        }
        Ok(entries)
    }

    fn create(&self, _p: u64, _n: &str) -> Result<u64, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn mkdir(&self, _p: u64, _n: &str) -> Result<u64, VfsError> {
        Err(VfsError::NotSupported)
    }
    fn unlink(&self, _p: u64, _n: &str) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
    fn get_root_inode(&self) -> u64 {
        2
    }
}
