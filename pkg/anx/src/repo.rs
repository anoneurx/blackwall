//! Repository index management (v2.0: channels + pinning).
//!
//! The repository is organised into **channels**. Each channel points at a
//! repository base URL and serves an `index.toml`. Channels have a priority
//! (higher wins). `anx refresh` fetches every enabled channel and merges them;
//! `anx install` resolves each package from the highest-priority channel that
//! carries it. Packages can be **pinned** to a specific version, after which
//! only that version is installed/kept.
//!
//! Config: `/etc/anx/anx.toml`
//! ```toml
//! default_channel = "stable"
//!
//! [[channels]]
//! name     = "stable"
//! url      = "http://localhost:8484"
//! priority = 10
//! enabled  = true
//!
//! [pin]
//! nginx = "1.27.2"
//! ```

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::pkg::RepoEntry;

const INDEX_CACHE_PATH: &str = "/var/cache/anx/index.toml";
const CONFIG_PATH: &str = "/etc/anx/anx.toml";
pub const DEFAULT_REPO_URL: &str = "http://localhost:8484";

// ─── Repository config ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub name: String,
    pub url: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnxConfig {
    #[serde(default)]
    pub default_channel: Option<String>,
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// Package name → pinned version.
    #[serde(default)]
    pub pin: HashMap<String, String>,
    /// Legacy single-repo key (still honoured when `channels` is empty).
    #[serde(default)]
    pub repo_url: Option<String>,
}

fn default_priority() -> u8 {
    10
}
fn default_true() -> bool {
    true
}

fn load_config() -> Result<AnxConfig> {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(raw) => Ok(toml::from_str(&raw).context("malformed /etc/anx/anx.toml")?),
        Err(_) => Ok(AnxConfig::default()),
    }
}

fn save_config(cfg: &AnxConfig) -> Result<()> {
    if let Some(parent) = std::path::Path::new(CONFIG_PATH).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(CONFIG_PATH, toml::to_string(cfg)?)?;
    Ok(())
}

/// Resolve the effective channels: config `channels`, else a single stable
/// channel from the legacy `repo_url` (or the default repository).
fn channels(cfg: &AnxConfig) -> Vec<Channel> {
    if !cfg.channels.is_empty() {
        return cfg.channels.clone();
    }
    let url = cfg.repo_url.clone().unwrap_or_else(|| DEFAULT_REPO_URL.to_string());
    vec![Channel { name: "stable".to_string(), url, priority: default_priority(), enabled: true }]
}

/// Highest-priority enabled channel (used by server docs / tooling).
pub fn default_channel_name(cfg: &AnxConfig) -> String {
    cfg.default_channel.as_deref().unwrap_or("stable").to_string()
}

// ─── Index ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct RepoIndex {
    #[serde(default)]
    packages: Vec<RepoEntry>,
}

fn fetch_index(url: &str) -> Result<Vec<RepoEntry>> {
    let url = format!("{}/index.toml", url.trim_end_matches('/'));
    let response = reqwest::blocking::get(&url)
        .with_context(|| format!("Failed to fetch repository index from {}", url))?;
    if !response.status().is_success() {
        anyhow::bail!("Repository {} returned HTTP {}", url, response.status());
    }
    let body = response.text().context("Failed to read repository index")?;
    let idx: RepoIndex =
        toml::from_str(&body).with_context(|| format!("Malformed index at {}", url))?;
    Ok(idx.packages)
}

/// Download the index from every enabled channel, merge (highest priority
/// wins per package) and cache the combined result.
pub fn refresh(quiet: bool) -> Result<()> {
    let cfg = load_config()?;
    let chans = channels(&cfg);

    crate::pkg::ensure_dirs()?;

    let mut merged: HashMap<String, (RepoEntry, u8)> = HashMap::new();
    for ch in chans.iter().filter(|c| c.enabled) {
        if !quiet {
            println!(
                "{} {} (channel {}, priority {})",
                "Refreshing".dimmed(),
                ch.url.bright_cyan(),
                ch.name.bright_green(),
                ch.priority
            );
        }
        for entry in fetch_index(&ch.url)? {
            match merged.get(&entry.name) {
                Some((_, prio)) if *prio >= ch.priority => continue,
                _ => {
                    merged.insert(entry.name.clone(), (entry, ch.priority));
                }
            }
        }
    }

    if merged.is_empty() {
        anyhow::bail!("No packages found across enabled channels");
    }

    let mut packages: Vec<RepoEntry> = merged.into_values().map(|(e, _)| e).collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let count = packages.len();
    let body = toml::to_string(&RepoIndex { packages })?;
    std::fs::write(INDEX_CACHE_PATH, body).context("Failed to cache index")?;

    if !quiet {
        println!("{} Repository index refreshed ({} packages).", "✓".bold().bright_green(), count);
    }
    Ok(())
}

