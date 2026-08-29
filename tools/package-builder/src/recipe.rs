//! Recipe definition — the declarative source from which `.anxpkg` archives
//! are built by `bw-pkg`.
//!
//! A recipe lives at `packages/<category>/<name>/recipe.toml` and describes the
//! package metadata (mirrored into the archive's `MANIFEST.toml`), how to obtain
//! the payload, and any post-install / pre-remove hooks.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level recipe document.
#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub package: Package,

    #[serde(default)]
    pub source: Source,

    #[serde(default)]
    pub install: Hooks,
}

/// Metadata copied verbatim into the `.anxpkg` `MANIFEST.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: String,

    /// Target CPU architecture. Defaults to `x86_64`.
    #[serde(default = "default_arch")]
    pub architecture: String,

    #[serde(default = "default_license")]
    pub license: String,

    #[serde(default)]
    pub homepage: Option<String>,

    #[serde(default)]
    pub maintainer: Option<String>,

    /// Package category (containers, databases, web, ...). Informational.
    #[serde(default)]
    #[allow(dead_code)]
    pub category: Option<String>,

    /// Runtime dependencies: package name → version constraint.
    #[serde(default)]
    pub depends: HashMap<String, String>,
}

/// How the package payload is obtained.
#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    /// `dir`       — use a local staging directory (the default).
    /// `prebuilt`  — use a local staging directory of prebuilt binaries.
    ///               If `url` is set and no staged files exist yet, the file is
    ///               downloaded from `url` into `payload_dir/<dest>`.
    /// `tarball`   — download an archive, verify it, extract it into `payload/`.
    #[serde(rename = "type", default)]
    pub source_type: String,

    /// For `tarball` sources: the archive URL.
    /// For `prebuilt` sources: optional single-file (binary) download URL used
    /// to stage the payload automatically when none exists yet.
    #[serde(default)]
    pub url: Option<String>,

    /// For `tarball` sources: expected SHA-256 of the archive.
    /// For `prebuilt` sources: optional expected SHA-256 of the downloaded file.
    /// (`archive_sha256` is accepted as an alias for older recipes.)
    #[serde(default, alias = "archive_sha256")]
    pub sha256: Option<String>,

    /// For `prebuilt` single-file downloads: destination path inside the
    /// payload (relative to filesystem root). Defaults to `usr/bin/<name>`.
    #[serde(default)]
    pub dest: Option<String>,

    /// Directory holding the staged payload (relative to the recipe directory).
    /// For `tarball` sources this is the extraction target.
    #[serde(default = "default_payload_dir")]
    pub payload_dir: String,

    /// When `true` (tarball sources), strip a single enclosing top-level
    /// directory after extraction so payload files install directly under `/`.
    #[serde(default)]
    pub flatten: bool,
}

impl Default for Source {
    fn default() -> Self {
        Source {
            source_type: default_source_type(),
            url: None,
            sha256: None,
            dest: None,
            payload_dir: default_payload_dir(),
            flatten: false,
        }
    }
}

fn default_source_type() -> String {
    "dir".to_string()
}

fn default_payload_dir() -> String {
    "payload".to_string()
}

/// Install-time hooks embedded into the manifest.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Hooks {
    /// Post-install shell snippet (run as root, from `/`).
    pub post: Option<String>,

    /// Pre-remove shell snippet (run as root, from `/`).
    pub pre_remove: Option<String>,

    /// Optional `bwinit` service unit written to `/etc/bwinit/services/`.
    pub unit: Option<String>,
}

fn default_arch() -> String {
    "x86_64".to_string()
}

fn default_license() -> String {
    "unknown".to_string()
}

impl Recipe {
    /// Load and parse a recipe from `recipe.toml`.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read recipe {}", path.display()))?;
        let recipe: Recipe =
            toml::from_str(&raw).with_context(|| format!("Invalid recipe {}", path.display()))?;
        recipe.validate()?;
        Ok(recipe)
    }

    fn validate(&self) -> Result<()> {
        if self.package.name.is_empty() {
            anyhow::bail!("recipe.package.name must not be empty");
        }
        if self.package.version.is_empty() {
            anyhow::bail!("recipe.package.version must not be empty");
        }
        match self.source.source_type.as_str() {
            "dir" | "prebuilt" => {}
            "tarball" => {
                if self.source.url.is_none() {
                    anyhow::bail!("recipe.source.url is required when source.type = 'tarball'");
                }
            }
            other => anyhow::bail!(
                "unknown recipe.source.type '{}' (expected 'dir', 'prebuilt' or 'tarball')",
                other
            ),
        }
        Ok(())
    }
}
