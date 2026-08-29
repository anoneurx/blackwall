extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::vfs::{DirEntry, FileSystem, VfsError, Vnode, VnodeType};

// ── On-disk structures (RFC 3010 / Ext2 spec) ────────────────────────────────

/// Ext2 superblock — lives at byte offset 1024 in the partition image.
/// Fields use little-endian byte order.
///
/// # Safety
/// All field accesses on this `#[repr(C, packed)]` struct MUST use
/// `core::ptr::read_unaligned` because the compiler cannot guarantee
/// natural alignment when the struct is cast from a raw byte slice.
#[repr(C, packed)]
pub struct Superblock {
    pub s_inodes_count: u32,      // 0x00
    pub s_blocks_count: u32,      // 0x04
    pub s_r_blocks_count: u32,    // 0x08
    pub s_free_blocks_count: u32, // 0x0C
    pub s_free_inodes_count: u32, // 0x10
    pub s_first_data_block: u32,  // 0x14
    pub s_log_block_size: u32,    // 0x18  (block_size = 1024 << s_log_block_size)
    pub s_log_frag_size: u32,     // 0x1C
    pub s_blocks_per_group: u32,  // 0x20
    pub s_frags_per_group: u32,   // 0x24
    pub s_inodes_per_group: u32,  // 0x28
    pub s_mtime: u32,             // 0x2C
    pub s_wtime: u32,             // 0x30
    pub s_mnt_count: u16,         // 0x34
    pub s_max_mnt_count: u16,     // 0x36
    pub s_magic: u16,             // 0x38  must be 0xEF53
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
    pub i_mode: u16,        // 0x00
    pub i_uid: u16,         // 0x02
    pub i_size: u32,        // 0x04
    pub i_atime: u32,       // 0x08
    pub i_ctime: u32,       // 0x0C
    pub i_mtime: u32,       // 0x10
    pub i_dtime: u32,       // 0x14
    pub i_gid: u16,         // 0x18
    pub i_links_count: u16, // 0x1A
    pub i_blocks: u32,      // 0x1C
    pub i_flags: u32,       // 0x20
    pub i_osd1: u32,        // 0x24
    pub i_block: [u32; 15], // 0x28  direct[0..11], indirect[12..14]
    pub i_generation: u32,
    pub i_file_acl: u32,
    pub i_dir_acl: u32,
    pub i_faddr: u32,
    pub i_osd2: [u8; 12],
}

// ── Ext2Fs ───────────────────────────────────────────────────────────────────

/// Read-only Ext2 filesystem driver backed by an in-memory byte image.
///
/// Write support (`create`, `mkdir`, `unlink`, `write`) returns
/// `VfsError::NotSupported` — Ext2 write support is planned for a future phase.
pub struct Ext2Fs {
    image: Vec<u8>,
}

impl Ext2Fs {
    pub fn new(image: Vec<u8>) -> Self {
        Self { image }
    }

    /// Parse and validate the superblock. Returns `None` if the magic is wrong
    /// or the image is too small.
    pub fn get_superblock(&self) -> Option<&Superblock> {
        if self.image.len() < 2048 {
            return None;
        }
        // SAFETY: We verify the image is large enough above and only read the
        // magic field via read_unaligned to avoid misaligned access UB.
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

    /// Read the raw bytes of a single disk block into `buf`.
    #[allow(dead_code)]
    fn read_block(&self, block: u32, buf: &mut [u8]) -> Result<(), VfsError> {
        let bs = self.block_size().ok_or(VfsError::IOError)? as usize;
        let offset = block as usize * bs;
        if self.image.len() < offset + bs.min(buf.len()) {
            return Err(VfsError::IOError);
        }
        let len = buf.len().min(bs);
        buf[..len].copy_from_slice(&self.image[offset..offset + len]);
        Ok(())
    }

    /// Retrieve the raw inode for a given 1-based inode number.
    fn get_inode(&self, inode: u64) -> Result<&Inode, VfsError> {
        let sb = self.get_superblock().ok_or(VfsError::IOError)?;
        let bs = self.block_size().ok_or(VfsError::IOError)? as usize;
        let inodes_per_group =
            unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(sb.s_inodes_per_group)) };
        let group = ((inode - 1) / inodes_per_group as u64) as usize;
        let index = ((inode - 1) % inodes_per_group as u64) as usize;

        let bg_offset = if bs == 1024 { 2048 } else { bs };
        if self.image.len() < bg_offset + (group + 1) * 32 {
            return Err(VfsError::IOError);
        }

        // SAFETY: packed struct; read_unaligned used for all field access below.
        let bg_ptr = unsafe {
            self.image.as_ptr().add(bg_offset + group * 32) as *const BlockGroupDescriptor
        };
        let inode_table_block =
            unsafe { core::ptr::read_unaligned(core::ptr::addr_of!((*bg_ptr).bg_inode_table)) };
        let inode_table_off = inode_table_block as usize * bs;
        const INODE_SIZE: usize = 128;

        if self.image.len() < inode_table_off + (index + 1) * INODE_SIZE {
            return Err(VfsError::IOError);
        }

        // SAFETY: aligned check done above; packed fields accessed via addr_of!.
        Ok(unsafe {
            &*(self.image.as_ptr().add(inode_table_off + index * INODE_SIZE) as *const Inode)
        })
    }
}

impl FileSystem for Ext2Fs {
    fn read(&self, inode: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        let bs = self.block_size().ok_or(VfsError::IOError)?;
        let inode_ref = self.get_inode(inode)?;

        // SAFETY: packed struct fields — read_unaligned required.
        let i_size = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(inode_ref.i_size)) };
        let i_block: [u32; 15] =
            unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(inode_ref.i_block)) };

        if offset >= i_size as u64 {
            return Ok(0);
        }
        let read_len = buf.len().min((i_size as u64 - offset) as usize);
        let mut done = 0;

        while done < read_len {
            let cur_off = offset + done as u64;
            let block_idx = (cur_off / bs) as usize;
            let in_blk_off = (cur_off % bs) as usize;

            if block_idx >= 12 {
                return Err(VfsError::NotSupported); // indirect blocks not implemented yet
            }

            let disk_block = i_block[block_idx] as usize;
            let disk_off = disk_block * bs as usize + in_blk_off;
            let chunk = (read_len - done).min(bs as usize - in_blk_off);

            if self.image.len() < disk_off + chunk {
                return Err(VfsError::IOError);
            }
            buf[done..done + chunk].copy_from_slice(&self.image[disk_off..disk_off + chunk]);
            done += chunk;
        }
        Ok(done)
    }

    /// Ext2 write is not implemented (read-only driver).
    fn write(&self, _inode: u64, _buf: &[u8], _offset: u64) -> Result<usize, VfsError> {
        Err(VfsError::NotSupported)
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
