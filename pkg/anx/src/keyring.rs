//! Trusted GPG keyring management.
//!
//! Keys live in `/etc/anx/trusted-keys/` as `.gpg` or `.asc` files.
//! Each file is named `<fingerprint>.<ext>`.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};

const KEYRING_DIR: &str = "/etc/anx/trusted-keys";

fn keyring_dir() -> PathBuf {
    PathBuf::from(KEYRING_DIR)
}

/// Add a trusted key from a file path.
pub fn add(path: &str) -> Result<()> {
    let src = Path::new(path);
    anyhow::ensure!(src.exists(), "Key file '{}' does not exist", path);

    // Import the key with gpg to get its fingerprint.
    let output = std::process::Command::new("gpg")
        .args(["--import", "--with-fingerprint", "--with-colons", path])
        .output()
        .context("gpg binary not found — cannot import key")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gpg import failed:\n{}", stderr);
    }

    // Extract fingerprint from gpg output.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let fingerprint = stdout
        .lines()
        .find(|l| l.starts_with("fpr:"))
        .and_then(|l| l.split(':').nth(9))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            src.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        });

    std::fs::create_dir_all(keyring_dir()).context("Failed to create keyring directory")?;

    let ext = src.extension().unwrap_or_default().to_string_lossy();
    let dest = keyring_dir().join(format!("{}.{}", fingerprint, ext));

    std::fs::copy(src, &dest)
        .with_context(|| format!("Failed to copy key to {}", dest.display()))?;

    println!("{} Trusted key added: {}", "✓".bold().bright_green(), fingerprint.bright_cyan());
    Ok(())
}

/// List all trusted keys.
pub fn list() -> Result<()> {
    let dir = keyring_dir();
    if !dir.exists() {
        println!("No trusted keys enrolled.");
        return Ok(());
    }

    let keys: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "gpg" || x == "asc"))
        .collect();

    if keys.is_empty() {
        println!("No trusted keys enrolled.");
        return Ok(());
    }

    println!("{} trusted key(s):", keys.len());
    for key in &keys {
        let name =
            key.path().file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        println!("  {} {}", "•".bright_cyan(), name);
    }
    Ok(())
}

/// Remove a trusted key by fingerprint.
pub fn remove(fingerprint: &str) -> Result<()> {
    let dir = keyring_dir();
    let mut found = false;

    for ext in ["gpg", "asc"] {
        let path = dir.join(format!("{}.{}", fingerprint, ext));
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to remove key {}", path.display()))?;
            found = true;
            break;
        }
    }

    if !found {
        anyhow::bail!("Key '{}' not found in keyring", fingerprint);
    }

    println!("{} Removed key: {}", "✓".bold().bright_green(), fingerprint.bright_cyan());
    Ok(())
}
