//! # anxd — Black Wall Core Automatic Update Daemon
//!
//! Runs in the background and periodically checks the repository for updates.
//! Applies updates automatically when configured to do so, with automatic
//! rollback support (v2.0): before an update a file-level snapshot is taken
//! via `bwsnap`; if the update fails — or the configured post-update health
//! check reports the host unhealthy — the snapshot is restored.
//!
//! Configuration: `/etc/anx/update.toml`
//!
//! ```toml
//! interval = 3600            # check interval in seconds (default 3600)
//! auto_install = true        # automatically install updates (default true)
//! log_level = "info"
//!
//! rollback_enabled = true    # take a snapshot + restore on failure
//! snapshot_bin   = "/usr/bin/bwsnap"
//! snapshot_store = "/var/lib/bwsnap/snapshots"
//! snapshot_name  = "auto-update"
//! snapshot_paths = ["/etc/anx", "/etc/bwapi", "/etc/bwinit/services"]
//!
//! health_check = "/usr/local/bin/check-services"   # optional argv string
//! health_timeout_secs = 30
//! ```

use clap::Parser;
use serde::Deserialize;
use std::process::Command;
use std::time::Duration;

const CONFIG_PATH: &str = "/etc/anx/update.toml";

#[derive(Parser)]
#[command(name = "anxd", version = env!("CARGO_PKG_VERSION"), about = "Black Wall automatic update daemon")]
struct Cli {
    /// Path to the update configuration file
    #[arg(short, long, default_value = CONFIG_PATH)]
    config: String,
}

const DEFAULT_PATHS: &[&str] = &["/etc/anx", "/etc/bwapi", "/etc/bwinit/services"];

#[derive(Debug, Deserialize)]
struct UpdateConfig {
    #[serde(default = "default_interval")]
    interval: u64,
    #[serde(default = "default_auto")]
    auto_install: bool,
    #[serde(default = "default_log")]
    #[allow(dead_code)]
    log_level: String,

    #[serde(default = "default_true")]
    rollback_enabled: bool,
    #[serde(default = "default_snapshot_bin")]
    snapshot_bin: String,
    #[serde(default = "default_snapshot_store")]
    snapshot_store: String,
    #[serde(default = "default_snapshot_name")]
    snapshot_name: String,
    #[serde(default = "default_snapshot_paths")]
    snapshot_paths: Vec<String>,
    #[serde(default)]
    health_check: Option<String>,
    #[serde(default = "default_timeout")]
    health_timeout_secs: u64,
}

fn default_interval() -> u64 {
    3600
}
fn default_auto() -> bool {
    true
}
fn default_log() -> String {
    "info".to_string()
}
fn default_true() -> bool {
    true
}
fn default_snapshot_bin() -> String {
    "/usr/bin/bwsnap".to_string()
}
fn default_snapshot_store() -> String {
    "/var/lib/bwsnap/snapshots".to_string()
}
fn default_snapshot_name() -> String {
    "auto-update".to_string()
}
fn default_snapshot_paths() -> Vec<String> {
    DEFAULT_PATHS.iter().map(|s| s.to_string()).collect()
}
fn default_timeout() -> u64 {
    30
}

impl Default for UpdateConfig {
    fn default() -> Self {
        UpdateConfig {
            interval: default_interval(),
            auto_install: default_auto(),
            log_level: default_log(),
            rollback_enabled: default_true(),
            snapshot_bin: default_snapshot_bin(),
            snapshot_store: default_snapshot_store(),
            snapshot_name: default_snapshot_name(),
            snapshot_paths: default_snapshot_paths(),
            health_check: None,
            health_timeout_secs: default_timeout(),
        }
    }
}

