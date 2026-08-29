//! # anx-repo-server — Black Wall Secure Package Repository Server
//!
//! A high-performance web server for hosting `.anxpkg` packages and the repository
//! `index.toml`.
//!
//! Features:
//! - Serves static `.anxpkg` files securely.
//! - Validates the structure of the repository on startup.
//! - Can run behind a reverse proxy (like Nginx/Caddy) or standalone.

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use clap::Parser;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory containing the repository index and packages
    #[arg(short, long, default_value = "/var/www/blackwall-repo")]
    repo_dir: PathBuf,

    /// IP address and port to bind to
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    bind: String,
}

#[derive(Deserialize)]
struct RepoEntry {
    name: String,
    version: String,
    description: String,
    arch: String,
    url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct RepoIndex {
    #[serde(default)]
    packages: Vec<RepoEntry>,
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Regenerate `index.html` in the repo directory from `index.toml` so the
/// repository is browsable at `/`.
fn write_html_index(repo_dir: &std::path::Path) -> Result<()> {
    use std::fmt::Write as _;

    let mut entries: Vec<RepoEntry> = Vec::new();
    let index_path = repo_dir.join("index.toml");
    if let Ok(raw) = std::fs::read_to_string(&index_path) {
        if let Ok(idx) = toml::from_str::<RepoIndex>(&raw) {
            entries = idx.packages;
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut rows = String::new();
    for e in &entries {
        let _ = write!(
            rows,
            concat!(
                "<tr><td><a href=\"{url}\">{name}</a></td>",
                "<td>{version}</td><td>{arch}</td><td>{size} B</td>",
                "<td>{description}</td>",
                "<td><a href=\"{url}\" download>[download .anxpkg]</a></td></tr>\n"
            ),
            name = html_escape(&e.name),
            version = html_escape(&e.version),
            arch = html_escape(&e.arch),
            description = html_escape(&e.description),
            size = e.size,
            url = html_escape(&e.url),
        );
    }

    let page = format!(
        concat!(
            "<!doctype html><html><head><meta charset=\"utf-8\">",
            "<title>Black Wall Repository</title>",
            "<style>body{{font-family:sans-serif;margin:2rem}}table{{border-collapse:collapse;width:100%}}",
            "th,td{{text-align:left;padding:.4rem .6rem;border-bottom:1px solid #ddd}}",
            "th{{background:#f4f4f4}}</style></head>",
            "<body><h1>Black Wall Package Repository</h1>",
            "<p>{count} packages &middot; <a href=\"/index.toml\">index.toml</a></p>",
            "<table><thead><tr><th>Package</th><th>Version</th><th>Arch</th><th>Size</th><th>Description</th><th></th></tr></thead>",
            "<tbody>{rows}</tbody></table></body></html>"
        ),
        count = entries.len(),
        rows = rows
    );
    std::fs::write(repo_dir.join("index.html"), page).context("failed to write index.html")?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let args = Args::parse();
    let repo_dir = args.repo_dir;

    if !repo_dir.exists() {
        tracing::warn!(
            "Repository directory {} does not exist. Creating it...",
            repo_dir.display()
        );
        std::fs::create_dir_all(&repo_dir)
            .with_context(|| format!("Failed to create repo dir: {}", repo_dir.display()))?;
    }

    let index_path = repo_dir.join("index.toml");
    if !index_path.exists() {
        tracing::warn!(
            "index.toml not found at {}. Clients will fail to sync.",
            index_path.display()
        );
        // Create an empty index to prevent 404s for new repos
        let empty_index = "[[packages]]\n# Empty repository\n";
        std::fs::write(&index_path, empty_index).ok();
    }
    write_html_index(&repo_dir)?;

    tracing::info!("Starting Black Wall Repository Server...");
    tracing::info!("Serving packages from: {}", repo_dir.display());

    // Build the Axum router: static package files plus a generated browsable
    // `index.html`. A nested root serves everything else.
    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .nest_service("/", ServeDir::new(repo_dir))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = args.bind.parse().context("Invalid bind address")?;
    tracing::info!("Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
