//! # Backup library
//!
//! Scheduled, rotating backups of configured paths. Two transports are
//! supported:
//!
//! - **local** — sources are copied into a timestamped directory under the
//!   configured destination;
//! - **remote** — sources are pushed over `rsync` to `user@host:path`, with
//!   metadata kept locally in the state index so backups can be listed and
//!   restored.
//!
//! Every backup carries a `BACKUP.toml` metadata file. Rotation prunes the
//! oldest backups past the configured `keep` count.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default configuration file.
pub const DEFAULT_CONFIG: &str = "/etc/bwbackup/backup.toml";
/// Default metadata / local state directory.
pub const DEFAULT_STATE_DIR: &str = "/var/lib/bwbackup";
/// Default local backup target.
pub const DEFAULT_DEST: &str = "/var/lib/bwbackup/backups";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Local directory (or `user@host:path` for remote) receiving backups.
    pub destination: String,
    /// Paths to back up.
    pub paths: Vec<String>,
    /// Rotate: keep at most this many backups.
    pub keep: usize,
    /// Seconds between scheduled backups (0 = manual only).
    pub interval_secs: u64,
    /// Metadata state directory (used for remote backups).
    pub state_dir: String,
    /// Extra rsync args (remote only).
    pub rsync_extra: Vec<String>,
    /// rsync binary path.
    pub rsync_bin: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            destination: DEFAULT_DEST.to_string(),
            paths: vec!["/etc".to_string()],
            keep: 7,
            interval_secs: 0,
            state_dir: DEFAULT_STATE_DIR.to_string(),
            rsync_extra: vec!["--delete".to_string()],
            rsync_bin: "/usr/bin/rsync".to_string(),
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Config {
        fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    fn is_remote(&self) -> bool {
        let host_part = self.destination.splitn(2, ':').next().unwrap_or("");
        let has_port = self.destination.contains(':')
            && !self.destination.starts_with('/')
            && !host_part.is_empty();
        has_port
    }
}

/// Metadata for one backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMeta {
    pub name: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub paths: Vec<String>,
    pub remote: bool,
    pub destination: String,
}

/// One listed backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub name: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub paths: Vec<String>,
    pub remote: bool,
    pub source: String,
}

