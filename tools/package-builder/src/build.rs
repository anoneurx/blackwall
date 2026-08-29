//! Package assembly — turns a recipe into a compressed, signed `.anxpkg`.
//!
//! `.anxpkg` layout (zstd-compressed tar):
//! ```text
//! MANIFEST.toml   — package metadata (see package-manager/anx/src/pkg.rs)
//! files/          — payload files, installed relative to filesystem root
//! ```
//!
//! A detached GPG signature (`<name>-<version>.sig.gpg`) is written next to the
//! archive when `--sign` is passed; `anx` expects that signature alongside the
//! package for verification.

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tar::{Archive, Builder, Header};
use walkdir::WalkDir;

use crate::recipe::Recipe;

/// Mirror of the manifest shape used by `anx` (package-manager/anx/src/pkg.rs).
#[derive(Debug, serde::Serialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub arch: String,
    pub license: String,
    pub homepage: Option<String>,
    pub maintainer: String,
    #[serde(default)]
    pub depends: HashMap<String, String>,
    pub files: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_install: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_remove: Option<String>,
}

/// Build a single package from a recipe. Returns the produced `.anxpkg` path.
pub fn build(recipe_path: &Path, out_dir: &Path, sign: bool) -> Result<PathBuf> {
    let recipe = Recipe::load(recipe_path).with_context(|| "Failed to load recipe".to_string())?;

    let recipe_dir = recipe_path.parent().context("Recipe path has no parent directory")?;

    let payload_dir = recipe_dir.join(&recipe.source.payload_dir);

    // Obtain the payload (download + extract for tarball sources).
    prepare_payload(&recipe, recipe_dir, &payload_dir)?;

    // Collect payload files and their hashes.
    let mut files = collect_payload(&payload_dir)?;

    // Include the generated bwinit service unit, if declared.
    if let Some(unit) = &recipe.install.unit {
        files.insert(
            format!("etc/bwinit/services/{}.service", recipe.package.name),
            sha256_hex(unit.as_bytes()),
        );
    }

    let manifest = Manifest {
        name: recipe.package.name.clone(),
        version: recipe.package.version.clone(),
        description: recipe.package.description.clone(),
        arch: recipe.package.architecture.clone(),
        license: recipe.package.license.clone(),
        homepage: recipe.package.homepage.clone(),
        maintainer: recipe
            .package
            .maintainer
            .clone()
            .unwrap_or_else(|| "Black Wall Core <dev@blackwall.local>".to_string()),
        depends: recipe.package.depends.clone(),
        files,
        post_install: recipe.install.post.clone(),
        pre_remove: recipe.install.pre_remove.clone(),
    };

    let manifest_toml =
        toml::to_string_pretty(&manifest).context("Failed to serialize manifest")?;

    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("Failed to create output dir {}", out_dir.display()))?;

    let pkg_name = format!("{}-{}.anxpkg", recipe.package.name, recipe.package.version);
    let pkg_path = out_dir.join(&pkg_name);

    write_archive(&pkg_path, &manifest_toml, &payload_dir, &recipe)?;

    if sign {
        let sig_path = out_dir.join(format!("{}.sig.gpg", pkg_name));
        gpg_sign(&pkg_path, &sig_path)?;
    }

    Ok(pkg_path)
}

/// Download (if needed) and extract the payload directory.
fn prepare_payload(recipe: &Recipe, recipe_dir: &Path, payload_dir: &Path) -> Result<()> {
    match recipe.source.source_type.as_str() {
        "dir" => {
            if !payload_dir.exists() {
                anyhow::bail!(
                    "Payload directory '{}' does not exist. Stage files there first \
                     (or set source.type = 'tarball' with a source.url).",
                    payload_dir.display()
                );
            }
            Ok(())
        }
        "prebuilt" => {
            if payload_has_files(payload_dir) {
                return Ok(()); // already staged by hand or a previous fetch
            }
            match recipe.source.url.as_deref() {
                Some(url) => fetch_single_file(recipe, recipe_dir, payload_dir, url),
                None => anyhow::bail!(
                    "Payload directory '{}' is empty and no source.url is set. \
                     Stage files there first, or add [source] url / run `bw-pkg fetch`.",
                    payload_dir.display()
                ),
            }
        }
        "tarball" => {
            if payload_dir.exists() {
                return Ok(()); // reuse an earlier extraction
            }
            let url = recipe.source.url.as_deref().context("tarball source needs source.url")?;
            // download() also serves cache hits — verify against the pin
            // regardless of provenance so corrupt caches never ship.
            let archive_path = download(url, recipe_dir)?;
            verify_sha256(&archive_path, recipe.source.sha256.as_deref(), url)?;
            if looks_like_zip(&archive_path) {
                extract_zip(&archive_path, payload_dir)?;
                if recipe.source.flatten {
                    flatten_payload(payload_dir)?;
                }
            } else if looks_like_tar(&archive_path) {
                extract(&archive_path, payload_dir)?;
                if recipe.source.flatten {
                    flatten_payload(payload_dir)?;
                }
            } else {
                // Compressed single file (e.g. restic_*.bz2): stage as one
                // executable at usr/bin/<name>.
                stage_single_binary(&archive_path, payload_dir, &recipe.package.name)?;
            }
            Ok(())
        }
        other => anyhow::bail!("unsupported source type '{}'", other),
    }
}

