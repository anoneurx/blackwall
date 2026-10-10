extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

// ── Error Types ──────────────────────────────────────────────────────────────

/// All errors the VFS layer can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    Success,
    FileNotFound,
    NotADirectory,
    IsADirectory,
    NoSpace,
    IOError,
    InvalidPath,
    PermissionDenied,
    NotSupported,
    AlreadyExists,
    OutOfInodes,
}

// ── Vnode ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VnodeType {
    File,
    Directory,
    Symlink,
    CharDevice,
    BlockDevice,
}

/// A resolved filesystem node (file or directory).
pub struct Vnode {
    pub inode: u64,
    pub size: u64,
    pub vtype: VnodeType,
    pub fs: Arc<dyn FileSystem + Send + Sync>,
}

/// A listing entry returned by `readdir`.
pub struct DirEntry {
    pub name: String,
    pub inode: u64,
    pub vtype: VnodeType,
}

// ── FileSystem Trait ─────────────────────────────────────────────────────────

/// Every mounted filesystem must implement this trait.
pub trait FileSystem {
    /// Read up to `buf.len()` bytes from `inode` starting at `offset`.
    fn read(&self, inode: u64, buf: &mut [u8], offset: u64) -> Result<usize, VfsError>;

    /// Write `buf` into `inode` starting at `offset`.
    fn write(&self, inode: u64, buf: &[u8], offset: u64) -> Result<usize, VfsError>;

    /// Look up a name within `parent_inode`. Returns the child Vnode.
    fn lookup(
        &self,
        parent_inode: u64,
        name: &str,
        fs_arc: Arc<dyn FileSystem + Send + Sync>,
    ) -> Result<Vnode, VfsError>;

    /// List all entries within `inode` (must be a directory).
    fn readdir(&self, inode: u64) -> Result<Vec<DirEntry>, VfsError>;

    /// Create a new file under `parent_inode`.
    fn create(&self, parent_inode: u64, name: &str) -> Result<u64, VfsError>;

    /// Create a new directory under `parent_inode`.
    fn mkdir(&self, parent_inode: u64, name: &str) -> Result<u64, VfsError>;

    /// Unlink (delete) a file by name from `parent_inode`.
    fn unlink(&self, parent_inode: u64, name: &str) -> Result<(), VfsError>;

    /// Return the inode number for the root of this filesystem.
    fn get_root_inode(&self) -> u64;
}

// ── Mount Table ──────────────────────────────────────────────────────────────

pub struct MountPoint {
    pub path: String,
    pub fs: Arc<dyn FileSystem + Send + Sync>,
}

/// The VFS mount-point manager. One global instance lives in the kernel
/// protected by its own SpinLock.
pub struct VfsManager {
    mounts: Vec<MountPoint>,
}

impl VfsManager {
    pub fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    /// Mount a filesystem at the given absolute path.
    pub fn mount(
        &mut self,
        path: &str,
        fs: Arc<dyn FileSystem + Send + Sync>,
    ) -> Result<(), VfsError> {
        self.mounts.push(MountPoint { path: String::from(path), fs });
        Ok(())
    }

    /// Unmount a filesystem by its absolute path.
    pub fn umount(&mut self, path: &str) -> Result<(), VfsError> {
        let idx = self.mounts.iter().position(|m| m.path == path).ok_or(VfsError::FileNotFound)?;
        self.mounts.remove(idx);
        Ok(())
    }

