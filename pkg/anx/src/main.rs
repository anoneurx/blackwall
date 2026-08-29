//! # anx — Black Wall Core Package Manager
//!
//! `anx` is the native CLI package manager for Black Wall Core.
//!
//! ## Commands
//! - `anx install <pkg>` — download, verify, and install a package
//! - `anx remove <pkg>`  — uninstall a package (with rollback)
//! - `anx update`        — refresh index and upgrade installed packages
//! - `anx search <term>` — search the repository index
//! - `anx list`          — list installed packages
//! - `anx info <pkg>`    — show package metadata
//! - `anx rollback <id>` — roll back a transaction

mod install;
mod keyring;
mod lockfile;
mod pkg;
mod remove;
mod repo;
mod transaction;
mod update;
mod verify;

use clap::{Parser, Subcommand};
use colored::Colorize;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ─── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "anx",
    version = VERSION,
    about = "Black Wall Core Package Manager",
    long_about = "anx manages software packages on Black Wall Core.\n\nPackages are cryptographically verified before installation."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Suppress progress output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Force operation without confirmation
    #[arg(short, long, global = true)]
    yes: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Install one or more packages
    Install {
        /// Package name(s) to install
        #[arg(required = true)]
        packages: Vec<String>,

        /// Skip signature verification (DANGEROUS)
        #[arg(long, hide = true)]
        no_verify: bool,
    },

    /// Remove one or more packages
    Remove {
        /// Package name(s) to remove
        #[arg(required = true)]
        packages: Vec<String>,
    },

    /// Update all installed packages
    Update,

    /// Refresh the package index without upgrading
    Refresh,

    /// Search the package repository
    Search {
        /// Search term
        term: String,
    },

    /// List installed packages
    List,

    /// Show package information
    Info {
        /// Package name
        package: String,
    },

    /// Roll back a transaction
    Rollback {
        /// Transaction ID to roll back (use `anx list-transactions` to find IDs)
        id: String,
    },

    /// List transaction history
    #[command(name = "list-transactions")]
    ListTransactions,

    /// Manage repository channels
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },

    /// Pin a package to a specific version
    Pin {
        /// Package name
        package: String,
        /// Version to pin to (defaults to the indexed version)
        version: Option<String>,
    },

    /// Remove a package pin
    Unpin {
        /// Package name
        package: String,
    },

    /// Manage trusted signing keys
    Key {
        #[command(subcommand)]
        action: KeyAction,
    },
}

#[derive(Subcommand)]
enum RepoAction {
    /// Add a channel to the repository config
    Add {
        /// Channel name (e.g. stable)
        name: String,
        /// Channel base URL
        url: String,
        /// Channel priority (higher wins; default 10)
        #[arg(long)]
        priority: Option<u8>,
    },
    /// Remove a channel
    Remove { name: String },
    /// List configured channels
    List,
}

#[derive(Subcommand)]
enum KeyAction {
    /// Add a trusted key
    Add {
        /// Path to GPG public key file
        path: String,
    },
    /// List trusted keys
    List,
    /// Remove a trusted key by fingerprint
    Remove { fingerprint: String },
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // Print banner.
    if !cli.quiet {
        println!(
            "{} {} — {}",
            "anx".bold().bright_cyan(),
            VERSION.dimmed(),
            "Black Wall Package Manager".dimmed()
        );
        println!();
    }

    let result = match cli.command {
        Commands::Install { packages, no_verify } => {
            install::run(&packages, no_verify, cli.quiet, cli.yes)
        }
        Commands::Remove { packages } => remove::run(&packages, cli.quiet, cli.yes),
        Commands::Update => update::run(cli.quiet),
        Commands::Refresh => repo::refresh(cli.quiet),
        Commands::Search { term } => repo::search(&term),
        Commands::List => pkg::list_installed(),
        Commands::Info { package } => pkg::info(&package),
        Commands::Rollback { id } => transaction::rollback(&id, cli.quiet),
        Commands::ListTransactions => transaction::list(),
        Commands::Repo { action } => match action {
            RepoAction::Add { name, url, priority } => repo::add_channel(&name, &url, priority),
            RepoAction::Remove { name } => repo::remove_channel(&name),
            RepoAction::List => repo::list_channels(),
        },
        Commands::Pin { package, version } => repo::pin(&package, version),
        Commands::Unpin { package } => repo::unpin(&package),
        Commands::Key { action } => match action {
            KeyAction::Add { path } => keyring::add(&path),
            KeyAction::List => keyring::list(),
            KeyAction::Remove { fingerprint } => keyring::remove(&fingerprint),
        },
    };

    if let Err(e) = result {
        eprintln!("{} {}", "error:".bold().red(), e);
        std::process::exit(1);
    }
}