/// Fetch and stage the payload for one recipe without building the archive.
///
/// Returns `true` when something was downloaded, `false` when the payload was
/// already present.
pub fn fetch(recipe_path: &Path) -> Result<bool> {
    let recipe = Recipe::load(recipe_path).with_context(|| "Failed to load recipe".to_string())?;
    let recipe_dir = recipe_path.parent().context("Recipe path has no parent directory")?;
    let payload_dir = recipe_dir.join(&recipe.source.payload_dir);

    match recipe.source.source_type.as_str() {
        "prebuilt" | "tarball" => {
            if recipe.source.source_type == "prebuilt" && payload_has_files(&payload_dir) {
                return Ok(false);
            }
            if recipe.source.source_type == "tarball" && payload_dir.exists() {
                return Ok(false);
            }
            prepare_payload(&recipe, recipe_dir, &payload_dir)?;
            Ok(true)
        }
        "dir" => anyhow::bail!(
            "source type 'dir' cannot be fetched — stage files in '{}' manually",
            payload_dir.display()
        ),
        other => anyhow::bail!("unsupported source type '{}'", other),
    }
}

/// Outcome of walking a package tree with [`fetch_tree`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TreeFetchStats {
    pub fetched: usize,
    pub cached: usize,
    pub no_url: usize,
    pub failed: usize,
}

/// Outcome of walking a package tree with [`build_tree`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TreeBuildStats {
    pub built: usize,
    /// Recipes with nothing to stage and no way to stage it.
    pub skipped_no_payload: usize,
    pub failed: usize,
    /// Produced `.anxpkg` paths (relative-ish, as given to the builder).
    pub artifacts: Vec<PathBuf>,
}

/// Walk `root` for every `recipe.toml`, fetching payloads where possible.
pub fn fetch_tree(root: &Path) -> Result<TreeFetchStats> {
    let mut stats = TreeFetchStats::default();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name() == "recipe.toml" {
            match fetch(entry.path()) {
                Ok(true) => stats.fetched += 1,
                Ok(false) => stats.cached += 1,
                Err(_) => {
                    // Distinguish "no URL to fetch" from real failures.
                    let no_url = Recipe::load(entry.path()).is_ok_and(|r| {
                        r.source.source_type == "prebuilt" && r.source.url.is_none()
                    });
                    if no_url {
                        stats.no_url += 1;
                    } else {
                        stats.failed += 1;
                        eprintln!("fetch failed: {}", entry.path().display());
                    }
                }
            }
        }
    }
    Ok(stats)
}

/// Walk `root` and build every buildable recipe into `out_dir`.
///
/// A recipe is *skipped* when it has neither a staged payload nor a URL to
/// fetch one from; anything else that errors counts as *failed*. Deterministic
/// order (sorted paths) keeps CI logs comparable across runs.
pub fn build_tree(root: &Path, out_dir: &Path, sign: bool) -> Result<TreeBuildStats> {
    let mut recipes: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.file_name() == "recipe.toml"
                && !e.path().components().any(|c| c.as_os_str() == "out")
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    recipes.sort();

    let mut stats = TreeBuildStats::default();
    for recipe_path in recipes {
        let skippable = Recipe::load(&recipe_path).is_ok_and(|r| {
            r.source.source_type == "prebuilt"
                && r.source.url.is_none()
                && !payload_has_files(
                    &recipe_path.parent().unwrap_or(Path::new(".")).join(&r.source.payload_dir),
                )
        });

        if skippable {
            stats.skipped_no_payload += 1;
            continue;
        }

        match build(&recipe_path, out_dir, sign) {
            Ok(pkg) => {
                println!("built {} → {}", recipe_path.display(), pkg.display());
                stats.built += 1;
                stats.artifacts.push(pkg);
            }
            Err(e) => {
                stats.failed += 1;
                eprintln!("build failed {}: {e:#}", recipe_path.display());
            }
        }
    }
    Ok(stats)
}

