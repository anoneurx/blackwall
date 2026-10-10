//! Kernel filesystem adapter — thin bridge between the kernel init sequence
//! and the portable `blackwall-fs` library crate.
//!
//! # Responsibilities
//! - Holds the global `VFS: SpinLock<Option<VfsManager>>` that the rest of the
//!   kernel uses (syscall layer, process PCB, `init.rs`).
//! - `init()` populates the RAM filesystem with the embedded init ELF, formats
//!   an in-memory Ext2 image, and mounts both via VfsManager.
//!
//! All VFS trait, node, and error types now live in `blackwall-fs` — see
//! `fs/src/` in the workspace root.

extern crate alloc;

use alloc::sync::Arc;

use blackwall_fs::block::MemBlockDevice;
use blackwall_fs::ext2::Ext2Fs;
use blackwall_fs::ramfs::RamFs;
use blackwall_fs::vfs::{FileSystem, VfsManager};

// Re-export VFS types so the rest of the kernel can do
// `use crate::fs::vfs::{OpenFile, VFS, VfsError}` unchanged.
pub mod vfs {
    pub use blackwall_fs::vfs::{
        DirEntry, FileSystem, MountPoint, VfsError, VfsManager, Vnode, VnodeType,
    };

    // OpenFile stays kernel-side because it holds a SpinLock reference.
    extern crate alloc;
    use crate::sync::spin::SpinLock;
    use alloc::sync::Arc;

    #[derive(Clone)]
    pub struct OpenFile {
        pub vnode: Arc<Vnode>,
        pub offset: Arc<SpinLock<u64>>,
    }

    // Global VFS manager — protected by the kernel's SpinLock.
    pub static VFS: SpinLock<Option<VfsManager>> = SpinLock::new(None);
}

use crate::arch::x86_64::serial;
use vfs::VFS;

/// Initialise the Virtual Filesystem layer (Phase 6).
///
/// 1. Creates a `RamFs` and populates it with `/bin/init` (the embedded ELF).
/// 2. Creates a simulated Ext2 image in-memory for `/mnt`.
/// 3. Mounts both via `VfsManager` and stores it in the global `VFS`.
pub fn init(init_elf: &[u8]) {
    serial::line("[FS] Initializing Filesystem (Phase 6, blackwall-fs)...");

    // ── 1. RamFs ──────────────────────────────────────────────────────────────
    let ramfs = Arc::new(RamFs::new());
    ramfs.add_dir(1, "bin", 2);
    ramfs.add_file(2, "init", 3, init_elf.to_vec());
    ramfs.add_dir(1, "mnt", 4);
    serial::line("[FS] RamFs populated: /bin/init, /mnt");

    // ── 2. In-memory Ext2 image ──────────────────────────────────────────────
    let dev = Arc::new(MemBlockDevice::new(4 * 1024 * 1024 / 512));
    let ext2fs = Arc::new(Ext2Fs::format(dev, 256).expect("format ext2"));
    let hello = ext2fs.create(2, "hello.txt").expect("create /mnt/hello.txt");
    ext2fs.write(hello, b"Hello from Ext2!\n", 0).expect("write /mnt/hello.txt");
    serial::line("[FS] In-memory Ext2 image ready.");

    // ── 3. Mount both filesystems ─────────────────────────────────────────────
    let mut mgr = VfsManager::new();
    mgr.mount("/", ramfs).expect("mount ramfs at /");
    mgr.mount("/mnt", ext2fs).expect("mount ext2 at /mnt");
    *VFS.lock() = Some(mgr);

    serial::line("[FS] VFS online — RamFs at /, Ext2 at /mnt");
}

/// Replace the in-memory `/mnt` with a real Ext2 filesystem backed by the
/// AHCI data disk, if one is present. An existing Ext2 image is mounted as-is;
/// otherwise the disk is formatted and seeded with `/hello.txt`.
///
/// Must be called after `drivers::init()` has discovered the data disk.
pub fn mount_disk() {
    let disk = crate::drivers::ahci::AHCI_DISK.lock().clone();
    let Some(dev) = disk else {
        serial::line("[FS] No AHCI data disk found; /mnt remains in-memory.");
        return;
    };

    let fs: Arc<dyn blackwall_fs::vfs::FileSystem + Send + Sync> =
        match Ext2Fs::from_device(Arc::clone(&dev)) {
            Ok(existing) => {
                serial::line("[FS] Mounted existing Ext2 image from disk.");
                Arc::new(existing)
            }
            Err(_) => match Ext2Fs::format(Arc::clone(&dev), 2048) {
                Ok(fresh) => {
                    let fresh = Arc::new(fresh);
                    if let Ok(ino) = fresh.create(2, "hello.txt") {
                        let _ = fresh.write(ino, b"Hello from Ext2 disk!\n", 0);
                    }
                    serial::line("[FS] Formatted fresh Ext2 image on disk.");
                    fresh
                }
                Err(_) => {
                    serial::line("[FS] Failed to format Ext2 on disk; keeping in-memory /mnt.");
                    return;
                }
            },
        };

    let mut guard = VFS.lock();
    if let Some(mgr) = guard.as_mut() {
        let _ = mgr.umount("/mnt");
        match mgr.mount("/mnt", fs) {
            Ok(()) => serial::line("[FS] Ext2 mounted from AHCI disk at /mnt"),
            Err(_) => serial::line("[FS] Failed to mount disk Ext2 at /mnt"),
        }
    }
}