fn sanitize_component(p: &Path) -> String {
    let s = p.display().to_string();
    let s = s.trim_start_matches('/');
    s.replace('/', "_")
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

fn meta_path(meta_dir: &Path, name: &str) -> PathBuf {
    meta_dir.join(name).join("BACKUP.toml")
}

fn write_meta(meta_dir: &Path, meta: &BackupMeta) -> Result<()> {
    fs::create_dir_all(meta_dir.join(&meta.name))?;
    fs::write(meta_path(meta_dir, &meta.name), toml::to_string(meta)?)?;
    Ok(())
}

fn read_meta(meta_dir: &Path, name: &str) -> Result<BackupMeta> {
    let raw = fs::read_to_string(meta_path(meta_dir, name))
        .with_context(|| format!("backup '{}' missing metadata in {}", name, meta_dir.display()))?;
    toml::from_str(&raw).context("malformed backup metadata")
}

/// Create a backup named `name` (or a timestamped default).
pub fn create(cfg: &Config, name: &str) -> Result<BackupMeta> {
    if name.is_empty() || name.contains("..") || name.contains('/') {
        anyhow::bail!("invalid backup name: {name:?}");
    }
    let remote = cfg.is_remote();
    let (target_dir, meta_dir): (PathBuf, PathBuf) = if remote {
        // Data lives on the remote host; metadata stays local.
        let state = PathBuf::from(&cfg.state_dir);
        fs::create_dir_all(state.join("remote"))?;
        let dest = PathBuf::from(&cfg.destination);
        (dest.join(name), state.join("remote"))
    } else {
        let dest = PathBuf::from(&cfg.destination);
        if !dest.exists() {
            fs::create_dir_all(&dest)?;
        }
        (dest.join(name), dest)
    };

    let mut copied: Vec<String> = Vec::new();
    for p in &cfg.paths {
        let src = Path::new(p);
        if !src.exists() {
            continue;
        }
        copied.push(p.clone());
        if remote {
            let rsync_target = format!("{}/{}", cfg.destination, name);
            let mut cmd = Command::new(&cfg.rsync_bin);
            let mut cmd = cmd.args(["-a".to_string()]);
            for extra in &cfg.rsync_extra {
                cmd = cmd.arg(extra);
            }
            cmd = cmd.arg(format!("{}/", src.display())).arg(&rsync_target);
            let out = cmd.output().with_context(|| format!("failed to run {}", cfg.rsync_bin))?;
            if !out.status.success() {
                anyhow::bail!("rsync failed: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
        } else {
            let to = target_dir.join(sanitize_component(src));
            copy_tree(src, &to)?;
        }
    }

    let size = if remote { 0 } else { size_of_dir(&target_dir) };

    let meta = BackupMeta {
        name: name.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        size_bytes: size,
        paths: copied,
        remote,
        destination: cfg.destination.clone(),
    };
    write_meta(&meta_dir, &meta)?;
    rotate(cfg)?;
    Ok(meta)
}

/// List backups newest first.
pub fn list(cfg: &Config) -> Result<Vec<Backup>> {
    let mut out = Vec::new();

    let local_dir = PathBuf::from(&cfg.destination);
    if !cfg.is_remote() {
        if local_dir.exists() {
            for entry in fs::read_dir(&local_dir)? {
                let entry = entry?;
                if !entry.path().is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(m) = read_meta(&local_dir, &name) {
                    out.push(Backup {
                        name: m.name,
                        created_at: m.created_at,
                        size_bytes: m.size_bytes,
                        paths: m.paths,
                        remote: m.remote,
                        source: entry.path().display().to_string(),
                    });
                }
            }
        }
    }

    let remote_dir = PathBuf::from(&cfg.state_dir).join("remote");
    if remote_dir.exists() {
        for entry in fs::read_dir(&remote_dir)? {
            let entry = entry?;
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(m) = read_meta(&remote_dir, &name) {
                out.push(Backup {
                    name: m.name,
                    created_at: m.created_at,
                    size_bytes: m.size_bytes,
                    paths: m.paths,
                    remote: m.remote,
                    source: format!("{}", m.destination),
                });
            }
        }
    }

    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// Restore the newest copy of each tracked path.
pub fn restore(cfg: &Config, name: &str) -> Result<BackupMeta> {
    let remote = cfg.is_remote();
    let (target_dir, meta_dir): (PathBuf, PathBuf) = if remote {
        (PathBuf::from(&cfg.destination).join(name), PathBuf::from(&cfg.state_dir).join("remote"))
    } else {
        let dest = PathBuf::from(&cfg.destination);
        (dest.join(name), dest)
    };
    let meta = read_meta(&meta_dir, name)?;

    if meta.remote || remote {
        for p in &meta.paths {
            let src = Path::new(p);
            fs::create_dir_all(src)?;
            let remote_src = format!("{}/", target_dir.display());
            let mut cmd = Command::new(&cfg.rsync_bin);
            let mut cmd = cmd.args(["-a".to_string()]);
            for extra in &cfg.rsync_extra {
                cmd = cmd.arg(extra);
            }
            cmd = cmd.arg(&remote_src).arg(src);
            let out = cmd.output().with_context(|| format!("failed to run {}", cfg.rsync_bin))?;
            if !out.status.success() {
                anyhow::bail!(
                    "rsync restore failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
        }
        return Ok(meta);
    }

    for p in &meta.paths {
        let src = Path::new(p);
        let captured = target_dir.join(sanitize_component(src));
        if !captured.exists() {
            continue;
        }
        fs::create_dir_all(src)?;
        copy_tree(&captured, src)?;
    }
    Ok(meta)
}

/// Delete an old backup.
pub fn delete(cfg: &Config, name: &str) -> Result<BackupMeta> {
    let remote = cfg.is_remote();
    let (target_dir, meta_dir): (PathBuf, PathBuf) = if remote {
        (PathBuf::from(&cfg.destination).join(name), PathBuf::from(&cfg.state_dir).join("remote"))
    } else {
        let dest = PathBuf::from(&cfg.destination);
        (dest.join(name), dest)
    };
    let meta = read_meta(&meta_dir, name)?;
    if !meta.remote {
        let _ = fs::remove_dir_all(&target_dir);
    }
    let _ = fs::remove_dir_all(meta_dir.join(name));
    Ok(meta)
}

/// Prune the oldest backups beyond `cfg.keep`.
fn rotate(cfg: &Config) -> Result<()> {
    if cfg.keep == 0 {
        return Ok(());
    }
    let all = list(cfg)?;
    if all.len() <= cfg.keep {
        return Ok(());
    }
    for b in all.iter().skip(cfg.keep) {
        let _ = delete(cfg, &b.name);
    }
    Ok(())
}
