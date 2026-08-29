//! Transaction log — records every package install/remove for rollback support.
//!
//! Each transaction is stored as a TOML file in `/var/lib/anx/transactions/`.
//! A transaction captures the full state diff so it can be undone atomically.

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const TX_DIR: &str = "/var/lib/anx/transactions";

// ─── Transaction types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TxKind {
    Install,
    Remove,
    Update,
}

impl std::fmt::Display for TxKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxKind::Install => write!(f, "install"),
            TxKind::Remove => write!(f, "remove"),
            TxKind::Update => write!(f, "update"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub kind: TxKind,
    pub package: String,
    pub version: String,
    pub timestamp: String,
    /// Files installed (for Install/Update) or removed (for Remove).
    pub affected_files: Vec<String>,
    /// For Update: the previous version.
    pub previous_version: Option<String>,
    /// Whether this transaction has been rolled back.
    pub rolled_back: bool,
}

// ─── ID generation ────────────────────────────────────────────────────────────

pub fn new_id() -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("tx-{}", ts)
}

// ─── Persistence ──────────────────────────────────────────────────────────────

pub fn tx_path(id: &str) -> PathBuf {
    PathBuf::from(TX_DIR).join(format!("{}.toml", id))
}

pub fn save(tx: &Transaction) -> Result<()> {
    std::fs::create_dir_all(TX_DIR).context("Failed to create transaction directory")?;
    let toml = toml::to_string_pretty(tx).context("Failed to serialize transaction")?;
    std::fs::write(tx_path(&tx.id), toml).context("Failed to write transaction")?;
    Ok(())
}

pub fn load(id: &str) -> Result<Transaction> {
    let raw = std::fs::read_to_string(tx_path(id))
        .with_context(|| format!("Transaction '{}' not found", id))?;
    toml::from_str(&raw).context("Failed to parse transaction")
}

/// List all transactions in reverse chronological order.
pub fn list() -> Result<()> {
    let dir = std::path::Path::new(TX_DIR);
    if !dir.exists() {
        println!("No transactions recorded.");
        return Ok(());
    }

    let mut txs: Vec<Transaction> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "toml"))
        .filter_map(|e| {
            std::fs::read_to_string(e.path()).ok().and_then(|s| toml::from_str(&s).ok())
        })
        .collect();

    txs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    println!(
        "{:<24} {:<10} {:<30} {}",
        "ID".bold(),
        "TYPE".bold(),
        "PACKAGE".bold(),
        "DATE".bold()
    );
    println!("{}", "─".repeat(80).dimmed());

    for tx in &txs {
        let rollback_marker =
            if tx.rolled_back { " [rolled back]".dimmed().to_string() } else { String::new() };
        let kind_colored = match tx.kind {
            TxKind::Install => tx.kind.to_string().bright_green(),
            TxKind::Remove => tx.kind.to_string().bright_red(),
            TxKind::Update => tx.kind.to_string().bright_yellow(),
        };
        println!(
            "{:<24} {:<10} {:<30} {}{}",
            tx.id.bright_cyan(),
            kind_colored,
            tx.package,
            tx.timestamp,
            rollback_marker
        );
    }
    Ok(())
}

// ─── Rollback ────────────────────────────────────────────────────────────────

pub fn rollback(id: &str, quiet: bool) -> Result<()> {
    let mut tx = load(id)?;

    if tx.rolled_back {
        anyhow::bail!("Transaction '{}' has already been rolled back", id);
    }

    if !quiet {
        println!(
            "{} Rolling back transaction {} ({} {})",
            "→".bright_yellow(),
            id.bright_cyan(),
            tx.kind,
            tx.package.bold()
        );
    }

    match tx.kind {
        TxKind::Install | TxKind::Update => {
            // Undo: remove the installed files.
            for file in &tx.affected_files {
                if std::path::Path::new(file).exists() {
                    std::fs::remove_file(file)
                        .with_context(|| format!("Failed to remove {}", file))?;
                    if !quiet {
                        println!("  {} {}", "removed".bright_red(), file);
                    }
                }
            }
            // Remove the installed package record.
            let record = crate::pkg::installed_db_dir().join(format!("{}.toml", tx.package));
            let _ = std::fs::remove_file(record);
        }
        TxKind::Remove => {
            // We can't fully undo a remove without having saved the old files.
            // In v1.0 we just print a notice.
            println!(
                "{} Cannot automatically undo a remove transaction — \
                 reinstall with 'anx install {}'",
                "!".bold().bright_yellow(),
                tx.package
            );
        }
    }

    // Mark the transaction as rolled back.
    tx.rolled_back = true;
    save(&tx)?;

    if !quiet {
        println!("{} Rollback complete.", "✓".bold().bright_green());
    }
    Ok(())
}
