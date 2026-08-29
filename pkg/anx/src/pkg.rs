//! Package format definitions for `.anxpkg` files.
//!
//! An `.anxpkg` is a zstd-compressed tar archive with the following layout:
//!
//! ```text
//! package.anxpkg  (tar.zst)
//!   MANIFEST.toml    — package metadata and dependency list
//!   files/           — payload (installed relative to /)
//!   sig.gpg          — detached GPG signature over MANIFEST.toml + files/
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ─── Manifest ─────────────────────────────────────────────────────────────────

/// Metadata embedded in every `.anxpkg` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub arch: String,
    pub license: String,
    pub homepage: Option<String>,
    pub maintainer: String,

    /// Runtime dependencies: package name → version constraint.
    #[serde(default)]
    pub depends: HashMap<String, String>,

    /// Files installed by this package and their SHA256 checksums.
    /// Key = path relative to filesystem root (e.g. `/usr/bin/curl`).
    pub files: HashMap<String, String>,

    /// Optional post-install shell snippet (run as root).
    pub post_install: Option<String>,

    /// Optional pre-remove shell snippet (run as root).
    pub pre_remove: Option<String>,
}

// ─── Repository index entry ───────────────────────────────────────────────────

/// A single entry in the repository `index.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub arch: String,
    pub url: String,
    /// SHA256 of the `.anxpkg` file.
    pub checksum: String,
    /// Size in bytes.
    pub size: u64,
}

// ─── Installed package record ─────────────────────────────────────────────────

/// Record written to `/var/lib/anx/installed/<name>.toml` after installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed_at: String, // RFC3339 timestamp
    pub transaction_id: String,
    /// All files installed on disk.
    pub installed_files: Vec<String>,
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Returns the directory where installed package records are stored.
pub fn installed_db_dir() -> PathBuf {
    PathBuf::from("/var/lib/anx/installed")
}

/// Returns the cached package download directory.
pub fn cache_dir() -> PathBuf {
    PathBuf::from("/var/cache/anx/packages")
}

/// Ensure the standard anx directory structure exists on disk.
pub fn ensure_dirs() -> Result<()> {
    for dir in [
        "/var/lib/anx/installed",
        "/var/lib/anx/transactions",
        "/var/cache/anx/packages",
        "/etc/anx/trusted-keys",
    ] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {}", dir))?;
    }
    Ok(())
}

/// Returns true if a package with the given name is currently installed.
pub fn is_installed(name: &str) -> bool {
    installed_db_dir().join(format!("{}.toml", name)).exists()
}

/// Load the installed record for a package.
pub fn load_installed(name: &str) -> Result<InstalledPackage> {
    let path = installed_db_dir().join(format!("{}.toml", name));
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Package '{}' is not installed", name))?;
    toml::from_str(&raw).context("Failed to parse installed package record")
}

/// List all installed packages.
pub fn list_installed() -> Result<()> {
    use colored::Colorize;

    let dir = installed_db_dir();
    if !dir.exists() {
        println!("No packages installed.");
        return Ok(());
    }

    let mut count = 0;
    let mut entries: Vec<InstalledPackage> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "toml"))
        .filter_map(|e| {
            std::fs::read_to_string(e.path()).ok().and_then(|s| toml::from_str(&s).ok())
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    println!("{:<30} {:<20} {}", "NAME".bold(), "VERSION".bold(), "DESCRIPTION".bold());
    println!("{}", "─".repeat(80).dimmed());
    for pkg in &entries {
        println!(
            "{:<30} {:<20} {}",
            pkg.name.bright_cyan(),
            pkg.version.bright_green(),
            pkg.description
        );
        count += 1;
    }
    println!();
    println!("{} package(s) installed.", count);
    Ok(())
}

/// Show detailed info about an installed package.
pub fn info(name: &str) -> Result<()> {
    use colored::Colorize;

    let pkg = load_installed(name)?;
    println!("{}: {}", "Package".bold(), pkg.name.bright_cyan());
    println!("{}: {}", "Version".bold(), pkg.version);
    println!("{}: {}", "Description".bold(), pkg.description);
    println!("{}: {}", "Installed".bold(), pkg.installed_at);
    println!("{}: {}", "Transaction".bold(), pkg.transaction_id);
    println!("{}: {} files", "Files".bold(), pkg.installed_files.len());
    Ok(())
}