fn load_config(path: &str) -> UpdateConfig {
    std::fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

fn log(level: &str, msg: &str) {
    let ts = Command::new("date")
        .args(["+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    eprintln!("[anxd] {} [{}] {}", ts, level.to_uppercase(), msg);
}

/// Split a command line into argv, respecting double quotes.
fn shlex_split(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut bytes = s.chars().peekable();
    while let Some(c) = bytes.next() {
        match c {
            '"' => in_quote = !in_quote,
            '\\' => {
                if let Some(&m) = bytes.peek() {
                    cur.push(m);
                    bytes.next();
                }
            }
            ' ' | '\t' if !in_quote => {
                if !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    args
}

/// Take a pre-update snapshot via bwsnap.
fn snapshot(cfg: &UpdateConfig) -> bool {
    if !cfg.rollback_enabled {
        return false;
    }
    // Drop any stale baseline so a fresh snapshot is taken each cycle.
    let _ = Command::new(&cfg.snapshot_bin)
        .args([
            format!("--store={}", cfg.snapshot_store),
            "delete".into(),
            cfg.snapshot_name.clone(),
        ])
        .output();

    let mut args = vec![
        format!("--store={}", cfg.snapshot_store),
        "create".to_string(),
        cfg.snapshot_name.clone(),
        "--json".to_string(),
    ];
    for p in &cfg.snapshot_paths {
        args.push("--path".to_string());
        args.push(p.clone());
    }
    match Command::new(&cfg.snapshot_bin).args(&args).output() {
        Ok(o) if o.status.success() => {
            log("info", &format!("pre-update snapshot '{}' taken", cfg.snapshot_name));
            true
        }
        Ok(o) => {
            log(
                "warn",
                &format!(
                    "failed to take pre-update snapshot: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            );
            false
        }
        Err(e) => {
            log("warn", &format!("cannot run bwsnap ({}), skipping snapshot", e));
            false
        }
    }
}

/// Restore the pre-update snapshot (rollback).
fn rollback(cfg: &UpdateConfig) {
    let args = vec![
        format!("--store={}", cfg.snapshot_store),
        "restore".to_string(),
        cfg.snapshot_name.clone(),
        "--json".to_string(),
    ];
    match Command::new(&cfg.snapshot_bin).args(&args).output() {
        Ok(o) if o.status.success() => {
            log("warn", &format!("rollback: restored pre-update snapshot '{}'", cfg.snapshot_name))
        }
        Ok(o) => {
            log("error", &format!("rollback FAILED: {}", String::from_utf8_lossy(&o.stderr).trim()))
        }
        Err(e) => log("error", &format!("rollback FAILED: cannot run bwsnap ({})", e)),
    }
}

/// Run the configured post-update health check. Returns true when healthy.
fn health_check_ok(cfg: &UpdateConfig) -> bool {
    let Some(hook) = &cfg.health_check else {
        return true;
    };
    if hook.trim().is_empty() {
        return true;
    }
    let argv = shlex_split(hook);
    if argv.is_empty() {
        return true;
    }
    log("info", &format!("running health check: {}", hook));

    let mut child = match Command::new(&argv[0]).args(&argv[1..]).spawn() {
        Ok(c) => c,
        Err(e) => {
            log("error", &format!("could not start health check ({}), marking unhealthy", e));
            return false;
        }
    };
    let timeout = Duration::from_secs(cfg.health_timeout_secs.max(1));
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    log("error", &format!("health check timed out after {}s", timeout.as_secs()));
                    return false;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                log("error", &format!("health check wait error: {}", e));
                return false;
            }
        }
    };
    if status.success() {
        log("info", "health check passed");
        true
    } else {
        log("error", &format!("health check FAILED (exit {:?})", status.code()));
        false
    }
}

/// One update cycle. Returns true if the system stayed healthy / was rolled back.
fn run_update(cfg: &UpdateConfig) -> bool {
    log("info", "Checking for package updates...");

    let snap_taken = snapshot(cfg);

    let output = Command::new("anx").args(["update", "--quiet"]).output();

    let update_ok = match &output {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            log("warn", &format!("Update failed: {}", String::from_utf8_lossy(&o.stderr).trim()));
            false
        }
        Err(e) => {
            log("error", &format!("Failed to run anx: {}", e));
            false
        }
    };

    if !update_ok {
        if snap_taken {
            rollback(cfg);
        }
        return false;
    }

    let output = output.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("up to date") {
        log("info", "All packages are up to date.");
    } else {
        log("info", "Update complete.");
    }

    if !health_check_ok(cfg) {
        if snap_taken {
            rollback(cfg);
        }
        return false;
    }
    true
}

fn main() {
    let cli = Cli::parse();
    log("info", "anxd automatic update daemon starting");

    let config = load_config(&cli.config);
    log(
        "info",
        &format!(
            "Update interval: {}s | Auto-install: {} | Rollback: {}",
            config.interval, config.auto_install, config.rollback_enabled
        ),
    );

    if config.health_check.is_some() {
        log(
            "info",
            &format!(
                "Health check hook: {} (timeout {}s)",
                config.health_check.as_deref().unwrap_or(""),
                config.health_timeout_secs
            ),
        );
    }

    loop {
        if config.auto_install {
            run_update(&config);
        } else {
            // Just refresh the index so `anx update` is instant when run manually.
            let _ = Command::new("anx").args(["refresh", "--quiet"]).status();
        }
        std::thread::sleep(Duration::from_secs(config.interval));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shlex_simple_args() {
        assert_eq!(shlex_split("ls -la"), vec!["ls", "-la"]);
    }

    #[test]
    fn shlex_quoted_string() {
        assert_eq!(shlex_split(r#"echo "hello world""#), vec!["echo", "hello world"]);
    }

    #[test]
    fn shlex_empty_string() {
        assert!(shlex_split("").is_empty());
    }

    #[test]
    fn shlex_whitespace_only() {
        assert!(shlex_split("   ").is_empty());
    }

    #[test]
    fn shlex_single_arg() {
        assert_eq!(shlex_split("ls"), vec!["ls"]);
    }

    #[test]
    fn shlex_mixed_quotes() {
        assert_eq!(shlex_split(r#"cmd "arg one" arg2"#), vec!["cmd", "arg one", "arg2"]);
    }

    #[test]
    fn shlex_escaped_quote() {
        assert_eq!(shlex_split(r#"echo \"hello\""#), vec!["echo", r#""hello""#]);
    }

    #[test]
    fn default_config_values() {
        let cfg = UpdateConfig::default();
        assert_eq!(cfg.interval, 3600);
        assert!(cfg.auto_install);
        assert_eq!(cfg.log_level, "info");
        assert!(cfg.rollback_enabled);
        assert_eq!(cfg.snapshot_bin, "/usr/bin/bwsnap");
        assert_eq!(cfg.health_timeout_secs, 30);
    }

    #[test]
    fn load_config_missing_file_returns_default() {
        let cfg = load_config("/nonexistent/path/config.toml");
        assert_eq!(cfg.interval, 3600);
        assert!(cfg.auto_install);
    }

    #[test]
    fn load_config_valid_toml() {
        let dir = std::env::temp_dir().join("anxd_test_config");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.toml");
        std::fs::write(&path, "interval = 7200\nauto_install = false\n").unwrap();
        let cfg = load_config(path.to_str().unwrap());
        assert_eq!(cfg.interval, 7200);
        assert!(!cfg.auto_install);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
