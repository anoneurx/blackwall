//! # bw-iso-builder — Black Wall Core ISO Image Builder
//!
//! Orchestrates the assembly of a bootable Black Wall Core ISO image.
//!
//! This tool:
//! 1. Validates required binaries exist in the build output
//! 2. Assembles the ISO filesystem tree
//! 3. Writes GRUB2 configuration
//! 4. Calls `grub-mkrescue` (or `xorriso`) to produce the final ISO
//! 5. Generates SHA256 checksum file
//!
//! Usage:
//!   bw-iso-builder [--release] [--output <path>] [--root <workspace>]

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ─── Config ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Config {
    workspace_root: PathBuf,
    profile: &'static str,
    output_iso: PathBuf,
    iso_staging: PathBuf,
}

impl Config {
    fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut release = false;
        let mut output: Option<String> = None;
        let mut root: Option<String> = None;
        let mut i = 1;

        while i < args.len() {
            match args[i].as_str() {
                "--release" => release = true,
                "--output" => {
                    output = args.get(i + 1).cloned();
                    i += 1;
                }
                "--root" => {
                    root = args.get(i + 1).cloned();
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }

        let workspace_root = root
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let profile = if release { "release" } else { "debug" };

        let output_iso = output
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root.join("dist").join("blackwall-server-v2.0.iso"));

        let iso_staging = workspace_root.join("_isobuild");

        Config { workspace_root, profile, output_iso, iso_staging }
    }

    fn target_dir(&self) -> PathBuf {
        self.workspace_root.join("target").join(self.profile)
    }

    fn uefi_target_dir(&self) -> PathBuf {
        self.workspace_root.join("target").join("x86_64-unknown-uefi").join(self.profile)
    }
}

// ─── Steps ────────────────────────────────────────────────────────────────────

fn step(n: u8, total: u8, msg: &str) {
    println!("[{}/{}] {}", n, total, msg.bold());
}

fn ok(msg: &str) {
    println!("       {} {}", "✓".bright_green(), msg);
}

fn warn(msg: &str) {
    println!("       {} {}", "!".bright_yellow(), msg);
}

/// Check that external tools are available.
fn check_deps() -> Result<()> {
    for tool in ["grub-mkrescue", "xorriso", "sha256sum"] {
        let found =
            Command::new("which").arg(tool).output().map(|o| o.status.success()).unwrap_or(false);
        if !found {
            anyhow::bail!(
                "Required tool '{}' not found.\n  Install with: sudo apt install {}",
                tool,
                tool
            );
        }
    }
    Ok(())
}

/// Clean and recreate the ISO staging tree.
fn setup_staging(cfg: &Config) -> Result<()> {
    if cfg.iso_staging.exists() {
        fs::remove_dir_all(&cfg.iso_staging).context("Failed to clean staging dir")?;
    }

    for dir in ["boot/grub", "EFI/BOOT", "blackwall/bin", "blackwall/lib", "blackwall/etc"] {
        fs::create_dir_all(cfg.iso_staging.join(dir))
            .with_context(|| format!("Failed to create staging/{}", dir))?;
    }
    ok("Staging tree created");
    Ok(())
}

/// Copy compiled artifacts into the staging tree.
fn stage_artifacts(cfg: &Config) -> Result<()> {
    let bins = [
        // (source relative to target dir, dest in ISO tree)
        ("blackwall-bootloader.efi", "EFI/BOOT/BOOTX64.EFI"),
    ];

    for (src_name, dest_rel) in &bins {
        let src = cfg.uefi_target_dir().join(src_name);
        if src.exists() {
            let dest = cfg.iso_staging.join(dest_rel);
            fs::copy(&src, &dest).with_context(|| format!("Failed to copy {}", src_name))?;
            ok(&format!("Staged {} → {}", src_name, dest_rel));
        } else {
            warn(&format!("{} not found (may be a host build) — skipping", src_name));
        }
    }

    // Installer binary.
    let installer = cfg.target_dir().join("blackwall-installer");
    if installer.exists() {
        fs::copy(&installer, cfg.iso_staging.join("blackwall/bin/installer"))?;
        ok("Staged installer");
    }

    // Service binaries.
    let extra_bins = [
        "bw-api",
        "bwmonitor",
        "bwsnap",
        "bwbackup",
        "bwcluster",
        "bwctl",
        "bwfw",
        "anxd",
        "anx",
        "bwsh",
    ];
    for bin in &extra_bins {
        let src = cfg.target_dir().join(bin);
        if src.exists() {
            fs::copy(&src, cfg.iso_staging.join(format!("blackwall/bin/{}", bin)))?;
            ok(&format!("Staged {}", bin));
        }
    }

    // Systemd service files.
    let svc_dir = cfg.workspace_root.join("installer").join("systemd");
    if svc_dir.exists() {
        let dest_dir = cfg.iso_staging.join("blackwall/etc/systemd");
        fs::create_dir_all(&dest_dir)?;
        for entry in fs::read_dir(&svc_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            fs::copy(entry.path(), dest_dir.join(&file_name))?;
            ok(&format!("Staged systemd/{}", file_name.to_string_lossy()));
        }
    }

    // Systemd units from the workspace (boot/init provides bwinit).
    let bwinit = cfg.target_dir().join("bwinit");
    if bwinit.exists() {
        fs::copy(&bwinit, cfg.iso_staging.join("blackwall/bin/bwinit"))?;
        ok("Staged bwinit");
    }

    Ok(())
}

