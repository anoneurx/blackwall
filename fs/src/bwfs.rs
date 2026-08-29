//! # BWFS — Black Wall Native Filesystem
//!
//! A lightweight, flat key-value store filesystem designed for the Black Wall
//! Server package layer. Unlike Ext2/Ext4, BWFS does not have a block-based
//! on-disk layout. Instead it stores all metadata in a compact header table
//! followed by contiguous data blobs — optimised for small package archives
//! and read-mostly workloads (similar to CPIO or TAR without compression).
//!
//! ## On-disk Format
//!
//! ```text
//! Offset  Length  Field
//! 0       4       Magic: b"BWFS"
//! 4       4       Version: u32 LE (currently 1)
//! 8       4       Entry count: u32 LE
//! 12      N*64    Entry table (each entry = 64 bytes, see BwfsEntry)
//! 12+N*64 ...     Raw data blobs (concatenated, byte-aligned)
//! ```
//!
//! ## Entry Layout (64 bytes)
//!
//! ```text
//! 0   48   null-padded UTF-8 name
//! 48   8   data_offset: u64 LE (absolute from start of file)
//! 56   8   data_length: u64 LE
//! ```

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::vfs::{DirEntry, FileSystem, VfsError, Vnode, VnodeType};

// ── Constants ────────────────────────────────────────────────────────────────

pub const BWFS_MAGIC: &[u8; 4] = b"BWFS";
pub const BWFS_VERSION: u32 = 1;
pub const ENTRY_SIZE: usize = 64;
pub const NAME_SIZE: usize = 48;
pub const HEADER_BASE: usize = 12;

static NEXT_INODE: AtomicU64 = AtomicU64::new(100);
fn alloc_inode() -> u64 {
    NEXT_INODE.fetch_add(1, Ordering::Relaxed)
}

// ── In-memory entry ──────────────────────────────────────────────────────────

struct BwNode {
    inode: u64,
    name: String,
    is_dir: bool,
    data: Vec<u8>,
}

// ── BwFs ─────────────────────────────────────────────────────────────────────

/// Black Wall native filesystem. Can be loaded from a serialized image or
/// constructed in-memory and then serialized out.
pub struct BwFs {
    nodes: core::cell::UnsafeCell<Vec<BwNode>>,
}

// SAFETY: kernel always wraps in SpinLock before sharing across cores.
unsafe impl Send for BwFs {}
unsafe impl Sync for BwFs {}

impl BwFs {
    /// Create an empty BwFs with a root directory.
    pub fn new() -> Self {
        let root = BwNode { inode: 1, name: String::new(), is_dir: true, data: Vec::new() };
        Self { nodes: core::cell::UnsafeCell::new(Vec::from([root])) }
    }

    /// Deserialize a BWFS image from a raw byte slice.
    pub fn from_image(image: &[u8]) -> Option<Self> {
        if image.len() < 12 || &image[0..4] != BWFS_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes([image[4], image[5], image[6], image[7]]);
        if version != BWFS_VERSION {
            return None;
        }
        let count = u32::from_le_bytes([image[8], image[9], image[10], image[11]]) as usize;

        let fs = BwFs::new();
        for i in 0..count {
            let off = HEADER_BASE + i * ENTRY_SIZE;
            if off + ENTRY_SIZE > image.len() {
                break;
            }

            let name_bytes = &image[off..off + NAME_SIZE];
            let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(NAME_SIZE);
            let name = core::str::from_utf8(&name_bytes[..name_end]).ok()?;

            let data_off =
                u64::from_le_bytes(image[off + NAME_SIZE..off + NAME_SIZE + 8].try_into().ok()?)
                    as usize;
            let data_len = u64::from_le_bytes(
                image[off + NAME_SIZE + 8..off + NAME_SIZE + 16].try_into().ok()?,
            ) as usize;

            if data_off + data_len > image.len() {
                break;
            }
            let data = image[data_off..data_off + data_len].to_vec();
            let inode = alloc_inode();
            fs.nodes_mut().push(BwNode { inode, name: String::from(name), is_dir: false, data });
        }
        Some(fs)
    }

