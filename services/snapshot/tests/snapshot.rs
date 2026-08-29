//! Unit tests for the snapshot engine.

use bwsnap::{create, delete, list, read_meta, restore};
use std::path::Path;

fn tmp_store(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("bwsnap-test-{tag}-{}", std::process::id()))
}

fn setup(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let store = tmp_store(tag);
    let _ = std::fs::remove_dir_all(&store);
    std::fs::create_dir_all(&store).unwrap();

    // A source tree we control.
    let src = std::env::temp_dir().join(format!("bwsnap-src-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("conf.toml"), b"key = \"one\"\n").unwrap();
    std::fs::write(src.join("sub").join("nested.txt"), b"nested\n").unwrap();
    (store, src)
}

fn teardown(store: &Path, src: &Path) {
    let _ = std::fs::remove_dir_all(store);
    let _ = std::fs::remove_dir_all(src);
}

#[test]
fn create_list_restore_delete_roundtrip() {
    let (store, src) = setup("roundtrip");
    let target = src.join("clone");
    std::fs::create_dir_all(&target).unwrap();

    let path = src.display().to_string();
    let meta = create(&store, "pre", &[path.clone()]).unwrap();
    assert_eq!(meta.name, "pre");
    assert!(meta.created_at.contains('T'));
    assert!(meta.size_bytes > 0);
    assert_eq!(meta.paths, vec![path.clone()]);

    // Create should refuse duplicates.
    assert!(create(&store, "pre", &[path.clone()]).is_err());

    // Reject unsafe names.
    assert!(create(&store, "a/b", &[path.clone()]).is_err());

    // Verify captured file: <store>/<name>/<sanitized-src>/...
    fn sanitized(p: &std::path::Path) -> String {
        p.display().to_string().trim_start_matches('/').replace('/', "_")
    }
    let captured = store.join("pre").join(sanitized(&src));
    assert!(captured.join("conf.toml").exists());
    assert!(captured.join("sub").join("nested.txt").exists());

    // Modify original, then restore.
    std::fs::write(src.join("conf.toml"), b"key = \"changed\"\n").unwrap();
    std::fs::remove_file(src.join("sub").join("nested.txt")).unwrap();

    let m = restore(&store, "pre").unwrap();
    assert_eq!(m.name, "pre");
    assert_eq!(std::fs::read(src.join("conf.toml")).unwrap(), b"key = \"one\"\n");
    assert!(src.join("sub").join("nested.txt").exists());

    // read_meta + list see it.
    let listed = list(&store).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "pre");
    assert_eq!(read_meta(&store, "pre").unwrap().size_bytes, listed[0].size_bytes);

    // Delete removes it.
    let d = delete(&store, "pre").unwrap();
    assert_eq!(d.name, "pre");
    assert!(!store.join("pre").exists());
    assert!(list(&store).unwrap().is_empty());
    assert!(read_meta(&store, "pre").is_err());

    // Delete of a missing snapshot errors.
    assert!(delete(&store, "missing").is_err());

    teardown(&store, &src);
}

#[test]
fn list_sorts_newest_first_and_ignores_garbage() {
    let (store, src) = setup("sort");
    let path = src.display().to_string();
    let _ = create(&store, "old", &[path.clone()]).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let _ = create(&store, "new", &[path.clone()]).unwrap();

    // Drop a non-snapshot dir that should be ignored.
    std::fs::create_dir_all(store.join("junk")).unwrap();

    let snaps = list(&store).unwrap();
    assert_eq!(snaps.len(), 2);
    assert_eq!(snaps[0].name, "new");
    assert_eq!(snaps[1].name, "old");

    teardown(&store, &src);
}

#[test]
fn missing_paths_are_skipped_in_snapshot() {
    let (store, src) = setup("missing");
    let real = src.display().to_string();
    let meta = create(&store, "p", &[real.clone(), "/nonexistent/definitely/not/here".to_string()])
        .unwrap();
    assert!(meta.paths.contains(&real.clone()));
    // The nonexistent path was skipped entirely.
    assert!(!meta.paths.iter().any(|p| p.contains("nonexistent")));
    assert!(restore(&store, "p").is_ok());

    teardown(&store, &src);
}
