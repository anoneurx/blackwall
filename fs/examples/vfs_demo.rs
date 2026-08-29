//! Filesystem example: create an in-memory (`RamFs`) filesystem, populate a
//! file, mount it into the VFS, and read it back through the path resolver.
//!
//! This is a host-run demonstration of the `blackwall-fs` API. The same types
//! are used by the kernel to build its root filesystem at boot.
//!
//! Run with: `cargo run --example vfs_demo -p blackwall-fs`

use blackwall_fs::ramfs::RamFs;
use blackwall_fs::vfs::{FileSystem, VfsManager};
use std::sync::Arc;

fn main() {
    // 1. Create a RAM filesystem and pre-populate a single file "greeting.txt".
    let ram = RamFs::new();
    ram.add_file(1, "greeting.txt", 2, b"Hello from Black Wall Core\n".to_vec());
    let fs: Arc<dyn FileSystem + Send + Sync> = Arc::new(ram);

    // 2. Mount it at the root of the VFS.
    let mut vfs = VfsManager::new();
    vfs.mount("/", fs).expect("mount root");

    // 3. Resolve and read the file through the VFS.
    let data = vfs.read_all("/greeting.txt").expect("read file");
    print!("{}", String::from_utf8_lossy(&data));

    // Show directory listing through the trait.
    let root = vfs.resolve_path("/").expect("resolve /");
    let entries = root.fs.readdir(root.inode).expect("readdir");
    println!(
        "root entries: {}",
        entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>().join(", ")
    );
}