/// Health report for every recipe under `root`.
#[derive(Debug, Default)]
pub struct TreeVerifyReport {
    /// Recipes examined.
    pub total: usize,
    pub with_url: usize,
    pub without_url: usize,
    /// URL-bearing recipes whose upstream answered HEAD successfully.
    pub url_ok: usize,
    pub url_broken: Vec<(PathBuf, String)>,
    /// URL-bearing recipes missing a pinned SHA-256 (distribution hygiene).
    pub sha_missing: Vec<PathBuf>,
}

impl TreeVerifyReport {
    pub fn summary(&self) -> String {
        format!(
            "recipes: {} | with-url: {} (ok {}, broken {}) | no-url: {} | unpinned: {}",
            self.total,
            self.with_url,
            self.url_ok,
            self.url_broken.len(),
            self.without_url,
            self.sha_missing.len(),
        )
    }
}

/// Verify every recipe's source URL answers HEAD, and flag missing pins.
///
/// Network-bound but cheap: one HEAD per distinct URL, short timeout.
pub fn verify_tree(root: &Path) -> Result<TreeVerifyReport> {
    use std::time::Duration;

    let mut report = TreeVerifyReport::default();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("bw-pkg-verify/0.1")
        .build()?;

    let mut recipes: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.file_name() == "recipe.toml")
        .map(|e| e.path().to_path_buf())
        .collect();
    recipes.sort();

    for path in recipes {
        report.total += 1;
        let Ok(recipe) = Recipe::load(&path) else {
            report.url_broken.push((path.clone(), "invalid recipe".into()));
            continue;
        };
        match recipe.source.url.as_deref() {
            None => report.without_url += 1,
            Some(url) => {
                report.with_url += 1;
                if recipe.source.sha256.is_none() {
                    report.sha_missing.push(path.clone());
                }
                match client.head(url).send() {
                    Ok(resp) if resp.status().is_success() => report.url_ok += 1,
                    Ok(resp) => report
                        .url_broken
                        .push((path.clone(), format!("HTTP {}", resp.status()))),
                    Err(e) => report.url_broken.push((path.clone(), e.to_string())),
                }
            }
        }
    }
    Ok(report)
}

/// True when the payload directory exists and holds at least one file.
fn payload_has_files(payload_dir: &Path) -> bool {
    payload_dir.exists()
        && WalkDir::new(payload_dir)
            .follow_links(false)
            .into_iter()
            .any(|e| e.map(|e| e.file_type().is_file()).unwrap_or(false))
}

/// Download a single file into `payload_dir/<dest>` (default `usr/bin/<name>`).
fn fetch_single_file(
    recipe: &Recipe,
    recipe_dir: &Path,
    payload_dir: &Path,
    url: &str,
) -> Result<()> {
    let dest =
        recipe.source.dest.clone().unwrap_or_else(|| format!("usr/bin/{}", recipe.package.name));
    let target = payload_dir.join(&dest);
    std::fs::create_dir_all(target.parent().unwrap_or(payload_dir))?;

    println!("  fetching {} → {}", url, dest);
    let archive_path = download(url, recipe_dir)?;
    verify_sha256(&archive_path, recipe.source.sha256.as_deref(), url)?;

    std::fs::copy(&archive_path, &target).with_context(|| {
        format!("Failed to move {} to {}", archive_path.display(), target.display())
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms)?;
    }
    Ok(())
}

/// Verify a downloaded file against an optional expected SHA-256.
fn verify_sha256(path: &Path, expected: Option<&str>, label: &str) -> Result<()> {
    if let Some(expected) = expected {
        let actual = sha256_hex(&std::fs::read(path)?);
        if actual != *expected {
            anyhow::bail!(
                "checksum mismatch for {}:\n  expected: {}\n  got:      {}",
                label,
                expected,
                actual
            );
        }
    }
    Ok(())
}

