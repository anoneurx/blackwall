//! Repository index generation.
//!
//! Scans a directory tree of built `.anxpkg` files, reads each embedded
//! `MANIFEST.toml`, computes the archive SHA-256 and size, and writes the
//! repository `index.toml` that the `anx` client consumes.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use walkdir::WalkDir;

/// A single `[[packages]]` row in `index.toml` (mirrors `anx`'s `RepoEntry`).
#[derive(Debug, serde::Serialize)]
pub struct RepoEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub arch: String,
    pub url: String,
    pub checksum: String,
    pub size: u64,
}

/// The full repository index document.
#[derive(Debug, serde::Serialize)]
pub struct RepoIndex {
    pub packages: Vec<RepoEntry>,
}

/// The subset of `MANIFEST.toml` needed for the index.
#[derive(Debug, serde::Deserialize)]
pub struct ManifestMeta {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub arch: String,
}

/// Walk `packages_dir` for `.anxpkg` files and return their index entries.
pub fn generate(packages_dir: &Path, base_url: &str) -> Result<Vec<RepoEntry>> {
    let mut entries = Vec::new();

    for entry in WalkDir::new(packages_dir).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("anxpkg") {
            continue;
        }

        let meta = read_manifest(path)?;
        let checksum = sha256_file(path)?;
        let size = path.metadata()?.len();

        // Download URL relative to the configured base URL.
        let rel = path.strip_prefix(packages_dir).context("path escaping repo dir")?;
        let url = format!("{}/{}", base_url.trim_end_matches('/'), rel.to_string_lossy());

        entries.push(RepoEntry {
            name: meta.name,
            version: meta.version,
            description: meta.description,
            arch: meta.arch,
            url,
            checksum,
            size,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Extract and parse the `MANIFEST.toml` embedded in an `.anxpkg` archive.
pub fn read_manifest_public(pkg_path: &Path) -> Result<ManifestMeta> {
    read_manifest(pkg_path)
}

/// Extract and parse the `MANIFEST.toml` embedded in an `.anxpkg` archive.
fn read_manifest(pkg_path: &Path) -> Result<ManifestMeta> {
    let file =
        File::open(pkg_path).with_context(|| format!("Failed to open {}", pkg_path.display()))?;
    let decoder = zstd::Decoder::new(file)
        .with_context(|| format!("Not a zstd archive: {}", pkg_path.display()))?;
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("Failed to read archive entries")? {
        let mut entry = entry.context("Invalid archive entry")?;
        if entry.path()?.to_string_lossy() == "MANIFEST.toml" {
            let mut raw = String::new();
            entry.read_to_string(&mut raw)?;
            return toml::from_str(&raw)
                .with_context(|| format!("Invalid MANIFEST in {}", pkg_path.display()));
        }
    }

    anyhow::bail!("No MANIFEST.toml in {}", pkg_path.display())
}

/// SHA-256 of the archive bytes (used as the client-side checksum).
fn sha256_file(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut file = File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