    /// Resolve an absolute path to a Vnode by walking the mount table and
    /// then descending the directory tree of the best-matching mount.
    pub fn resolve_path(&self, path: &str) -> Result<Vnode, VfsError> {
        if !path.starts_with('/') {
            return Err(VfsError::InvalidPath);
        }

        // Find the longest-prefix matching mount point
        let mut best: Option<&MountPoint> = None;
        for mount in &self.mounts {
            let matches = if mount.path == "/" {
                path.starts_with('/')
            } else {
                path == mount.path
                    || (path.starts_with(&mount.path)
                        && path.chars().nth(mount.path.len()) == Some('/'))
            };
            if matches {
                if let Some(b) = best {
                    if mount.path.len() > b.path.len() {
                        best = Some(mount);
                    }
                } else {
                    best = Some(mount);
                }
            }
        }

        let mount = best.ok_or(VfsError::FileNotFound)?;
        let relative = &path[mount.path.len()..];

        let mut current = Vnode {
            inode: mount.fs.get_root_inode(),
            size: 0,
            vtype: VnodeType::Directory,
            fs: Arc::clone(&mount.fs),
        };

        for part in relative.split('/').filter(|s| !s.is_empty()) {
            if current.vtype != VnodeType::Directory {
                return Err(VfsError::NotADirectory);
            }
            let fs_ref = Arc::clone(&current.fs);
            current = current.fs.lookup(current.inode, part, fs_ref)?;
        }

        Ok(current)
    }

    /// Convenience: read an entire file into a Vec<u8>.
    pub fn read_all(&self, path: &str) -> Result<Vec<u8>, VfsError> {
        let vnode = self.resolve_path(path)?;
        if vnode.vtype == VnodeType::Directory {
            return Err(VfsError::IsADirectory);
        }
        let size = vnode.size as usize;
        let mut buf = alloc::vec![0u8; size.max(4096)];
        let n = vnode.fs.read(vnode.inode, &mut buf, 0)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Split `path` into its parent directory path and final component.
    /// `/foo` → (`/`, `foo`); `/mnt/dir/file` → (`/mnt/dir`, `file`).
    /// Returns `None` for the root itself.
    pub fn split_parent(&self, path: &str) -> Option<(String, String)> {
        if !path.starts_with('/') {
            return None;
        }
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        let idx = trimmed.rfind('/')?;
        if idx == 0 {
            Some((String::from("/"), String::from(&trimmed[1..])))
        } else {
            Some((String::from(&trimmed[..idx]), String::from(&trimmed[idx + 1..])))
        }
    }

    /// Create a new empty file at `path`. Fails with [`VfsError::AlreadyExists`]
    /// if a node with that name already exists.
    pub fn create_file(&self, path: &str) -> Result<u64, VfsError> {
        let (parent_path, name) = self.split_parent(path).ok_or(VfsError::InvalidPath)?;
        let parent = self.resolve_path(&parent_path)?;
        if parent.vtype != VnodeType::Directory {
            return Err(VfsError::NotADirectory);
        }
        parent.fs.create(parent.inode, &name)
    }

    /// Create a new directory at `path`.
    pub fn mkdir(&self, path: &str) -> Result<u64, VfsError> {
        let (parent_path, name) = self.split_parent(path).ok_or(VfsError::InvalidPath)?;
        let parent = self.resolve_path(&parent_path)?;
        if parent.vtype != VnodeType::Directory {
            return Err(VfsError::NotADirectory);
        }
        parent.fs.mkdir(parent.inode, &name)
    }

    /// Remove a file at `path`. Fails if the node is a directory.
    pub fn unlink(&self, path: &str) -> Result<(), VfsError> {
        let (parent_path, name) = self.split_parent(path).ok_or(VfsError::InvalidPath)?;
        let parent = self.resolve_path(&parent_path)?;
        if parent.vtype != VnodeType::Directory {
            return Err(VfsError::NotADirectory);
        }
        parent.fs.unlink(parent.inode, &name)
    }

    /// Remove a directory at `path` (must be empty). Mimics POSIX `rmdir`.
    pub fn rmdir(&self, path: &str) -> Result<(), VfsError> {
        let node = self.resolve_path(path)?;
        if node.vtype != VnodeType::Directory {
            return Err(VfsError::NotADirectory);
        }
        self.unlink(path)
    }

    /// List the entries of the directory at `path`.
    pub fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, VfsError> {
        let node = self.resolve_path(path)?;
        if node.vtype != VnodeType::Directory {
            return Err(VfsError::NotADirectory);
        }
        node.fs.readdir(node.inode)
    }

    /// Return `(size, vtype)` for the node at `path`.
    pub fn stat(&self, path: &str) -> Result<(u64, VnodeType), VfsError> {
        let node = self.resolve_path(path)?;
        Ok((node.size, node.vtype))
    }
}
