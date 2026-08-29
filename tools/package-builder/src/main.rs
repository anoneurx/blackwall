//! # bw-pkg — Black Wall `.anxpkg` package builder
//!
//! Builds cryptographically-verified `.anxpkg` archives from declarative
//! `recipe.toml` files and generates the repository `index.toml` that the
//! `anx` package manager consumes.
//!
//! Usage:
//! ```text
//! bw-pkg check <recipe.toml>              # validate a recipe
//! bw-pkg build <recipe.toml> [-o <dir>] [--sign]
//! bw-pkg fetch <recipe.toml>              # stage payload without building
//! bw-pkg fetch-tree <packages-dir>        # stage payloads for every recipe
//! bw-pkg index <packages-dir> --base-url <url> [-o index.toml]
//! ```

mod build;
mod index;
mod recipe;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "bw-pkg",
    version = VERSION,
    about = "Black Wall Core package builder (.anxpkg) & repository index generator"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate a recipe.toml without building
    Check {
        /// Path to the recipe.toml
        recipe: PathBuf,
    },

    /// Build an .anxpkg archive from a recipe.toml
    Build {
        /// Path to the recipe.toml
        recipe: PathBuf,

        /// Output directory for the built archive
        #[arg(short, long, default_value = "packages/out")]
        out: PathBuf,

        /// Also produce a detached GPG signature (`--sign` requires a configured key)
        #[arg(long)]
        sign: bool,
    },

    /// Stage the payload for a recipe (download if source.url is set)
    Fetch {
        /// Path to the recipe.toml
        recipe: PathBuf,
    },

    /// Walk a package tree and stage every fetchable payload
    FetchTree {
        /// Root directory containing recipe.toml files
        root: PathBuf,
    },

    /// Build every buildable recipe in a tree into .anxpkg archives
    BuildTree {
        /// Root directory containing recipe.toml files
        root: PathBuf,

        /// Output directory for built archives
        #[arg(short, long, default_value = "packages/out")]
        out: PathBuf,

        /// Also produce detached GPG signatures
        #[arg(long)]
        sign: bool,
    },

    /// Check every recipe's source URL answers HEAD; flag missing SHA pins
    VerifyTree {
        /// Root directory containing recipe.toml files
        root: PathBuf,
    },

    /// Generate a repository index.toml from built .anxpkg files
    Index {
        /// Directory containing .anxpkg files (searched recursively)
        packages_dir: PathBuf,

        /// Base URL used to form package download links
        #[arg(long)]
        base_url: String,

        /// Output index.toml path (default: <packages_dir>/index.toml)
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();

    println!(
        "{} {} — {}",
        "bw-pkg".bold().bright_cyan(),
        VERSION.dimmed(),
        "Black Wall Package Builder".dimmed()
    );
    println!();

    if let Err(e) = run(cli) {
        eprintln!("{} {}", "error:".bold().red(), e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Check { recipe } => recipe::Recipe::load(&recipe)
            .map(|r| {
                println!(
                    "{} recipe {} v{} ({})",
                    "✓".bold().bright_green(),
                    r.package.name.bright_cyan(),
                    r.package.version.bright_green(),
                    r.package.architecture
                );
            })
            .context("recipe validation failed"),

        Commands::Build { recipe, out, sign } => {
            let pkg = build::build(&recipe, &out, sign)?;
            println!(
                "{} Built {} ({} bytes)",
                "✓".bold().bright_green(),
                pkg.display().to_string().bright_cyan(),
                pkg.metadata().map(|m| m.len()).unwrap_or(0)
            );
            Ok(())
        }

        Commands::Fetch { recipe } => build::fetch(&recipe).map(|fetched| {
            if fetched {
                println!("{} Payload staged for {}", "✓".bold().bright_green(), recipe.display());
            } else {
                println!(
                    "{} Payload already staged for {}",
                    "✓".bold().bright_green(),
                    recipe.display()
                );
            }
        }),

        Commands::FetchTree { root } => {
            let stats = build::fetch_tree(&root).context("fetch tree walk failed")?;
            println!(
                "{} Fetched: {}  cached: {}  no-url: {}  failed: {}",
                "✓".bold().bright_green(),
                stats.fetched,
                stats.cached,
                stats.no_url,
                stats.failed
            );
            if stats.failed > 0 {
                anyhow::bail!("{} payload fetch(es) failed", stats.failed);
            }
            Ok(())
        }

        Commands::BuildTree { root, out, sign } => {
            let stats = build::build_tree(&root, &out, sign).context("build tree walk failed")?;
            println!(
                "{} Built: {}  skipped (no payload): {}  failed: {}",
                "✓".bold().bright_green(),
                stats.built,
                stats.skipped_no_payload,
                stats.failed
            );
            if let Some(first) = stats.artifacts.first() {
                println!(
                    "  artifacts in {} ({} files)",
                    out.display(),
                    stats.artifacts.len()
                );
                let _ = first; // listed count is enough for the summary
            }
            if stats.failed > 0 {
                anyhow::bail!("{} recipe build(s) failed", stats.failed);
            }
            Ok(())
        }

        Commands::VerifyTree { root } => {
            let report = build::verify_tree(&root).context("verify tree walk failed")?;
            println!("{}", report.summary());
            for (path, why) in &report.url_broken {
                eprintln!("{} {}: {}", "broken:".bold().red(), path.display(), why);
            }
            for path in report.sha_missing.iter().take(20) {
                eprintln!("{} {} (no archive_sha256)", "unpinned:".bold().yellow(), path.display());
            }
            if !report.url_broken.is_empty() {
                anyhow::bail!("{} URL(s) unreachable", report.url_broken.len());
            }
            Ok(())
        }

        Commands::Index { packages_dir, base_url, out } => {
            let entries = index::generate(&packages_dir, &base_url)
                .context("failed to scan package directory")?;

            let index_doc = index::RepoIndex { packages: entries };
            let toml_out =
                toml::to_string_pretty(&index_doc).context("failed to serialize index.toml")?;

            let out_path = out.unwrap_or_else(|| packages_dir.join("index.toml"));
            std::fs::write(&out_path, &toml_out)
                .with_context(|| format!("failed to write index {}", out_path.display()))?;

            println!(
                "{} Wrote {} package entries to {}",
                "✓".bold().bright_green(),
                index_doc.packages.len(),
                out_path.display().to_string().bright_cyan()
            );
            Ok(())
        }
    }
}
