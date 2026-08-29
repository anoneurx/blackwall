//! # Black Wall Init Daemon (PID 1)
//!
//! `bwinit` is the first userspace process launched by the Black Wall kernel.
//! It is responsible for:
//!
//! - Mounting essential filesystems (`/proc`, `/sys`, `/dev`)
//! - Starting system services in the correct order
//! - Reaping orphaned child processes
//! - Handling reboot / shutdown / halt signals
//!
//! ## Service Start Order
//! 1. filesystem mounts
//! 2. hostname
//! 3. network (DHCP)
//! 4. firewall (`bwfw`)
//! 5. SSH (`sshd`)
//! 6. cron (`bwcron`)
//! 7. login prompt

use std::io;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

// ─── Service descriptor ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ServiceDef {
    pub name: &'static str,
    pub bin: &'static str,
    pub args: &'static [&'static str],
    pub restart: bool,
}

/// All services that init should start for a v1.0 Black Wall system.
/// Order matters — later services depend on earlier ones.
const SERVICES: &[ServiceDef] = &[
    ServiceDef { name: "network", bin: "/usr/sbin/dhclient", args: &["-nw", "-1"], restart: true },
    ServiceDef {
        name: "firewall",
        bin: "/usr/sbin/bwfw",
        args: &["--apply", "/etc/bwfw/rules.toml"],
        restart: false,
    },
    ServiceDef { name: "sshd", bin: "/usr/sbin/sshd", args: &["-D"], restart: true },
    ServiceDef { name: "cron", bin: "/usr/sbin/bwcron", args: &[], restart: true },
    ServiceDef { name: "anxd", bin: "/usr/sbin/anxd", args: &[], restart: true },
];

// ─── Running service table ─────────────────────────────────────────────────

struct RunningService {
    def: ServiceDef,
    child: Child,
}

// ─── Filesystem mounts ────────────────────────────────────────────────────────

fn mount_pseudo_filesystems() {
    let mounts = [
        ("proc", "/proc", "proc"),
        ("sysfs", "/sys", "sysfs"),
        ("devtmpfs", "/dev", "devtmpfs"),
        ("tmpfs", "/run", "tmpfs"),
        ("devpts", "/dev/pts", "devpts"),
    ];

    for (fstype, target, src) in &mounts {
        // Ensure mount point exists.
        let _ = std::fs::create_dir_all(target);

        let status = Command::new("/bin/mount").args(["-t", fstype, src, target]).status();

        match status {
            Ok(s) if s.success() => {
                log_info(&format!("Mounted {} on {}", src, target));
            }
            Ok(s) => {
                log_warn(&format!(
                    "mount {} failed with status {}",
                    target,
                    s.code().unwrap_or(-1)
                ));
            }
            Err(e) => {
                log_warn(&format!("mount {} error: {}", target, e));
            }
        }
    }
}

// ─── Hostname ─────────────────────────────────────────────────────────────────

fn set_hostname() {
    let hostname =
        std::fs::read_to_string("/etc/hostname").unwrap_or_else(|_| "blackwall".to_string());
    let hostname = hostname.trim().to_string();

    let status = Command::new("/bin/hostname").arg(&hostname).status();
    match status {
        Ok(s) if s.success() => log_info(&format!("Hostname set to {}", hostname)),
        _ => log_warn("Failed to set hostname"),
    }
}

// ─── Service management ───────────────────────────────────────────────────────

fn start_service(def: &ServiceDef) -> io::Result<Child> {
    log_info(&format!("Starting service: {}", def.name));
    Command::new(def.bin).args(def.args.iter().copied()).spawn()
}

// ─── Zombie reaper ────────────────────────────────────────────────────────────
//
// As PID 1 we inherit all orphaned processes. We must periodically waitpid()
// to avoid zombie accumulation.

fn reap_zombies() {
    loop {
        let pid = libc_waitpid(-1, std::ptr::null_mut(), LIBC_WNOHANG);
        if pid <= 0 {
            break;
        }
    }
}

// Minimal libc wrappers (no-std compatible stub in hosted env):
extern "C" {
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
}
const WNOHANG: i32 = 1;

fn libc_waitpid(pid: i32, status: *mut i32, options: i32) -> i32 {
    unsafe { waitpid(pid, status, options) }
}
const LIBC_WNOHANG: i32 = WNOHANG;

// ─── Logging helpers ──────────────────────────────────────────────────────────

fn log_info(msg: &str) {
    eprintln!("\x1b[32m[  OK  ]\x1b[0m {}", msg);
}

fn log_warn(msg: &str) {
    eprintln!("\x1b[33m[ WARN ]\x1b[0m {}", msg);
}

fn log_error(msg: &str) {
    eprintln!("\x1b[31m[ FAIL ]\x1b[0m {}", msg);
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    eprintln!();
    eprintln!("╔══════════════════════════════════════════╗");
    eprintln!("║       Black Wall Core — Init v1.0      ║");
    eprintln!("╚══════════════════════════════════════════╝");
    eprintln!();

    // Phase 1: mount pseudo-filesystems.
    mount_pseudo_filesystems();

    // Phase 2: set hostname.
    set_hostname();

    // Phase 3: start services.
    let mut running: Vec<RunningService> = Vec::new();

    for def in SERVICES {
        match start_service(def) {
            Ok(child) => {
                running.push(RunningService { def: def.clone(), child });
            }
            Err(e) => {
                log_warn(&format!(
                    "Could not start {}: {} (binary may not exist yet)",
                    def.name, e
                ));
            }
        }
    }

    log_info("All services started. Entering supervision loop.");
    eprintln!();

    // Phase 4: supervision + zombie reaping loop.
    loop {
        // Reap any dead children.
        reap_zombies();

        // Restart restartable services that have exited.
        for svc in running.iter_mut() {
            if !svc.def.restart {
                continue;
            }
            match svc.child.try_wait() {
                Ok(Some(status)) => {
                    log_warn(&format!(
                        "Service {} exited ({}), restarting...",
                        svc.def.name,
                        status.code().unwrap_or(-1)
                    ));
                    match start_service(&svc.def) {
                        Ok(new_child) => {
                            svc.child = new_child;
                            log_info(&format!("Service {} restarted", svc.def.name));
                        }
                        Err(e) => {
                            log_error(&format!("Failed to restart {}: {}", svc.def.name, e));
                        }
                    }
                }
                Ok(None) => {} // still running
                Err(e) => {
                    log_error(&format!("Error checking {}: {}", svc.def.name, e));
                }
            }
        }

        thread::sleep(Duration::from_secs(5));
    }
}
