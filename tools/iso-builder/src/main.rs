use anyhow::{Context, Result};
use colored::Colorize;
use isobemak::{build_iso, BootInfo, IsoImage, IsoImageFile, IsoLayoutProfile, UefiBootInfo};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
struct Config {
    workspace_root: PathBuf,
    profile: &'static str,
    output_iso: PathBuf,
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
                "--output" | "-o" => {
                    output = args.get(i + 1).cloned();
                    i += 1;
                }
                "--root" | "-r" => {
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
            .unwrap_or_else(|| workspace_root.join("dist").join("blackwall-server-v0.1.iso"));

        Config { workspace_root, profile, output_iso }
    }

    fn target_dir(&self) -> PathBuf {
        self.workspace_root.join("target").join(self.profile)
    }

    fn uefi_target_dir(&self) -> PathBuf {
        self.workspace_root.join("target").join("x86_64-unknown-uefi").join(self.profile)
    }
}

fn step(n: u8, total: u8, msg: &str) {
    println!("[{}/{}] {}", n, total, msg.bold());
}

fn ok(msg: &str) {
    println!("       {} {}", "+".bright_green(), msg);
}

fn warn(msg: &str) {
    println!("       {} {}", "!".bright_yellow(), msg);
}

fn collect_files(cfg: &Config) -> Vec<IsoImageFile> {
    let mut files = Vec::new();
    let target = cfg.target_dir();

    let placed: &[(&str, &str)] = &[
        ("bwinit", "blackwall/bin/bwinit"),
        ("anx", "blackwall/bin/anx"),
        ("anxd", "blackwall/bin/anxd"),
        ("bwsh", "blackwall/bin/bwsh"),
        ("bwlogin", "blackwall/bin/bwlogin"),
        ("bwfw", "blackwall/bin/bwfw"),
        ("bwcron", "blackwall/bin/bwcron"),
        ("bwssh", "blackwall/bin/bwssh"),
        ("bwsnap", "blackwall/bin/bwsnap"),
        ("bwbackup", "blackwall/bin/bwbackup"),
        ("bw-pkg", "blackwall/bin/bw-pkg"),
        ("anx-repo-server", "blackwall/bin/anx-repo-server"),
        ("blackwall-runner", "blackwall/bin/blackwall-runner"),
    ];

    for (bin, dest) in placed {
        let src = target.join(bin);
        if src.exists() {
            files.push(IsoImageFile { source: src, destination: dest.to_string() });
            ok(&format!("{} -> {}", bin, dest));
        }
    }

    let coreutils: &[&str] = &[
        "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "chmod", "chown", "echo", "find",
        "grep", "head", "tail", "wc", "sort", "uniq", "ps", "kill", "id", "whoami", "hostname",
        "date", "df", "du", "uname", "pwd",
    ];
    for cmd in coreutils {
        let src = target.join(cmd);
        if src.exists() {
            files.push(IsoImageFile { source: src, destination: format!("blackwall/bin/{}", cmd) });
        }
    }
    ok("coreutils (28 tools)");

    let init_src = cfg
        .workspace_root
        .join("userspace/init/target/x86_64-unknown-none")
        .join(cfg.profile)
        .join("init");
    if init_src.exists() {
        files
            .push(IsoImageFile { source: init_src, destination: "blackwall/bin/init".to_string() });
        ok("userspace init -> blackwall/bin/init");
    }

    files
}