/// Strip a single enclosing top-level directory from an extracted payload.
///
/// Many upstream archives (nodejs, go, Java) unpack into a versioned directory
/// like `node-v22.16.0-linux-x64/`. When `flatten` is enabled we hoist its
/// contents into the payload root so files install directly under `/`.
fn flatten_payload(payload_dir: &Path) -> Result<()> {
    let entries: Vec<_> = std::fs::read_dir(payload_dir)?.filter_map(|e| e.ok()).collect();
    if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
        return Ok(()); // nothing to flatten
    }

    let top = entries[0].path();
    let tmp = payload_dir.join(".flatten-tmp");
    std::fs::rename(&top, &tmp)?;

    let inner: Vec<_> = std::fs::read_dir(&tmp)?.filter_map(|e| e.ok()).collect();
    for entry in inner {
        let dest = payload_dir.join(entry.file_name());
        std::fs::rename(entry.path(), &dest)?;
    }
    std::fs::remove_dir(&tmp)?;
    Ok(())
}

/// Download a remote URL into `dir`, reusing the file if already present.
///
/// Large archives are fetched to `<name>.part` with byte-range resume and a
/// few retries, then atomically renamed — an interrupted download never
/// masquerades as a complete cache hit.
fn download(url: &str, dir: &Path) -> Result<PathBuf> {
    use std::time::Duration;

    let file_name = url.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("source.archive");
    let dest = dir.join(file_name);
    if dest.exists() {
        return Ok(dest);
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(3600))
        .user_agent("bw-pkg/0.1")
        .build()?;
    let partial = dir.join(format!(".{file_name}.part"));

    const MAX_ATTEMPTS: usize = 5;
    let mut last_err = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match download_once(&client, url, &partial) {
            Ok(()) => {
                std::fs::rename(&partial, &dest).with_context(|| {
                    format!("Failed to finalize {}", dest.display())
                })?;
                return Ok(dest);
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < MAX_ATTEMPTS {
                    eprintln!("  retry {attempt}/{MAX_ATTEMPTS} for {url}");
                }
            }
        }
    }

    let _ = std::fs::remove_file(&partial);
    Err(last_err.unwrap()).with_context(|| format!("Failed to download {url}"))
}

/// One full/partial-range fetch of `url` appended onto `partial`.
fn download_once(
    client: &reqwest::blocking::Client,
    url: &str,
    partial: &Path,
) -> Result<()> {
    let start = partial.metadata().map(|m| m.len()).unwrap_or(0);

    let mut request = client.get(url);
    if start > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={start}-"));
    }

    let mut response = request.send().context("request failed")?;
    match response.status() {
        reqwest::StatusCode::OK => {
            // Server ignored the range (or none sent): start over.
            File::create(partial).context("Failed to create partial file")?;
        }
        reqwest::StatusCode::PARTIAL_CONTENT => {}
        status => anyhow::bail!("HTTP {status}"),
    }

    let mut file = std::fs::OpenOptions::new().append(true).create(true).open(partial)?;
    response.copy_to(&mut file).context("transfer interrupted")?;
    Ok(())
}

/// Open an archive as a read stream, transparently decompressing by extension
/// (`.gz`, `.bz2`, `.zst`; anything else is treated as raw).
fn open_maybe_compressed(path: &Path) -> Result<Box<dyn std::io::Read>> {
    let file = File::open(path)?;
    Ok(match path.extension().and_then(|e| e.to_str()) {
        Some("gz") => Box::new(GzDecoder::new(file)),
        Some("bz2") => Box::new(bzip2::read::MultiBzDecoder::new(file)),
        Some("xz") => Box::new(xz2::read::XzDecoder::new(file)),
        Some("zst") => Box::new(zstd::Decoder::new(file)?),
        _ => Box::new(file),
    })
}

/// True when the (possibly compressed) file at `path` is a tar archive.
fn looks_like_tar(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 512];
    let n = match open_maybe_compressed(path) {
        Ok(mut r) => r.read(&mut buf).unwrap_or(0),
        Err(_) => 0,
    };
    n == 512 && (&buf[257..262] == b"ustar")
}

/// True when the file at `path` is a ZIP archive (checked via magic bytes).
fn looks_like_zip(path: &Path) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 4];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    file.read_exact(&mut buf).is_ok() && &buf == b"PK\x03\x04"
}

