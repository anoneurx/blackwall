//! # bwsnap — Snapshot Manager CLI
//!
//! Usage:
//! - `bwsnap create <name> [--path ...]`  — snapshot configured paths
//! - `bwsnap list`                        — list snapshots (JSON with `--json`)
//! - `bwsnap restore <name>`              — restore files to original locations
//! - `bwsnap delete <name>`               — remove a snapshot
//!
//! Store defaults to `BWSNAP_STORE` or `/var/lib/bwsnap/snapshots`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

const DEFAULT_STORE: &str = bwsnap::DEFAULT_STORE;

#[derive(Parser)]
#[command(
    name = "bwsnap",
    version = env!("CARGO_PKG_VERSION"),
    about = "Black Wall Core snapshot manager"
)]
struct Cli {
    /// Snapshot store directory
    #[arg(short, long, env = "BWSNAP_STORE", default_value = DEFAULT_STORE)]
    store: PathBuf,

    /// Emit JSON output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a snapshot
    Create {
        /// Snapshot name
        name: String,
        /// Path(s) to capture (repeatable). Defaults to the standard set.
        #[arg(short, long)]
        path: Vec<String>,
    },
    /// List snapshots
    List,
    /// Restore a snapshot
    Restore { name: String },
    /// Delete a snapshot
    Delete { name: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Create { name, path } => {
            let paths = if path.is_empty() {
                bwsnap::DEFAULT_PATHS.iter().map(|s| s.to_string()).collect()
            } else {
                path
            };
            let meta = bwsnap::create(&cli.store, &name, &paths)?;
            if cli.json {
                println!("{}", serde_json::to_string(&meta)?);
            } else {
                println!(
                    "created snapshot '{}' ({} bytes, {} path(s))",
                    meta.name,
                    meta.size_bytes,
                    meta.paths.len()
                );
            }
        }
        Command::List => {
            let snaps = bwsnap::list(&cli.store)?;
            if cli.json {
                println!("{}", serde_json::to_string(&snaps)?);
            } else {
                for s in snaps {
                    println!(
                        "{:<20} {}  {:>10} B  {}",
                        s.name,
                        s.created_at,
                        s.size_bytes,
                        s.paths.join(",")
                    );
                }
            }
        }
        Command::Restore { name } => {
            let meta = bwsnap::restore(&cli.store, &name)?;
            if cli.json {
                println!("{}", serde_json::to_string(&meta)?);
            } else {
                println!("restored snapshot '{}'", meta.name);
            }
        }
        Command::Delete { name } => {
            let meta = bwsnap::delete(&cli.store, &name)?;
            if cli.json {
                println!("{}", serde_json::to_string(&meta)?);
            } else {
                println!("deleted snapshot '{}'", meta.name);
            }
        }
    }
    Ok(())
}
