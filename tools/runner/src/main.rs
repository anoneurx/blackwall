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

    // Trailing flags after the subcommand (currently only `--kvm`).
    let cmdline_flags: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let kvm = cmdline_flags.iter().any(|f| f == "--kvm");

    match command.to_string_lossy().as_ref() {
        "build" => build_all().map(|_| ()),
        "image" => build_all().and_then(|_| build_image()).map(|_| ()),
        // `run` opens a graphical QEMU window (the serial console becomes an
        // interactive tab inside it).  `run-headless` keeps the old behaviour
        // for CI / scripted tests.
        "run" => {
            build_all().and_then(|_| build_image()).and_then(|image| run_qemu(&image, true, kvm))
        }
        "run-headless" => {
            build_all().and_then(|_| build_image()).and_then(|image| run_qemu(&image, false, kvm))
        }
        other => Err(format!(
            "unknown command: {other} (expected: build | image | run | run-headless [--kvm])"
        )),
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
    // Reuse the maintained ISO builder instead of duplicating its El Torito /
    // FAT ESP logic here.  `bw-iso-builder` produces a bootable image (it
    // synthesizes the EFI boot image exactly the way OVMF expects), verified
    // by the QEMU boot tests.
    let iso_path = PathBuf::from("target/blackwall.iso");
    run_command(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "bw-iso-builder",
            "--",
            "--output",
            &iso_path.to_string_lossy(),
        ],
    )?;
    Ok(iso_path)
}

// QEMU's isa-debug-exit device exits with the written port value plus one;
// the kernel powerdown writes 0, so a clean shell exit surfaces as status 1.
const CLEAN_POWERDOWN_CODE: i32 = 1;

fn run_qemu(image: &Path, gui: bool, kvm: bool) -> Result<(), String> {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-m", "512M"]);

    // KVM is opt-in: the kernel's serial output is unreliable when the VM
    // runs under KVM, so `-enable-kvm` is only added when explicitly asked.
    if kvm {
        cmd.arg("-enable-kvm");
    }

    if gui {
        // The window shows the firmware/kernel framebuffer; the serial
        // console (the actual interactive shell) is reachable from the GTK
        // window menu: View -> serial0  (Ctrl+Alt+2).
        cmd.args(["-display", "gtk", "-serial", "vc"]);
        cmd.args(["-device", "qemu-xhci", "-device", "usb-tablet"]);
    } else {
        cmd.args(["-display", "none", "-serial", "stdio"]);
    }

    // Full-system powerdown support: the shell's `exit`/`poweroff` write to
    // isa-debug-exit, which terminates QEMU instead of leaving it hanging.
    cmd.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);

    if let Some((code, vars)) = find_ovmf_pair() {
        // NOTE: a pristine OVMF var store has no boot-order entries, so an
        // ISO-only VM's first boot stops at the "Please select boot device"
        // firmware menu.  Keep the copy persistent so the configured boot
        // order survives across runs (like a real VM's NVRAM).
        let vars_copy = PathBuf::from("target/blackwall-ovmf-vars.fd");
        if !vars_copy.exists() {
            fs::copy(&vars, &vars_copy).map_err(|error| error.to_string())?;
            if !gui {
                println!("note: first boot with fresh OVMF vars may stop at the firmware");
                println!("      boot menu; re-run this command once (state is now saved)");
            }
        }
        cmd.args(["-drive"])
            .arg(format!("if=pflash,format=raw,unit=0,readonly=on,file={}", code.display()))
            .args(["-drive"])
            .arg(format!("if=pflash,format=raw,unit=1,file={}", vars_copy.display()));
    } else if let Some(combined) = find_ovmf_combined() {
        cmd.args(["-bios"]).arg(combined);
    } else {
        return Err(format!(
            "unable to locate OVMF firmware; install it (Debian/Ubuntu: `apt install ovmf`, \
             Fedora: `dnf install edk2-ovmf`) or set OVMF_CODE + OVMF_VARS environment variables"
        ));
    }

    if image.is_dir() {
        cmd.args(["-drive", &format!("file=fat:rw:{},format=raw,media=disk", image.display())]);
    } else {
        cmd.args(["-cdrom"]).arg(image);
    }

    if gui {
        println!("Opening Black Wall QEMU window...");
        println!("  firmware menu (if shown): pick the DVD boot entry or just wait");
        println!("  shell console: QEMU window menu -> View -> serial0 (Ctrl+Alt+2)");
        println!("  type `help` for commands; `exit` powers the VM off");
    }

    let status = cmd.status().map_err(|error| error.to_string())?;
    if status.success() {
        return Ok(());
    }
    if let Some(code) = status.code() {
        if code == CLEAN_POWERDOWN_CODE {
            return Ok(()); // shell issued powerdown; not an error
        }
    }
    ensure_success(status, "qemu-system-x86_64")
}

fn find_ovmf_pair() -> Option<(PathBuf, PathBuf)> {
    let code = first_existing(&[
        env_var_path("OVMF_CODE"),
        PathBuf::from("/usr/share/OVMF/OVMF_CODE_4M.fd"),
        PathBuf::from("/usr/share/OVMF/OVMF_CODE.fd"),
        PathBuf::from("/usr/share/edk2/ovmf/OVMF_CODE.fd"),
        PathBuf::from("/usr/share/edk2/x64/OVMF_CODE.fd"),
    ])?;
    let vars = first_existing(&[
        env_var_path("OVMF_VARS"),
        PathBuf::from("/usr/share/OVMF/OVMF_VARS_4M.fd"),
        PathBuf::from("/usr/share/OVMF/OVMF_VARS.fd"),
        PathBuf::from("/usr/share/edk2/ovmf/OVMF_VARS.fd"),
        PathBuf::from("/usr/share/edk2/x64/OVMF_VARS.fd"),
    ])?;
    Some((code, vars))
}

fn find_ovmf_combined() -> Option<PathBuf> {
    first_existing(&[
        env_var_path("OVMF_BIOS"),
        PathBuf::from("/usr/share/ovmf/OVMF.fd"),
        PathBuf::from("/usr/share/OVMF/OVMF.fd"),
        PathBuf::from("/usr/share/edk2/ovmf/OVMF.fd"),
    ])
}

fn env_var_path(name: &str) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or_default()
}

fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|p| p.exists()).cloned()
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