/// Extract a ZIP archive into `dest`, placing files relative to `dest`.
fn extract_zip(path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("Failed to read ZIP archive {}", path.display()))?;
    std::fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let out_path = dest.join(entry.mangled_name());
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

/// Decompress a single-file archive into an executable at
/// `<payload_dir>/usr/bin/<name>` (used when upstream ships a bare binary).
fn stage_single_binary(archive: &Path, payload_dir: &Path, name: &str) -> Result<()> {
    let mut reader = open_maybe_compressed(archive)?;
    let target = payload_dir.join("usr/bin").join(name);
    std::fs::create_dir_all(target.parent().unwrap_or(payload_dir))?;
    let mut out = File::create(&target)
        .with_context(|| format!("Failed to create {}", target.display()))?;
    std::io::copy(&mut reader, &mut out).context("Decompress write error")?;
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&target)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms)?;
    }
    Ok(())
}

/// Extract a `.tar`, `.tar.gz`, `.tar.bz2` or `.tar.zst` archive into `dest`.
fn extract(path: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut archive = Archive::new(open_maybe_compressed(path)?);
    archive.set_preserve_permissions(true);
    archive.unpack(dest).with_context(|| format!("Failed to extract {}", path.display()))?;
    Ok(())
}

/// Produce destination path → SHA-256 mappings for every file in the payload.
fn collect_payload(payload_dir: &Path) -> Result<HashMap<String, String>> {
    let mut files = HashMap::new();
    for entry in WalkDir::new(payload_dir).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel =
            entry.path().strip_prefix(payload_dir).context("path escaping payload directory")?;
        let rel_str = rel.to_string_lossy().to_string();
        let data = std::fs::read(entry.path())
            .with_context(|| format!("Failed to read {}", entry.path().display()))?;
        files.insert(rel_str, sha256_hex(&data));
    }
    Ok(files)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Serialize the archive: `MANIFEST.toml` + payload + optional service unit.
fn write_archive(
    pkg_path: &Path,
    manifest_toml: &str,
    payload_dir: &Path,
    recipe: &Recipe,
) -> Result<()> {
    let file = File::create(pkg_path)?;
    let zstd = zstd::Encoder::new(BufWriter::new(file), 3)?;
    let mut builder = Builder::new(zstd);

    // MANIFEST.toml first.
    let mut header = Header::new_gnu();
    header.set_size(manifest_toml.len() as u64);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    builder
        .append_data(&mut header, "MANIFEST.toml", manifest_toml.as_bytes())
        .context("Failed to append MANIFEST.toml")?;

    // Payload files under `files/`.
    for entry in WalkDir::new(payload_dir).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel =
            entry.path().strip_prefix(payload_dir).context("path escaping payload directory")?;
        let archive_name = format!("files/{}", rel.to_string_lossy());

        let mut file = File::open(entry.path())?;
        let metadata = file.metadata()?;

        let mut header = Header::new_gnu();
        header.set_size(metadata.len());
        header.set_mode(metadata.permissions().mode() & 0o7777);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(&mut header, &archive_name, &mut file)
            .with_context(|| format!("Failed to append {}", archive_name))?;
    }

    // Synthetic service unit.
    if let Some(unit) = &recipe.install.unit {
        let unit_name = format!("files/etc/bwinit/services/{}.service", recipe.package.name);
        let mut header = Header::new_gnu();
        header.set_size(unit.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(&mut header, &unit_name, unit.as_bytes())
            .context("Failed to append service unit")?;
    }

    builder.finish()?;
    let encoder = builder.into_inner().context("finalize tar")?;
    encoder.finish()?;
    Ok(())
}