fn build(cfg: &Config) -> Result<()> {
    let bootloader = cfg.uefi_target_dir().join("blackwall-bootloader.efi");
    let kernel = cfg.uefi_target_dir().join("blackwall-kernel.efi");

    if !bootloader.exists() {
        anyhow::bail!(
            "UEFI bootloader not found at {}\n  Build with: cargo build -p blackwall-bootloader --target x86_64-unknown-uefi",
            bootloader.display()
        );
    }
    if !kernel.exists() {
        anyhow::bail!(
            "UEFI kernel not found at {}\n  Build with: cargo build -p blackwall-kernel --target x86_64-unknown-uefi",
            kernel.display()
        );
    }

    ok(&format!("Bootloader: {}", bootloader.display()));
    ok(&format!("Kernel:     {}", kernel.display()));

    let files = collect_files(cfg);

    let grub_cfg = r#"set timeout=5
set default=0

menuentry "Black Wall Core v0.1" {
    echo "Loading Black Wall Core..."
    insmod all_video
    terminal_output console
    chainloader /EFI/BOOT/BOOTX64.EFI
    boot
}

menuentry "Black Wall Core v0.1 (Recovery)" {
    echo "Loading Recovery Console..."
    chainloader /EFI/BOOT/BOOTX64.EFI
    boot
}
"#;

    let mut iso_files = files;

    iso_files.push(IsoImageFile {
        source: bootloader.clone(),
        destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
    });
    iso_files.push(IsoImageFile {
        source: kernel.clone(),
        destination: "EFI/BLACKWALL/KERNEL.EFI".to_string(),
    });

    let iso_image = IsoImage {
        volume_id: Some("BLACKWALL".to_string()),
        files: iso_files,
        boot_info: BootInfo {
            bios_boot: None,
            uefi_boot: Some(UefiBootInfo {
                boot_image: bootloader,
                kernel_image: kernel,
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: Vec::new(),
                grub_cfg_content: Some(grub_cfg.to_string()),
            }),
        },
        layout_profile: IsoLayoutProfile::default(),
    };

    if let Some(parent) = cfg.output_iso.parent() {
        fs::create_dir_all(parent).context("Failed to create output directory")?;
    }
    if cfg.output_iso.exists() {
        fs::remove_file(&cfg.output_iso)?;
    }

    let (iso_path, _temp_fat, _iso_file, _fat_size) =
        build_iso(&cfg.output_iso, &iso_image, false).context("Failed to create ISO image")?;

    let meta = fs::metadata(&iso_path)?;
    ok(&format!(
        "ISO created: {} ({:.1} MB)",
        iso_path.display(),
        meta.len() as f64 / (1024.0 * 1024.0)
    ));

    let mut hasher = Sha256::new();
    hasher.update(fs::read(&iso_path)?);
    let hash = format!("{:x}", hasher.finalize());
    let checksum_path = iso_path.with_extension("sha256");
    fs::write(&checksum_path, format!("{}  {}\n", hash, iso_path.display()))?;
    ok(&format!("SHA256: {}", hash));

    Ok(())
}

fn main() -> Result<()> {
    println!();
    println!("  {} {}", "Black Wall Core".bold().bright_cyan(), "ISO Builder".bold());
    println!("  {}", "-".repeat(40).dimmed());
    println!();

    let cfg = Config::from_args();

    println!("  Profile : {}", cfg.profile.bright_green());
    println!("  Output  : {}", cfg.output_iso.display());
    println!();

    const STEPS: u8 = 3;

    step(1, STEPS, "Validating build artifacts...");
    let uefi = cfg.uefi_target_dir();
    let target = cfg.target_dir();
    let mut missing = false;
    for name in &["blackwall-bootloader.efi", "blackwall-kernel.efi"] {
        if uefi.join(name).exists() {
            ok(&format!("{} found", name));
        } else {
            warn(&format!("{} NOT FOUND", name));
            missing = true;
        }
    }
    if missing {
        println!();
        anyhow::bail!("Missing UEFI artifacts. Build first:");
    }

    step(2, STEPS, "Assembling ISO filesystem...");
    let file_count = fs::read_dir(&target)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);
    ok(&format!("{} binaries in target directory", file_count));

    step(3, STEPS, "Building ISO image...");
    build(&cfg)?;

    println!();
    println!("  {} ISO ready for installation!", "Done.".bold().bright_green());
    println!("  {}", cfg.output_iso.display().to_string().bright_cyan());
    println!();
    println!("  To test in QEMU:");
    println!("    qemu-system-x86_64 -bios /usr/share/OVMF/OVMF_CODE.fd \\");
    println!("      -cdrom {} -m 256M -serial stdio -display none", cfg.output_iso.display());
    println!();

    Ok(())
}
