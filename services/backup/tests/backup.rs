//! Integration tests for the backup library (local transport).

use bwbackup::{create, delete, list, restore, Config};
use std::path::Path;

fn make_cfg(state: &Path, dest: &Path, paths: Vec<String>, keep: usize) -> Config {
    let mut c = Config::default();
    c.destination = dest.display().to_string();
    c.state_dir = state.display().to_string();
    c.paths = paths;
    c.keep = keep;
    c
}

fn prepare(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let id = std::process::id();
    let base = std::env::temp_dir().join(format!("bwbackup-test-{tag}-{id}"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let state = base.join("state");
    let dest = base.join("backups");
    let src = base.join("data");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.toml"), b"a\n").unwrap();
    std::fs::write(src.join("sub").join("b.txt"), b"b\n").unwrap();
    (state, dest, src)
}

#[test]
fn local_create_list_restore_rotate_delete() {
    let (state, dest, src) = prepare("local");
    let cfg = make_cfg(&state, &dest, vec![src.display().to_string()], 3);

    let m1 = create(&cfg, "sun1").unwrap();
    assert!(m1.size_bytes > 0);
    // Create a gap for deterministic ordering.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let m2 = create(&cfg, "sun2").unwrap();
    assert_eq!(m1.name, "sun1");
    assert_eq!(m2.name, "sun2");

    let listed = list(&cfg).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "sun2", "newest first");

    // Backed-up file exists under the copy.
    fn sanitized(p: &std::path::Path) -> String {
        p.display().to_string().trim_start_matches('/').replace('/', "_")
    }
    assert!(dest.join("sun1").join(sanitized(&src)).join("a.toml").exists());

    // Mutate source and restore content.
    std::fs::write(src.join("a.toml"), b"changed\n").unwrap();
    std::fs::remove_file(src.join("sub").join("b.txt")).unwrap();
    restore(&cfg, "sun1").unwrap();
    assert_eq!(std::fs::read_to_string(src.join("a.toml")).unwrap(), "a\n");
    assert!(src.join("sub").join("b.txt").exists());

    // Rotation: create keep+2 backups, oldest should prune below `keep`.
    std::thread::sleep(std::time::Duration::from_millis(20));
    create(&cfg, "sun3").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    create(&cfg, "sun4").unwrap();
    let listed = list(&cfg).unwrap();
    assert!(listed.len() <= 3, "rotation keeps only `keep` backups, got {}", listed.len());

    // Delete removes the entry.
    delete(&cfg, "sun4").unwrap();
    assert!(list(&cfg).unwrap().iter().all(|b| b.name != "sun4"));
    assert!(delete(&cfg, "nope").is_err());

    let _ = std::fs::remove_dir_all(&dest.parent().unwrap());
}

#[test]
fn unsafe_names_rejected() {
    let (state, dest, src) = prepare("names");
    let cfg = make_cfg(&state, &dest, vec![src.display().to_string()], 5);
    assert!(create(&cfg, "a/../../b").is_err());
    assert!(create(&cfg, "").is_err());
    let _ = std::fs::remove_dir_all(&dest.parent().unwrap());
}