/// Write the GRUB2 boot configuration.
fn write_grub_config(cfg: &Config) -> Result<()> {
    let grub_cfg = r#"# Black Wall Core — GRUB2 Boot Configuration
set timeout=5
set default=0

menuentry "Black Wall Core v2.0" {
    echo "Loading Black Wall Core..."
    insmod all_video
    terminal_output console
    chainloader /EFI/BOOT/BOOTX64.EFI
    boot
}

menuentry "Black Wall Core v2.0 (Safe Mode)" {
    echo "Loading in safe mode..."
    chainloader /EFI/BOOT/BOOTX64.EFI
    set bwcmdline="single"
    boot
}

menuentry "Black Wall Core v2.0 (Recovery Console)" {
    echo "Loading Recovery Console..."
    chainloader /EFI/BOOT/BOOTX64.EFI
    set bwcmdline="recovery"
    boot
}
"#;
    let path = cfg.iso_staging.join("boot/grub/grub.cfg");
    fs::write(&path, grub_cfg).context("Failed to write grub.cfg")?;
    ok("Written grub.cfg");
    Ok(())
}

/// Call grub-mkrescue to produce the ISO.
fn build_iso(cfg: &Config) -> Result<()> {
    // Ensure output directory exists.
    if let Some(parent) = cfg.output_iso.parent() {
        fs::create_dir_all(parent).context("Failed to create output directory")?;
    }

    let status = Command::new("grub-mkrescue")
        .arg(format!("--output={}", cfg.output_iso.display()))
        .arg(cfg.iso_staging.display().to_string())
        .arg("--")
        .args(["-volid", "BLACKWALL_2_0"])
        .status()
        .context("Failed to execute grub-mkrescue")?;

    if !status.success() {
        anyhow::bail!("grub-mkrescue failed with status {:?}", status.code());
    }

    if !cfg.output_iso.exists() {
        anyhow::bail!(
            "grub-mkrescue succeeded but ISO file not found at {}",
            cfg.output_iso.display()
        );
    }

    let meta = fs::metadata(&cfg.output_iso)?;
    ok(&format!("ISO created ({:.1} MB)", meta.len() as f64 / (1024.0 * 1024.0)));
    Ok(())
}

/// Compute SHA256 and write a sidecar file.
fn write_checksum(cfg: &Config) -> Result<()> {
    let output = Command::new("sha256sum")
        .arg(cfg.output_iso.display().to_string())
        .output()
        .context("Failed to run sha256sum")?;

    let hash_line = String::from_utf8_lossy(&output.stdout).to_string();
    let checksum_path = cfg.output_iso.with_extension("sha256");
    fs::write(&checksum_path, &hash_line).context("Failed to write checksum file")?;

    let hash = hash_line.split_whitespace().next().unwrap_or("?");
    ok(&format!("SHA256: {}", hash.bright_cyan()));
    Ok(())
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    println!();
    println!("  {} {}", "Black Wall Core".bold().bright_cyan(), "ISO Builder".bold());
    println!("  {}", "────────────────────────────────────────".dimmed());
    println!();

    let cfg = Config::from_args();

    println!("  Profile : {}", cfg.profile.bright_green());
    println!("  Output  : {}", cfg.output_iso.display());
    println!();

    const STEPS: u8 = 5;

    step(1, STEPS, "Checking dependencies...");
    check_deps()?;

    step(2, STEPS, "Setting up staging tree...");
    setup_staging(&cfg)?;

    step(3, STEPS, "Staging artifacts...");
    stage_artifacts(&cfg)?;
    write_grub_config(&cfg)?;

    step(4, STEPS, "Building ISO image...");
    build_iso(&cfg)?;

    step(5, STEPS, "Writing checksum...");
    write_checksum(&cfg)?;

    println!();
    println!("  {} Build complete!", "✓".bold().bright_green());
    println!("  {}", cfg.output_iso.display().to_string().bright_cyan());
    println!();
    println!("  Test with: scripts/run-qemu.sh");
    println!();

    Ok(())
}
