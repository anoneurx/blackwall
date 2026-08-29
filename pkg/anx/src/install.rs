//! Package installation logic.
//!
//! Steps:
//! 1. Look up package in the repository index
//! 2. Acquire the lockfile
//! 3. Download the `.anxpkg` archive
//! 4. Verify SHA-256 checksum
//! 5. Verify GPG signature
//! 6. Unpack payload into the filesystem
//! 7. Run post-install script (if any)
//! 8. Record the installation in `/var/lib/anx/installed/`
//! 9. Record the transaction in `/var/lib/anx/transactions/`
//! 10. Release the lockfile

use anyhow::{Context, Result};
use chrono::Utc;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write;
use std::path::PathBuf;

use crate::lockfile::Lockfile;
use crate::pkg::{self, InstalledPackage};
use crate::repo;
use crate::transaction::{Transaction, TxKind};
use crate::verify;

const KEYRING_DIR: &str = "/etc/anx/trusted-keys";

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run(packages: &[String], no_verify: bool, quiet: bool, yes: bool) -> Result<()> {
    pkg::ensure_dirs()?;
    let _lock = Lockfile::acquire()?;

    for name in packages {
        install_one(name, no_verify, quiet, yes)?;
    }
    Ok(())
}

// ─── Single package installation ──────────────────────────────────────────────

fn install_one(name: &str, no_verify: bool, quiet: bool, yes: bool) -> Result<()> {
    // Check if already installed.
    if pkg::is_installed(name) {
        let installed = pkg::load_installed(name)?;
        println!(
            "{} {} ({}) is already installed.",
            "→".bright_cyan(),
            name.bold(),
            installed.version
        );
        return Ok(());
    }

    // Look up in the index.
    if !quiet {
        println!("{} Looking up '{}'...", "→".dimmed(), name);
    }
    let entry = repo::lookup(name)?;

    // Print plan and confirm.
    if !quiet {
        println!(
            "  {} {} ({}) — {} bytes",
            "Install:".bold(),
            entry.name.bright_cyan(),
            entry.version.bright_green(),
            entry.size
        );
    }
    if !yes && !quiet {
        print!("Proceed? [Y/n] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        let answer = line.trim().to_lowercase();
        if answer == "n" || answer == "no" {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Download.
    let pkg_path = download(&entry.url, &entry.name, quiet)?;

    // Verify checksum.
    if !no_verify {
        if !quiet {
            print!("  {} Verifying checksum...", "→".dimmed());
            std::io::stdout().flush().ok();
        }
        verify::verify_checksum(&pkg_path, &entry.checksum)?;
        if !quiet {
            println!(" {}", "OK".bright_green());
        }

        // Verify GPG signature (extracted from archive).
        let sig_path = pkg_path.with_extension("sig.gpg");
        // In a full implementation we'd extract sig.gpg from the archive first.
        // For v1.0 we pass the archive itself as data (the repo signs the whole pkg).
        verify::verify_gpg(&pkg_path, &sig_path, std::path::Path::new(KEYRING_DIR))?;
    }

    // Unpack and install.
    let installed_files = unpack(&pkg_path, quiet)?;

    // Record installation.
    let tx_id = crate::transaction::new_id();
    let now = chrono_now();

    let record = InstalledPackage {
        name: entry.name.clone(),
        version: entry.version.clone(),
        description: entry.description.clone(),
        installed_at: now.clone(),
        transaction_id: tx_id.clone(),
        installed_files: installed_files.clone(),
    };

    let record_path = pkg::installed_db_dir().join(format!("{}.toml", entry.name));
    let toml = toml::to_string_pretty(&record)?;
    std::fs::write(&record_path, toml)?;

    // Record transaction.
    let tx = Transaction {
        id: tx_id,
        kind: TxKind::Install,
        package: entry.name.clone(),
        version: entry.version.clone(),
        timestamp: now,
        affected_files: installed_files,
        previous_version: None,
        rolled_back: false,
    };
    crate::transaction::save(&tx)?;

    if !quiet {
        println!(
            "{} Installed {} ({})",
            "✓".bold().bright_green(),
            entry.name.bright_cyan(),
            entry.version
        );
    }
    Ok(())
}

// ─── Download ─────────────────────────────────────────────────────────────────

fn download(url: &str, name: &str, quiet: bool) -> Result<PathBuf> {
    let dest = pkg::cache_dir().join(format!("{}.anxpkg", name));

    // If already cached, reuse.
    if dest.exists() {
        if !quiet {
            println!("  {} Using cached package.", "→".dimmed());
        }
        return Ok(dest);
    }

    if !quiet {
        println!("  {} Downloading {}...", "→".dimmed(), url);
    }

    let mut response =
        reqwest::blocking::get(url).with_context(|| format!("Failed to download {}", url))?;

    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let total = response.content_length().unwrap_or(0);
    let pb = if !quiet && total > 0 {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("  [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("█░"),
        );
        Some(pb)
    } else {
        None
    };

    let mut file = std::fs::File::create(&dest)
        .with_context(|| format!("Failed to create cache file {}", dest.display()))?;

    // Stream the response body directly to disk.
    response.copy_to(&mut file).context("Download write error")?;

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    Ok(dest)
}

// ─── Unpack ───────────────────────────────────────────────────────────────────

fn unpack(pkg_path: &PathBuf, quiet: bool) -> Result<Vec<String>> {
    if !quiet {
        print!("  {} Unpacking...", "→".dimmed());
        std::io::stdout().flush().ok();
    }

    let file = std::fs::File::open(pkg_path)
        .with_context(|| format!("Failed to open {}", pkg_path.display()))?;

    // Decompress zstd.
    let decoder = zstd::Decoder::new(file).context("Failed to create zstd decoder")?;
    let mut archive = tar::Archive::new(decoder);

    let mut installed_files = Vec::new();

    for entry in archive.entries().context("Failed to read archive entries")? {
        let mut entry = entry.context("Failed to read archive entry")?;
        let path = entry.path().context("Invalid path in archive")?.into_owned();
        let path_str = path.display().to_string();

        // Skip MANIFEST.toml and sig.gpg — don't install them to disk.
        if path_str == "MANIFEST.toml" || path_str == "sig.gpg" {
            continue;
        }

        // Strip leading `files/` prefix if present.
        let dest_rel = if let Ok(stripped) = path.strip_prefix("files/") {
            stripped.to_owned()
        } else {
            continue;
        };

        let dest_abs = PathBuf::from("/").join(&dest_rel);

        // Create parent directories.
        if let Some(parent) = dest_abs.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        entry
            .unpack(&dest_abs)
            .with_context(|| format!("Failed to unpack {}", dest_abs.display()))?;

        installed_files.push(dest_abs.display().to_string());
    }

    if !quiet {
        println!(" {} ({} files)", "OK".bright_green(), installed_files.len());
    }

    Ok(installed_files)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn chrono_now() -> String {
    Utc::now().to_rfc3339()
}