/// Produce a detached armored GPG signature over the archive.
fn gpg_sign(data: &Path, sig: &Path) -> Result<()> {
    let status = std::process::Command::new("gpg")
        .args(["--no-default-keyring", "--detach-sign", "--armor"])
        .arg("--output")
        .arg(sig)
        .arg(data)
        .status()
        .context("gpg binary not found; cannot sign package")?;
    if !status.success() {
        anyhow::bail!("gpg signing failed for {}", data.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Serve one file over loopback HTTP for fetch tests.
    fn serve_once(body: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        format!("http://{}/tool", addr)
    }

    fn write_recipe(dir: &Path, url: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("recipe.toml");
        std::fs::write(
            &path,
            format!(
                "[package]\nname = \"demo\"\nversion = \"1.0.0\"\ndescription = \"d\"\n\
                 architecture = \"x86_64\"\nlicense = \"MIT\"\n\n[source]\ntype = \"prebuilt\"\n\
                 url = \"{}\"\npayload_dir = \"payload\"\n",
                url
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn build_tree_builds_staged_and_skips_unstageable() {
        let tmp = std::env::temp_dir().join(format!("bwpkg-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let out = tmp.join("out");

        // Buildable: dir-type recipe with a staged payload.
        let good = tmp.join("tree/good/pkg");
        std::fs::create_dir_all(good.join("payload/usr/bin")).unwrap();
        std::fs::write(good.join("payload/usr/bin/hello"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::write(
            good.join("recipe.toml"),
            "[package]\nname = \"hello\"\nversion = \"1.0\"\ndescription = \"d\"\n\
             architecture = \"x86_64\"\nlicense = \"MIT\"\n\n[source]\ntype = \"dir\"\n",
        )
        .unwrap();

        // Skippable: prebuilt, no URL, empty payload.
        let skip = tmp.join("tree/skip/pkg");
        std::fs::create_dir_all(&skip).unwrap();
        std::fs::write(
            skip.join("recipe.toml"),
            "[package]\nname = \"later\"\nversion = \"2.0\"\ndescription = \"d\"\n\
             architecture = \"x86_64\"\nlicense = \"MIT\"\n\n[source]\ntype = \"prebuilt\"\n",
        )
        .unwrap();

        let stats = build_tree(&tmp.join("tree"), &out, false).unwrap();
        assert_eq!(stats.built, 1);
        assert_eq!(stats.skipped_no_payload, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.artifacts.len(), 1);
        assert!(stats.artifacts[0].file_name().unwrap() == "hello-1.0.anxpkg");
        assert!(out.join("hello-1.0.anxpkg").is_file());

        // The produced archive must carry a readable MANIFEST.
        let manifest = crate::index::read_manifest_public(&out.join("hello-1.0.anxpkg")).unwrap();
        assert_eq!(manifest.name, "hello");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn build_tree_counts_failures_without_aborting() {
        let tmp = std::env::temp_dir().join(format!("bwpkg-treefail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        // Broken: tarball recipe pointing at an unreachable URL.
        let bad = tmp.join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("recipe.toml"),
            "[package]\nname = \"broken\"\nversion = \"9.9\"\ndescription = \"d\"\n\
             architecture = \"x86_64\"\nlicense = \"MIT\"\n\n[source]\ntype = \"tarball\"\n\
             url = \"http://127.0.0.1:1/x.tar.gz\"\n",
        )
        .unwrap();

        let stats = build_tree(&tmp, &tmp.join("out"), false).unwrap();
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.built, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prebuilt_fetch_downloads_into_dest() {
        let tmp = std::env::temp_dir().join(format!("bwpkg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let recipe_dir = tmp.join("demo");
        let body: &'static [u8] = b"#!/bin/sh\necho hi\n";
        let url = serve_once(body);
        let path = write_recipe(&recipe_dir, &url);

        assert!(fetch(&path).unwrap());
        let staged = recipe_dir.join("payload/usr/bin/demo");
        assert_eq!(std::fs::read(&staged).unwrap(), body);

        // Second fetch is a no-op (cached).
        assert!(!fetch(&path).unwrap());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn payload_has_files_detects_empty_and_full() {
        let tmp = std::env::temp_dir().join(format!("bwpkg-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let payload = tmp.join("payload");
        assert!(!payload_has_files(&payload)); // missing
        std::fs::create_dir_all(&payload).unwrap();
        assert!(!payload_has_files(&payload)); // empty
        std::fs::create_dir_all(payload.join("usr/bin")).unwrap();
        std::fs::write(payload.join("usr/bin/x"), b"x").unwrap();
        assert!(payload_has_files(&payload));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn verify_sha256_rejects_mismatch() {
        let tmp = std::env::temp_dir().join(format!("bwpkg-sha-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("f");
        std::fs::write(&f, b"data").unwrap();
        let good = sha256_hex(b"data");
        assert!(verify_sha256(&f, Some(&good), "test").is_ok());
        assert!(verify_sha256(&f, None, "test").is_ok());
        assert!(verify_sha256(&f, Some("deadbeef"), "test").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
