//! Package removal logic with rollback-safe file deletion.

use anyhow::{Context, Result};
use chrono::Utc;
use colored::Colorize;
use std::io::Write;

use crate::lockfile::Lockfile;
use crate::pkg;
use crate::transaction::{Transaction, TxKind};

pub fn run(packages: &[String], quiet: bool, yes: bool) -> Result<()> {
    pkg::ensure_dirs()?;
    let _lock = Lockfile::acquire()?;

    for name in packages {
        remove_one(name, quiet, yes)?;
    }
    Ok(())
}

fn remove_one(name: &str, quiet: bool, yes: bool) -> Result<()> {
    if !pkg::is_installed(name) {
        anyhow::bail!("Package '{}' is not installed", name);
    }

    let installed = pkg::load_installed(name)?;

    if !quiet {
        println!(
            "  {} Remove: {} ({})",
            "!".bold().bright_red(),
            installed.name.bold(),
            installed.version
        );
        println!("    {} files will be removed.", installed.installed_files.len());
    }

    if !yes && !quiet {
        print!("Proceed? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        let answer = line.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Remove installed files.
    let mut removed = Vec::new();
    for file in &installed.installed_files {
        let path = std::path::Path::new(file);
        if path.exists() {
            std::fs::remove_file(path).with_context(|| format!("Failed to remove {}", file))?;
            removed.push(file.clone());
            if !quiet {
                println!("  {} {}", "removed".bright_red(), file);
            }
        }
    }

    // Run pre-remove script if present (read from manifest).
    // (Manifest is embedded in the archive — for v1.0 we skip this.)

    // Remove the installed record.
    let record_path = pkg::installed_db_dir().join(format!("{}.toml", name));
    let _ = std::fs::remove_file(&record_path);

    // Record the transaction.
    let tx_id = crate::transaction::new_id();
    let tx = Transaction {
        id: tx_id,
        kind: TxKind::Remove,
        package: installed.name.clone(),
        version: installed.version.clone(),
        timestamp: Utc::now().to_rfc3339(),
        affected_files: removed,
        previous_version: None,
        rolled_back: false,
    };
    crate::transaction::save(&tx)?;

    if !quiet {
        println!(
            "{} Removed {} ({})",
            "✓".bold().bright_green(),
            installed.name.bright_cyan(),
            installed.version
        );
    }
    Ok(())
}
