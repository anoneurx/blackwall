use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os();
    let _program = args.next();
    let command = args.next().unwrap_or_else(|| OsString::from("run"));

    match command.to_string_lossy().as_ref() {
        "build" => build_all(),
        "image" => build_all().and_then(|_| build_image()).map(|_| ()),
        "run" => build_all().and_then(|_| build_image()).and_then(|image| run_qemu(&image)),
        other => Err(format!("unknown command: {other}")),
    }
}

fn build_all() -> Result<(), String> {
    run_command(
        "cargo",
        &[
            "build",
            "--manifest-path",
            "userspace/init/Cargo.toml",
            "--target",
            "x86_64-unknown-none",
        ],
    )?;
    run_command("cargo", &["build", "--workspace"])?;
    run_command(
        "cargo",
        &["build", "-p", "blackwall-bootloader", "--target", "x86_64-unknown-uefi"],
    )?;
    run_command("cargo", &["build", "-p", "blackwall-kernel", "--target", "x86_64-unknown-uefi"])?;
    Ok(())
}

fn build_image() -> Result<PathBuf, String> {
    let image_root = PathBuf::from("target/blackwall-image");
    let efi_boot_dir = image_root.join("EFI/BOOT");
    let efi_blackwall_dir = image_root.join("EFI/BLACKWALL");
    fs::create_dir_all(&efi_boot_dir).map_err(|error| error.to_string())?;
    fs::create_dir_all(&efi_blackwall_dir).map_err(|error| error.to_string())?;

    let bootloader = Path::new("target/x86_64-unknown-uefi/debug/blackwall-bootloader.efi");
    let kernel = Path::new("target/x86_64-unknown-uefi/debug/blackwall-kernel.efi");

    fs::copy(bootloader, efi_boot_dir.join("BOOTX64.EFI")).map_err(|error| error.to_string())?;
    fs::copy(kernel, efi_blackwall_dir.join("KERNEL.EFI")).map_err(|error| error.to_string())?;

    let iso_path = PathBuf::from("target/blackwall.iso");
    if iso_path.exists() {
        let _ = fs::remove_file(&iso_path);
    }

    let xorriso_res = run_command(
        "xorriso",
        &[
            "-as",
            "mkisofs",
            "-R",
            "-J",
            "-V",
            "BLACKWALL",
            "-eltorito-alt-boot",
            "-e",
            "EFI/BOOT/BOOTX64.EFI",
            "-no-emul-boot",
            "-o",
            iso_path.to_string_lossy().as_ref(),
            image_root.to_string_lossy().as_ref(),
        ],
    );

    if xorriso_res.is_ok() {
        Ok(iso_path)
    } else {
        println!("Warning: xorriso not found, falling back to directory-based FAT boot.");
        Ok(image_root)
    }
}

fn run_qemu(image: &Path) -> Result<(), String> {
    let ovmf_code = find_ovmf_code()?;
    let ovmf_vars = find_ovmf_vars()?;
    let vars_copy = PathBuf::from("target/blackwall-ovmf-vars.fd");
    fs::copy(&ovmf_vars, &vars_copy).map_err(|error| error.to_string())?;

    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-m", "256M", "-serial", "stdio", "-display", "none", "-drive"])
        .arg(format!("if=pflash,format=raw,unit=0,readonly=on,file={}", ovmf_code.display()))
        .args(["-drive"])
        .arg(format!("if=pflash,format=raw,unit=1,file={}", vars_copy.display()));

    if image.is_dir() {
        cmd.args(["-drive", &format!("file=fat:rw:{},format=raw,media=disk", image.display())]);
    } else {
        cmd.args(["-cdrom"]).arg(image);
    }

    let status = cmd.status().map_err(|error| error.to_string())?;
    ensure_success(status, "qemu-system-x86_64")
}

fn find_ovmf_code() -> Result<PathBuf, String> {
    candidate_paths(
        "OVMF_CODE",
        &[
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/edk2/ovmf/OVMF_CODE.fd",
        ],
    )
}

fn find_ovmf_vars() -> Result<PathBuf, String> {
    candidate_paths(
        "OVMF_VARS",
        &[
            "/usr/share/OVMF/OVMF_VARS.fd",
            "/usr/share/OVMF/OVMF_VARS_4M.fd",
            "/usr/share/edk2/ovmf/OVMF_VARS.fd",
        ],
    )
}

fn candidate_paths(env_name: &str, fallbacks: &[&str]) -> Result<PathBuf, String> {
    if let Some(value) = env::var_os(env_name) {
        let path = PathBuf::from(value);
        if path.exists() {
            return Ok(path);
        }
    }

    for fallback in fallbacks {
        let path = PathBuf::from(fallback);
        if path.exists() {
            return Ok(path);
        }
    }

    Err(format!("unable to locate {env_name}; set the environment variable or install OVMF"))
}

fn run_command(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("failed to run {program}: {error}"))?;

    ensure_success(status, program)
}

fn ensure_success(status: ExitStatus, program: &str) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with status {status}"))
    }
}
