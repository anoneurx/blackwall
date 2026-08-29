//! Lockfile management — prevents concurrent `anx` invocations from corrupting state.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const LOCK_PATH: &str = "/var/lib/anx/lock";

pub struct Lockfile;

impl Lockfile {
    /// Acquire the global anx lockfile.
    /// Returns an error if another process holds it.
    pub fn acquire() -> Result<Self> {
        if Path::new(LOCK_PATH).exists() {
            // Check if the PID in the lockfile is still alive.
            let old_pid = fs::read_to_string(LOCK_PATH)
                .unwrap_or_default()
                .trim()
                .parse::<u32>()
                .unwrap_or(0);

            if old_pid > 0 && process_alive(old_pid) {
                anyhow::bail!(
                    "anx is already running (PID {}). \
                     If this is a stale lock, remove {} and try again.",
                    old_pid,
                    LOCK_PATH
                );
            }
            // Stale lock — remove it.
            let _ = fs::remove_file(LOCK_PATH);
        }

        let pid = std::process::id();
        fs::create_dir_all(Path::new(LOCK_PATH).parent().unwrap_or(Path::new("/var/lib/anx")))
            .context("Failed to create lockfile directory")?;

        fs::write(LOCK_PATH, pid.to_string()).context("Failed to create lockfile")?;

        Ok(Lockfile)
    }
}

impl Drop for Lockfile {
    fn drop(&mut self) {
        let _ = fs::remove_file(LOCK_PATH);
    }
}

fn process_alive(pid: u32) -> bool {
    // On Linux, check /proc/<pid>.
    Path::new(&format!("/proc/{}", pid)).exists()
}
