//! Kernel filesystem adapter — thin bridge between the kernel init sequence
//! and the portable `blackwall-fs` library crate.
//!
//! # Responsibilities
//! - Holds the global `VFS: SpinLock<Option<VfsManager>>` that the rest of the
//!   kernel uses (syscall layer, process PCB, `init.rs`).
//! - `init()` populates the RAM filesystem with the embedded init ELF, creates
//!   a simulated Ext2 disk image for testing, and mounts both via VfsManager.
//!
//! All VFS trait, node, and error types now live in `blackwall-fs` — see
//! `fs/src/` in the workspace root.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec;

use blackwall_fs::ext2::Ext2Fs;
use blackwall_fs::ramfs::RamFs;
use blackwall_fs::vfs::VfsManager;

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

    // ── 2. Simulated Ext2 image ───────────────────────────────────────────────
    let mut ext2_image = vec![0u8; 8192];

    // Superblock magic (offset 1024 + 56)
    ext2_image[1024 + 56] = 0x53;
    ext2_image[1024 + 57] = 0xEF;
    ext2_image[1024 + 40] = 32; // s_inodes_per_group
    ext2_image[1024 + 24] = 0; // s_log_block_size = 0 → 1024-byte blocks
    ext2_image[1024 + 0] = 32; // s_inodes_count
    ext2_image[1024 + 4] = 8; // s_blocks_count

    // Block group descriptor (offset 2048): bg_inode_table = block 4
    ext2_image[2048 + 8] = 4;

    // Root inode (#2) at inode table block 4 (offset 4096), index 1 → offset 4224
    ext2_image[4224 + 0] = 0xED; // i_mode lo (directory, rwxr-xr-x)
    ext2_image[4224 + 1] = 0x41; // i_mode hi
    ext2_image[4224 + 4] = 0x00; // i_size lo
    ext2_image[4224 + 5] = 0x04; // i_size hi (1024)
    ext2_image[4224 + 40] = 5; // i_block[0] = block 5

    // Directory entries in block 5 (offset 5120)
    ext2_image[5120 + 0] = 2;
    ext2_image[5120 + 4] = 12;
    ext2_image[5120 + 6] = 1;
    ext2_image[5120 + 7] = 2;
    ext2_image[5120 + 8] = b'.'; // "."
    ext2_image[5132 + 0] = 2;
    ext2_image[5132 + 4] = 12;
    ext2_image[5132 + 6] = 2;
    ext2_image[5132 + 7] = 2;
    ext2_image[5132 + 8] = b'.';
    ext2_image[5132 + 9] = b'.'; // ".."
    ext2_image[5144 + 0] = 3; // inode 3
    ext2_image[5144 + 4] = 0xE8;
    ext2_image[5144 + 5] = 0x03; // rec_len 1000
    ext2_image[5144 + 6] = 9;
    ext2_image[5144 + 7] = 1; // name_len, file
    for (i, &b) in b"hello.txt".iter().enumerate() {
        ext2_image[5144 + 8 + i] = b;
    }

    // hello.txt inode (#3) at offset 4352 = 4096 + 2*128
    ext2_image[4352 + 0] = 0xA4;
    ext2_image[4352 + 1] = 0x81; // regular file
    ext2_image[4352 + 4] = 18; // i_size = 18
    ext2_image[4352 + 40] = 6; // i_block[0] = block 6

    // File content in block 6 (offset 6144)
    for (i, &b) in b"Hello from Ext2!\n".iter().enumerate() {
        ext2_image[6144 + i] = b;
    }

    let ext2fs = Arc::new(Ext2Fs::new(ext2_image));
    serial::line("[FS] Simulated Ext2 image ready.");

    // ── 3. Mount both filesystems ─────────────────────────────────────────────
    let mut mgr = VfsManager::new();
    mgr.mount("/", ramfs).expect("mount ramfs at /");
    mgr.mount("/mnt", ext2fs).expect("mount ext2 at /mnt");
    *VFS.lock() = Some(mgr);

    serial::line("[FS] VFS online — RamFs at /, Ext2 at /mnt");
}