/// Load the cached combined index.
fn load_index() -> Result<Vec<RepoEntry>> {
    let raw = std::fs::read_to_string(INDEX_CACHE_PATH)
        .context("Index not found. Run 'anx refresh' first.")?;
    let idx: RepoIndex = toml::from_str(&raw).context("Cached index is malformed")?;
    Ok(idx.packages)
}

/// Version a package is pinned to, if any.
pub fn pinned_version(name: &str) -> Option<String> {
    load_config().ok().and_then(|c| c.pin.get(name).cloned())
}

/// Look up a package, honouring channel merge order (already collapsed) and
/// version pins.
pub fn lookup(name: &str) -> Result<RepoEntry> {
    let packages = load_index()?;
    if let Some(ver) = pinned_version(name) {
        return packages.into_iter().find(|p| p.name == name && p.version == ver).ok_or_else(
            || {
                anyhow::anyhow!(
                    "Package '{}' is pinned to {} but no such version exists in the repository",
                    name,
                    ver
                )
            },
        );
    }
    packages
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow::anyhow!("Package '{}' not found in repository", name))
}

/// Search for packages matching a term across the merged index.
pub fn search(term: &str) -> Result<()> {
    let packages = load_index()?;
    let matches: Vec<_> = packages
        .iter()
        .filter(|p| {
            p.name.contains(term) || p.description.to_lowercase().contains(&term.to_lowercase())
        })
        .collect();

    if matches.is_empty() {
        println!("No packages found matching '{}'.", term);
        return Ok(());
    }

    println!("{:<30} {:<20} {}", "NAME".bold(), "VERSION".bold(), "DESCRIPTION".bold());
    println!("{}", "─".repeat(80).dimmed());
    for pkg in matches {
        let installed_marker = if crate::pkg::is_installed(&pkg.name) {
            " [installed]".bright_green().to_string()
        } else {
            String::new()
        };
        let pin_marker = if pinned_version(&pkg.name).as_deref() == Some(pkg.version.as_str()) {
            " [pinned]".bright_yellow().to_string()
        } else {
            String::new()
        };
        println!(
            "{:<30} {:<20} {}{}{}",
            pkg.name.bright_cyan(),
            pkg.version.bright_green(),
            pkg.description,
            installed_marker,
            pin_marker
        );
    }
    Ok(())
}

// ─── Channel management ───────────────────────────────────────────────────────

/// Add a channel to the config.
pub fn add_channel(name: &str, url: &str, priority: Option<u8>) -> Result<()> {
    if name.is_empty() || url.is_empty() {
        anyhow::bail!("channel name and url are required");
    }
    let mut cfg = load_config()?;
    if cfg.channels.iter().any(|c| c.name == name) {
        anyhow::bail!("channel '{}' already exists", name);
    }
    cfg.channels.push(Channel {
        name: name.to_string(),
        url: url.to_string(),
        priority: priority.unwrap_or(default_priority()),
        enabled: true,
    });
    save_config(&cfg)?;
    Ok(())
}

/// Remove a channel from the config.
pub fn remove_channel(name: &str) -> Result<()> {
    let mut cfg = load_config()?;
    let before = cfg.channels.len();
    cfg.channels.retain(|c| c.name != name);
    if cfg.channels.len() == before {
        anyhow::bail!("channel '{}' does not exist", name);
    }
    save_config(&cfg)?;
    Ok(())
}

/// List configured channels.
pub fn list_channels() -> Result<()> {
    let cfg = load_config()?;
    let chans = channels(&cfg);
    if chans.is_empty() {
        println!("No channels configured.");
    }
    println!(
        "{:<16} {:<10} {:<36} {}",
        "NAME".bold(),
        "PRIORITY".bold(),
        "URL".bold(),
        "PINNED".bold()
    );
    println!("{}", "─".repeat(90).dimmed());
    for ch in &chans {
        println!(
            "{:<16} {:<10} {:<36} {}",
            ch.name.bright_cyan(),
            ch.priority.to_string().bright_green(),
            ch.url,
            if ch.enabled { "enabled" } else { "disabled" }
        );
    }
    println!("Default channel: {}", default_channel_name(&cfg));
    Ok(())
}

/// Pin a package to a version (defaults to the version in the index).
pub fn pin(name: &str, version: Option<String>) -> Result<()> {
    let ver = match version {
        Some(v) => v,
        None => lookup(name)?.version,
    };
    let mut cfg = load_config()?;
    cfg.pin.insert(name.to_string(), ver.clone());
    save_config(&cfg)?;
    println!("{} pinned to {}", name.bright_cyan(), ver.bright_green());
    Ok(())
}

/// Remove a package pin.
pub fn unpin(name: &str) -> Result<()> {
    let mut cfg = load_config()?;
    if cfg.pin.remove(name).is_none() {
        anyhow::bail!("'{}' is not pinned", name);
    }
    save_config(&cfg)?;
    println!("{} unpinned", name.bright_cyan());
    Ok(())
}
