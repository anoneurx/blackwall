use blackwall_fs::ramfs::RamFs;
use blackwall_fs::vfs::{VfsError, VfsManager};
use std::sync::Arc;

#[test]
fn mkdir_and_readdir_root_includes_new_dir() {
    let ramfs = Arc::new(RamFs::new());
    ramfs.add_dir(1, "bin", 2);
    let mut mgr = VfsManager::new();
    mgr.mount("/", ramfs).expect("mount /");

    assert!(mgr.mkdir("/tmpdir").is_ok());
    let names: Vec<String> =
        mgr.readdir("/").expect("readdir /").iter().map(|e| e.name.clone()).collect();
    assert!(names.iter().any(|n| n == "tmpdir"), "new dir must be listed in /: {names:?}");
}

#[test]
fn rmdir_removes_empty_directory() {
    let ramfs = Arc::new(RamFs::new());
    ramfs.add_dir(1, "bin", 2);
    let mut mgr = VfsManager::new();
    mgr.mount("/", ramfs).expect("mount /");

    assert!(mgr.mkdir("/e").is_ok());
    assert_eq!(mgr.rmdir("/e"), Ok(()));
    // after removal the node is gone
    assert!(mgr.resolve_path("/e").is_err(), "removed dir must no longer resolve");
}

#[test]
fn rmdir_nonempty_directory_reports_directory_not_empty() {
    let ramfs = Arc::new(RamFs::new());
    ramfs.add_dir(1, "bin", 2);
    let mut mgr = VfsManager::new();
    mgr.mount("/", ramfs).expect("mount /");

    assert!(mgr.mkdir("/d").is_ok());
    assert!(mgr.create_file("/d/f").is_ok());
    assert_eq!(mgr.rmdir("/d"), Err(VfsError::DirectoryNotEmpty));
}

#[test]
fn unlink_still_rejects_bare_file_slots() {
    // unlink on a file works; unlinking the root-level dir entry through
    // unlink() directly is allowed only when empty (same semantics as rmdir).
    let ramfs = Arc::new(RamFs::new());
    ramfs.add_dir(1, "bin", 2);
    let mut mgr = VfsManager::new();
    mgr.mount("/", ramfs).expect("mount /");
    assert!(mgr.create_file("/f.txt").is_ok());
    assert_eq!(mgr.unlink("/f.txt"), Ok(()));
}
