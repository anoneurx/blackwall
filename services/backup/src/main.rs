//! # bwbackup — Backup Manager CLI
//!
//! - `bwbackup now [name]`          — run a backup now
//! - `bwbackup list`                — list backups (JSON with `--json`)
//! - `bwbackup restore <name>`      — restore tracked paths from a backup
//! - `bwbackup delete <name>`       — delete a backup
//! - `bwbackup serve`               — run scheduled backups in a loop
//!
//! Config: `--config` (default /etc/bwbackup/backup.toml).

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

use bwbackup::Config;

#[derive(Parser)]
#[command(
    name = "bwbackup",
    version = env!("CARGO_PKG_VERSION"),
    about = "Black Wall Core backup manager"
)]
struct Cli {
    /// Configuration file
    #[arg(short, long, default_value = bwbackup::DEFAULT_CONFIG)]
    config: PathBuf,

    /// State/config dir override for metadata
    #[arg(long, env = "BWBACKUP_STATE_DIR")]
    state_dir: Option<PathBuf>,

    /// Emit JSON output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a backup now
    Now {
        /// Optional backup label (default: timestamp)
        name: Option<String>,
    },
    /// List backups
    List,
    /// Restore paths from a backup
    Restore { name: String },
    /// Delete a backup
    Delete { name: String },
    /// Run scheduled backups
    Serve,
}

fn ts_name() -> String {
    chrono::Utc::now().format("auto-%Y%m%d-%H%M%S").to_string()
}

fn to_human(b: &bwbackup::Backup) -> String {
    format!(
        "{:<22} {}  {:>10} B  {}{}",
        b.name,
        b.created_at,
        b.size_bytes,
        if b.remote { "[remote] " } else { "" },
        b.paths.join(",")
    )
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load(&cli.config.to_string_lossy());
    if let Some(sd) = &cli.state_dir {
        cfg.state_dir = sd.display().to_string();
    }

    match cli.command {
        Command::Now { name } => {
            let name = name.unwrap_or_else(ts_name);
            let meta = bwbackup::create(&cfg, &name)?;
            if cli.json {
                println!("{}", serde_json::to_string(&meta)?);
            } else {
                println!(
                    "backup '{}' created ({} bytes, {} path(s){})",
                    meta.name,
                    meta.size_bytes,
                    meta.paths.len(),
                    if meta.remote { ", remote" } else { "" }
                );
            }
        }
        Command::List => {
            let bks = bwbackup::list(&cfg)?;
            if cli.json {
                println!("{}", serde_json::to_string(&bks)?);
            } else {
                for b in bks {
                    println!("{}", to_human(&b));
                }
            }
        }
        Command::Restore { name } => {
            let meta = bwbackup::restore(&cfg, &name)?;
            if cli.json {
                println!("{}", serde_json::to_string(&meta)?);
            } else {
                println!("restored backup '{}'", meta.name);
            }
        }
        Command::Delete { name } => {
            let meta = bwbackup::delete(&cfg, &name)?;
            if cli.json {
                println!("{}", serde_json::to_string(&meta)?);
            } else {
                println!("deleted backup '{}'", meta.name);
            }
        }
        Command::Serve => {
            if cfg.interval_secs == 0 {
                anyhow::bail!("interval_secs is 0; nothing to schedule");
            }
            println!("bwbackup serve: backing up every {}s", cfg.interval_secs);
            loop {
                let name = ts_name();
                match bwbackup::create(&cfg, &name) {
                    Ok(m) => println!("scheduled backup '{}' created", m.name),
                    Err(e) => eprintln!("scheduled backup failed: {:#}", e),
                }
                std::thread::sleep(Duration::from_secs(cfg.interval_secs));
            }
        }
    }
    Ok(())
}
