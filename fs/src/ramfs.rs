extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::vfs::{DirEntry, FileSystem, VfsError, Vnode, VnodeType};

// ── Inode counter ─────────────────────────────────────────────────────────────

static NEXT_INODE: AtomicU64 = AtomicU64::new(10);

fn alloc_inode() -> u64 {
    NEXT_INODE.fetch_add(1, Ordering::Relaxed)
}

// ── Internal node ─────────────────────────────────────────────────────────────

struct RamNode {
    inode: u64,
    parent_inode: u64,
    name: String,
    is_dir: bool,
    data: Vec<u8>,
}

// ── RamFs ────────────────────────────────────────────────────────────────────

/// Fully mutable in-memory filesystem.
///
/// Internally uses a `spin::Mutex`-equivalent approach; for the standalone crate
/// the inner list is wrapped in a `core::cell::UnsafeCell` with an atomic flag
/// acting as a ticket spin-lock so the kernel can supply its own `SpinLock` at
/// the integration layer.
///
/// For the standalone crate (no kernel) we expose a `RamFs` that is **Send +
/// Sync** by marking interior mutability safe at the single-threaded integration
/// boundary. The kernel wrapper re-wraps this in its own `SpinLock<RamFs>`.
pub struct RamFs {
    // We use a simple UnsafeCell here because the kernel always wraps this in
    // a SpinLock. In the standalone crate we assume single-threaded init.
    nodes: core::cell::UnsafeCell<Vec<RamNode>>,
}

// SAFETY: The kernel always wraps RamFs in a SpinLock before sharing across
// threads. The UnsafeCell interior is only mutated while the lock is held.
unsafe impl Send for RamFs {}
unsafe impl Sync for RamFs {}

impl RamFs {
    pub fn new() -> Self {
        let root = RamNode {
            inode: 1,
            parent_inode: 1,
            name: String::new(),
            is_dir: true,
            data: Vec::new(),
        };
        let nodes = Vec::from([root]);
        Self { nodes: core::cell::UnsafeCell::new(nodes) }
    }

    fn nodes(&self) -> &Vec<RamNode> {
        // SAFETY: caller holds the kernel SpinLock.
        unsafe { &*self.nodes.get() }
    }

    fn nodes_mut(&self) -> &mut Vec<RamNode> {
        // SAFETY: caller holds the kernel SpinLock.
        unsafe { &mut *self.nodes.get() }
    }

    /// Pre-populate a directory (used by kernel init).
    pub fn add_dir(&self, parent_inode: u64, name: &str, inode: u64) {
        self.nodes_mut().push(RamNode {
            inode,
            parent_inode,
            name: String::from(name),
            is_dir: true,
            data: Vec::new(),
        });
    }

    /// Pre-populate a file (used by kernel init to embed the init ELF).
    pub fn add_file(&self, parent_inode: u64, name: &str, inode: u64, data: Vec<u8>) {
        self.nodes_mut().push(RamNode {
            inode,
            parent_inode,
            name: String::from(name),
            is_dir: false,
            data,
        });
    }

    /// Return the total number of bytes allocated across all file nodes.
    pub fn memory_used(&self) -> usize {
        self.nodes().iter().map(|n| n.data.len()).sum()
    }
}

impl FileSystem for RamFs {
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
        let avail = node.data.len() - start;
        let n = buf.len().min(avail);
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
        // Extend data if needed
        if start + buf.len() > node.data.len() {
            node.data.resize(start + buf.len(), 0);
        }
        node.data[start..start + buf.len()].copy_from_slice(buf);
        Ok(buf.len())
    }

    fn lookup(
        &self,
        parent_inode: u64,
        name: &str,
        fs_arc: Arc<dyn FileSystem + Send + Sync>,
    ) -> Result<Vnode, VfsError> {
        let nodes = self.nodes();
        let node = nodes
            .iter()
            .find(|n| n.parent_inode == parent_inode && n.name == name)
            .ok_or(VfsError::FileNotFound)?;
        Ok(Vnode {
            inode: node.inode,
            size: node.data.len() as u64,
            vtype: if node.is_dir { VnodeType::Directory } else { VnodeType::File },
            fs: fs_arc,
        })
    }

    fn readdir(&self, inode: u64) -> Result<Vec<DirEntry>, VfsError> {
        let nodes = self.nodes();
        let entries = nodes
            .iter()
            .filter(|n| n.parent_inode == inode && n.inode != inode)
            .map(|n| DirEntry {
                name: n.name.clone(),
                inode: n.inode,
                vtype: if n.is_dir { VnodeType::Directory } else { VnodeType::File },
            })
            .collect();
        Ok(entries)
    }

    fn create(&self, parent_inode: u64, name: &str) -> Result<u64, VfsError> {
        // Check parent exists and is a directory
        {
            let nodes = self.nodes();
            let parent =
                nodes.iter().find(|n| n.inode == parent_inode).ok_or(VfsError::FileNotFound)?;
            if !parent.is_dir {
                return Err(VfsError::NotADirectory);
            }
            if nodes.iter().any(|n| n.parent_inode == parent_inode && n.name == name) {
                return Err(VfsError::AlreadyExists);
            }
        }
        let inode = alloc_inode();
        self.nodes_mut().push(RamNode {
            inode,
            parent_inode,
            name: String::from(name),
            is_dir: false,
            data: Vec::new(),
        });
        Ok(inode)
    }

    fn mkdir(&self, parent_inode: u64, name: &str) -> Result<u64, VfsError> {
        {
            let nodes = self.nodes();
            let parent =
                nodes.iter().find(|n| n.inode == parent_inode).ok_or(VfsError::FileNotFound)?;
            if !parent.is_dir {
                return Err(VfsError::NotADirectory);
            }
            if nodes.iter().any(|n| n.parent_inode == parent_inode && n.name == name) {
                return Err(VfsError::AlreadyExists);
            }
        }
        let inode = alloc_inode();
        self.nodes_mut().push(RamNode {
            inode,
            parent_inode,
            name: String::from(name),
            is_dir: true,
            data: Vec::new(),
        });
        Ok(inode)
    }

    fn unlink(&self, parent_inode: u64, name: &str) -> Result<(), VfsError> {
        let nodes = self.nodes_mut();
        let idx = nodes
            .iter()
            .position(|n| n.parent_inode == parent_inode && n.name == name)
            .ok_or(VfsError::FileNotFound)?;
        if nodes[idx].is_dir {
            return Err(VfsError::IsADirectory);
        }
        nodes.remove(idx);
        Ok(())
    }

    fn get_root_inode(&self) -> u64 {
        1
    }
}