    /// Serialize the filesystem to a binary image (BWFS format).
    pub fn to_image(&self) -> Vec<u8> {
        let nodes = self.nodes();
        let files: Vec<&BwNode> = nodes.iter().filter(|n| !n.is_dir).collect();
        let count = files.len() as u32;

        let data_start = HEADER_BASE + files.len() * ENTRY_SIZE;
        let total_data: usize = files.iter().map(|n| n.data.len()).sum();
        let mut out = Vec::with_capacity(data_start + total_data);

        // Magic + version + count
        out.extend_from_slice(BWFS_MAGIC);
        out.extend_from_slice(&BWFS_VERSION.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());

        // Entry table
        let mut cursor = data_start;
        for node in &files {
            let mut name_buf = [0u8; NAME_SIZE];
            let name_bytes = node.name.as_bytes();
            let n = name_bytes.len().min(NAME_SIZE);
            name_buf[..n].copy_from_slice(&name_bytes[..n]);
            out.extend_from_slice(&name_buf);
            out.extend_from_slice(&(cursor as u64).to_le_bytes());
            out.extend_from_slice(&(node.data.len() as u64).to_le_bytes());
            cursor += node.data.len();
        }

        // Data blobs
        for node in &files {
            out.extend_from_slice(&node.data);
        }
        out
    }

    fn nodes(&self) -> &Vec<BwNode> {
        unsafe { &*self.nodes.get() }
    }
    fn nodes_mut(&self) -> &mut Vec<BwNode> {
        unsafe { &mut *self.nodes.get() }
    }
}

impl FileSystem for BwFs {
    fn read(&self, inode: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError> {
        let nodes = self.nodes();
        let node = nodes.iter().find(|n| n.inode == inode).ok_or(VfsError::FileNotFound)?;
        if node.is_dir {
            return Err(VfsError::IsADirectory);
        }
        if offset >= node.data.len() as u64 {
            return Ok(0);
        }
        let start = offset as usize;
        let n = buf.len().min(node.data.len() - start);
        buf[..n].copy_from_slice(&node.data[start..start + n]);
        Ok(n)
    }

    fn write(&self, inode: u64, buf: &[u8], offset: u64) -> Result<usize, VfsError> {
        let nodes = self.nodes_mut();
        let node = nodes.iter_mut().find(|n| n.inode == inode).ok_or(VfsError::FileNotFound)?;
        if node.is_dir {
            return Err(VfsError::IsADirectory);
        }
        let start = offset as usize;
        if start + buf.len() > node.data.len() {
            node.data.resize(start + buf.len(), 0);
        }
        node.data[start..start + buf.len()].copy_from_slice(buf);
        Ok(buf.len())
    }

    fn lookup(
        &self,
        _parent: u64,
        name: &str,
        fs_arc: Arc<dyn FileSystem + Send + Sync>,
    ) -> Result<Vnode, VfsError> {
        // BWFS is flat — all files live at the root, directories are virtual
        let nodes = self.nodes();
        let node = nodes.iter().find(|n| n.name == name).ok_or(VfsError::FileNotFound)?;
        Ok(Vnode {
            inode: node.inode,
            size: node.data.len() as u64,
            vtype: if node.is_dir { VnodeType::Directory } else { VnodeType::File },
            fs: fs_arc,
        })
    }

    fn readdir(&self, _inode: u64) -> Result<Vec<DirEntry>, VfsError> {
        let nodes = self.nodes();
        Ok(nodes
            .iter()
            .filter(|n| n.inode != 1) // exclude root sentinel
            .map(|n| DirEntry {
                name: n.name.clone(),
                inode: n.inode,
                vtype: if n.is_dir { VnodeType::Directory } else { VnodeType::File },
            })
            .collect())
    }

    fn create(&self, _parent_inode: u64, name: &str) -> Result<u64, VfsError> {
        if self.nodes().iter().any(|n| n.name == name) {
            return Err(VfsError::AlreadyExists);
        }
        let inode = alloc_inode();
        self.nodes_mut().push(BwNode {
            inode,
            name: String::from(name),
            is_dir: false,
            data: Vec::new(),
        });
        Ok(inode)
    }

    fn mkdir(&self, _parent: u64, name: &str) -> Result<u64, VfsError> {
        if self.nodes().iter().any(|n| n.name == name) {
            return Err(VfsError::AlreadyExists);
        }
        let inode = alloc_inode();
        self.nodes_mut().push(BwNode {
            inode,
            name: String::from(name),
            is_dir: true,
            data: Vec::new(),
        });
        Ok(inode)
    }

    fn unlink(&self, _parent: u64, name: &str) -> Result<(), VfsError> {
        let nodes = self.nodes_mut();
        let idx =
            nodes.iter().position(|n| n.name == name && !n.is_dir).ok_or(VfsError::FileNotFound)?;
        nodes.remove(idx);
        Ok(())
    }

    fn get_root_inode(&self) -> u64 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_fs() -> BwFs {
        BwFs::new()
    }

    #[test]
    fn new_fs_has_root() {
        let fs = new_fs();
        assert_eq!(fs.get_root_inode(), 1);
    }

    #[test]
    fn create_and_read_file() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        let inode = fs.create(root, "test.txt").unwrap();
        fs.write(inode, b"hello world", 0).unwrap();

        let mut buf = [0u8; 32];
        let n = fs.read(inode, &mut buf, 0).unwrap();
        assert_eq!(&buf[..n], b"hello world");
    }

