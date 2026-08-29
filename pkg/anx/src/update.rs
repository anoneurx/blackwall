//! Automatic update logic — refreshes the index and upgrades installed packages.

use anyhow::Result;
use colored::Colorize;

use crate::lockfile::Lockfile;
use crate::pkg;
use crate::repo;

pub fn run(quiet: bool) -> Result<()> {
    pkg::ensure_dirs()?;
    let _lock = Lockfile::acquire()?;

    // Step 1: Refresh the index.
    if !quiet {
        println!("{} Refreshing package index...", "→".dimmed());
    }
    repo::refresh(quiet)?;

    // Step 2: Scan installed packages and compare versions.
    let dir = pkg::installed_db_dir();
    if !dir.exists() {
        println!("No packages installed.");
        return Ok(());
    }

    let installed: Vec<pkg::InstalledPackage> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "toml"))
        .filter_map(|e| {
            std::fs::read_to_string(e.path()).ok().and_then(|s| toml::from_str(&s).ok())
        })
        .collect();

    let mut upgrades: Vec<(pkg::InstalledPackage, pkg::RepoEntry)> = Vec::new();

    for pkg_rec in &installed {
        if let Ok(entry) = repo::lookup(&pkg_rec.name) {
            if entry.version != pkg_rec.version {
                upgrades.push((pkg_rec.clone(), entry));
            }
        }
    }

    if upgrades.is_empty() {
        if !quiet {
            println!("{} All packages are up to date.", "✓".bold().bright_green());
        }
        return Ok(());
    }

    if !quiet {
        println!("{} upgrades available:", upgrades.len());
        for (installed, new) in &upgrades {
            println!(
                "  {} {} {} → {}",
                "↑".bright_yellow(),
                installed.name.bright_cyan(),
                installed.version.dimmed(),
                new.version.bright_green()
            );
        }
        println!();
    }

    // Step 3: Install each upgrade.
    for (installed_pkg, _entry) in &upgrades {
        // Remove old version first.
        crate::remove::run(&[installed_pkg.name.clone()], quiet, true)?;
        // Install new version.
        crate::install::run(&[installed_pkg.name.clone()], false, quiet, true)?;
    }

    if !quiet {
        println!("{} {} package(s) upgraded.", "✓".bold().bright_green(), upgrades.len());
    }
    Ok(())
}
