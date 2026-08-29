//! # Snapshot library
//!
//! File-tree snapshots for transaction/rollback safety. Each snapshot is a
//! point-in-time copy of a configured set of paths (defaults: package & API
//! state) stored under a snapshot store directory:
//!
//! ```text
//! <store>/<name>/
//!   SNAPSHOT.toml    # metadata (created_at, size, captured paths)
//!   <sanitized path>/...  # copied files, mirrors originals
//! ```
//!
//! The original absolute path of every captured tree is recorded in the
//! metadata so `restore` can copy files back to exactly where they came from.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Snapshot that captures package-manager and API state.
pub const DEFAULT_PATHS: &[&str] = &["/etc/anx", "/etc/bwapi", "/etc/bwinit/services"];

/// Default snapshot store.
pub const DEFAULT_STORE: &str = "/var/lib/bwsnap/snapshots";

/// Metadata written at the root of every snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub name: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub paths: Vec<String>,
}

/// One row from the snapshot store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub paths: Vec<String>,
    pub path: String,
}

fn sanitize_component(p: &Path) -> String {
    let s = p.display().to_string();
    let s = s.trim_start_matches('/');
    s.replace('/', "_")
}

fn meta_path(store: &Path, name: &str) -> PathBuf {
    store.join(name).join("SNAPSHOT.toml")
}

fn size_of_dir(root: &Path) -> u64 {
    let mut total = 0u64;
    walk(root, &mut |f| {
        if f.is_file() {
            if let Ok(m) = f.metadata() {
                total += m.len();
            }
        }
    });
    total
}

fn walk(root: &Path, visit: &mut impl FnMut(&Path)) {
    if root.is_file() {
        visit(root);
        return;
    }
    if let Ok(rd) = fs::read_dir(root) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                walk(&p, visit);
            } else {
                visit(&p);
            }
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let from = e.path();
        let to = dst.join(e.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else if from.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Create a snapshot of `paths` under `store/name`.
pub fn create(store: &Path, name: &str, paths: &[String]) -> Result<SnapshotMeta> {
    if name.is_empty() || name == "SNAPSHOT" || name.contains("..") || name.contains('/') {
        anyhow::bail!("invalid snapshot name: {name:?}");
    }
    let dest = store.join(name);
    if dest.exists() {
        anyhow::bail!("snapshot '{name}' already exists");
    }
    fs::create_dir_all(&dest)?;

    let mut paths_copied: Vec<String> = Vec::new();
    for p in paths {
        let src = Path::new(p);
        if !src.exists() {
            continue;
        }
        paths_copied.push(p.clone());
        let target = dest.join(sanitize_component(src));
        copy_tree(src, &target)?;
    }

    let size = size_of_dir(&dest);
    let meta = SnapshotMeta {
        name: name.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        size_bytes: size,
        paths: paths_copied,
    };
    let raw = toml::to_string(&meta)?;
    fs::write(meta_path(store, name), &raw)?;
    Ok(meta)
}

/// Read the metadata of one snapshot.
pub fn read_meta(store: &Path, name: &str) -> Result<SnapshotMeta> {
    let raw = fs::read_to_string(meta_path(store, name))
        .with_context(|| format!("snapshot '{name}' missing in {}", store.display()))?;
    toml::from_str(&raw).context("malformed snapshot metadata")
}

/// List all snapshots, newest first.
pub fn list(store: &Path) -> Result<Vec<Snapshot>> {
    let mut out = Vec::new();
    if !store.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(store)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(m) = read_meta(store, &name) {
            out.push(Snapshot {
                path: entry.path().display().to_string(),
                name: m.name,
                created_at: m.created_at,
                size_bytes: m.size_bytes,
                paths: m.paths,
            });
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// Copy files from a snapshot back to their original locations.
pub fn restore(store: &Path, name: &str) -> Result<SnapshotMeta> {
    let meta = read_meta(store, name)?;
    let root = store.join(name);
    for p in &meta.paths {
        let src = Path::new(p);
        let captured = root.join(sanitize_component(src));
        if !captured.exists() {
            continue;
        }
        fs::create_dir_all(src)?;
        copy_tree(&captured, src)?;
    }
    Ok(meta)
}

/// Delete a snapshot.
pub fn delete(store: &Path, name: &str) -> Result<SnapshotMeta> {
    let meta = read_meta(store, name)?;
    fs::remove_dir_all(store.join(name))
        .with_context(|| format!("failed to remove snapshot '{name}'"))?;
    Ok(meta)
}