    #[test]
    fn create_duplicate_file_returns_error() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        fs.create(root, "file.txt").unwrap();
        assert_eq!(fs.create(root, "file.txt"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn read_nonexistent_inode() {
        let fs = new_fs();
        let mut buf = [0u8; 10];
        assert_eq!(fs.read(999, &mut buf, 0), Err(VfsError::FileNotFound));
    }

    #[test]
    fn read_offset_beyond_data() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        let inode = fs.create(root, "short").unwrap();
        fs.write(inode, b"ab", 0).unwrap();
        let mut buf = [0u8; 10];
        assert_eq!(fs.read(inode, &mut buf, 100), Ok(0));
    }

    #[test]
    fn write_extends_file() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        let inode = fs.create(root, "grow").unwrap();
        fs.write(inode, b"aaa", 0).unwrap();
        fs.write(inode, b"bbb", 3).unwrap();
        let mut buf = [0u8; 16];
        let n = fs.read(inode, &mut buf, 0).unwrap();
        assert_eq!(&buf[..n], b"aaabbb");
    }

    #[test]
    fn mkdir_and_readdir() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        fs.mkdir(root, "subdir").unwrap();
        fs.create(root, "file.txt").unwrap();

        let entries = fs.readdir(root).unwrap();
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"subdir"));
        assert!(names.contains(&"file.txt"));
    }

    #[test]
    fn mkdir_duplicate_returns_error() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        fs.mkdir(root, "dir").unwrap();
        assert_eq!(fs.mkdir(root, "dir"), Err(VfsError::AlreadyExists));
    }

    #[test]
    fn unlink_removes_file() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        let inode = fs.create(root, "delete_me").unwrap();
        fs.write(inode, b"data", 0).unwrap();
        fs.unlink(root, "delete_me").unwrap();
        let mut buf = [0u8; 10];
        assert_eq!(fs.read(inode, &mut buf, 0), Err(VfsError::FileNotFound));
    }

    #[test]
    fn unlink_nonexistent_returns_error() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        assert_eq!(fs.unlink(root, "nope"), Err(VfsError::FileNotFound));
    }

    #[test]
    fn write_to_directory_returns_error() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        assert_eq!(fs.write(root, b"data", 0), Err(VfsError::IsADirectory));
    }

    #[test]
    fn read_directory_returns_error() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        let mut buf = [0u8; 10];
        assert_eq!(fs.read(root, &mut buf, 0), Err(VfsError::IsADirectory));
    }

    #[test]
    fn serialize_roundtrip() {
        let fs = new_fs();
        let root = fs.get_root_inode();
        fs.create(root, "file1").unwrap();
        fs.write(fs.nodes()[1].inode, b"content1", 0).unwrap();
        fs.create(root, "file2").unwrap();
        fs.write(fs.nodes()[2].inode, b"content2", 0).unwrap();

        let image = fs.to_image();
        assert_eq!(&image[0..4], BWFS_MAGIC);

        let loaded = BwFs::from_image(&image).unwrap();
        let entries = loaded.readdir(loaded.get_root_inode()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn from_image_invalid_magic() {
        assert!(BwFs::from_image(&[0u8; 20]).is_none());
    }

    #[test]
    fn from_image_too_short() {
        assert!(BwFs::from_image(&[0u8; 11]).is_none());
    }

    #[test]
    fn from_image_wrong_version() {
        let mut image = alloc::vec![0u8; 128];
        image[0..4].copy_from_slice(BWFS_MAGIC);
        image[4..8].copy_from_slice(&99u32.to_le_bytes());
        assert!(BwFs::from_image(&image).is_none());
    }
}
