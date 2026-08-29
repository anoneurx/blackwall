//! # bwcron — Black Wall Core Cron Daemon
//!
//! Minimal cron daemon that reads `/etc/crontab` and `/etc/cron.d/*.cron`.
//!
//! ## Crontab format
//! Standard 5-field cron expression:
//! ```text
//! MIN HOUR DAY MON DOW command
//! * * * * * /usr/bin/some-script
//! ```
//! Comments (`#`) and blank lines are ignored.

use std::time::Duration;

// ─── Cron entry ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CronEntry {
    min: Field,
    hour: Field,
    day: Field,
    mon: Field,
    dow: Field,
    command: String,
}

#[derive(Debug, Clone)]
enum Field {
    Any,             // *
    Exact(u32),      // 5
    List(Vec<u32>),  // 1,2,3
    Range(u32, u32), // 1-5
    Step(u32),       // */5
}

impl Field {
    fn matches(&self, value: u32) -> bool {
        match self {
            Field::Any => true,
            Field::Exact(n) => *n == value,
            Field::List(ns) => ns.contains(&value),
            Field::Range(lo, hi) => value >= *lo && value <= *hi,
            Field::Step(step) => value % step == 0,
        }
    }
}

fn parse_field(s: &str) -> Field {
    if s == "*" {
        return Field::Any;
    }
    if s.starts_with("*/") {
        if let Ok(step) = s[2..].parse() {
            return Field::Step(step);
        }
    }
    if s.contains(',') {
        let ns: Vec<u32> = s.split(',').filter_map(|p| p.parse().ok()).collect();
        return Field::List(ns);
    }
    if s.contains('-') {
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        if parts.len() == 2 {
            if let (Ok(lo), Ok(hi)) = (parts[0].parse(), parts[1].parse()) {
                return Field::Range(lo, hi);
            }
        }
    }
    if let Ok(n) = s.parse() {
        return Field::Exact(n);
    }
    Field::Any
}

fn parse_line(line: &str) -> Option<CronEntry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let parts: Vec<&str> = line.splitn(6, ' ').collect();
    if parts.len() < 6 {
        return None;
    }

    Some(CronEntry {
        min: parse_field(parts[0]),
        hour: parse_field(parts[1]),
        day: parse_field(parts[2]),
        mon: parse_field(parts[3]),
        dow: parse_field(parts[4]),
        command: parts[5].to_string(),
    })
}

fn load_entries() -> Vec<CronEntry> {
    let mut entries = Vec::new();

    // Load /etc/crontab.
    if let Ok(content) = std::fs::read_to_string("/etc/crontab") {
        for line in content.lines() {
            if let Some(entry) = parse_line(line) {
                entries.push(entry);
            }
        }
    }

    // Load /etc/cron.d/*.cron.
    if let Ok(rd) = std::fs::read_dir("/etc/cron.d") {
        for file in rd.filter_map(|e| e.ok()) {
            if file.path().extension().map_or(false, |x| x == "cron") {
                if let Ok(content) = std::fs::read_to_string(file.path()) {
                    for line in content.lines() {
                        if let Some(entry) = parse_line(line) {
                            entries.push(entry);
                        }
                    }
                }
            }
        }
    }

    entries
}

// ─── Time helpers ─────────────────────────────────────────────────────────────

fn current_time() -> (u32, u32, u32, u32, u32) {
    let output = std::process::Command::new("date")
        .args(["+%M %H %d %m %u"])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            stdout: b"0 0 1 1 1".to_vec(),
            stderr: Vec::new(),
            status: std::process::ExitStatus::default(),
        });

    let s = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<u32> = s.trim().split_whitespace().filter_map(|p| p.parse().ok()).collect();

    if parts.len() < 5 {
        return (0, 0, 1, 1, 1);
    }
    (parts[0], parts[1], parts[2], parts[3], parts[4])
}

fn run_entry(entry: &CronEntry) {
    eprintln!("[cron] Running: {}", entry.command);
    let parts: Vec<&str> = entry.command.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }
    let mut cmd = std::process::Command::new(parts[0]);
    cmd.args(&parts[1..]);
    match cmd.spawn() {
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(e) => {
            eprintln!("[cron] Failed to run '{}': {}", entry.command, e);
        }
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    eprintln!("[bwcron] Black Wall Cron Daemon starting...");

    let mut last_min = u32::MAX;

    loop {
        let (min, hour, day, mon, dow) = current_time();

        if min != last_min {
            last_min = min;

            // Reload entries every minute (supports live edits to crontab).
            let entries = load_entries();

            for entry in &entries {
                if entry.min.matches(min)
                    && entry.hour.matches(hour)
                    && entry.day.matches(day)
                    && entry.mon.matches(mon)
                    && entry.dow.matches(dow)
                {
                    let entry_clone = entry.clone();
                    std::thread::spawn(move || run_entry(&entry_clone));
                }
            }
        }

        std::thread::sleep(Duration::from_secs(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_any_matches_all() {
        assert!(Field::Any.matches(0));
        assert!(Field::Any.matches(59));
        assert!(Field::Any.matches(23));
    }

    #[test]
    fn field_exact() {
        assert!(Field::Exact(15).matches(15));
        assert!(!Field::Exact(15).matches(14));
    }

    #[test]
    fn field_list() {
        let f = Field::List(vec![1, 5, 10]);
        assert!(f.matches(1));
        assert!(f.matches(5));
        assert!(!f.matches(3));
    }

    #[test]
    fn field_range() {
        let f = Field::Range(10, 20);
        assert!(f.matches(10));
        assert!(f.matches(15));
        assert!(f.matches(20));
        assert!(!f.matches(9));
        assert!(!f.matches(21));
    }

    #[test]
    fn field_step() {
        let f = Field::Step(5);
        assert!(f.matches(0));
        assert!(f.matches(5));
        assert!(f.matches(10));
        assert!(!f.matches(3));
    }

    #[test]
    fn parse_field_star() {
        assert!(matches!(parse_field("*"), Field::Any));
    }

    #[test]
    fn parse_field_step() {
        assert!(matches!(parse_field("*/15"), Field::Step(15)));
    }

    #[test]
    fn parse_field_list() {
        let f = parse_field("1,3,5");
        assert!(matches!(f, Field::List(_)));
        assert!(f.matches(1));
        assert!(f.matches(3));
        assert!(!f.matches(2));
    }

    #[test]
    fn parse_field_range() {
        let f = parse_field("10-20");
        assert!(matches!(f, Field::Range(10, 20)));
    }

    #[test]
    fn parse_field_exact() {
        assert!(matches!(parse_field("42"), Field::Exact(42)));
    }

    #[test]
    fn parse_line_blank() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
    }

    #[test]
    fn parse_line_comment() {
        assert!(parse_line("# this is a comment").is_none());
        assert!(parse_line("  # indented comment").is_none());
    }

    #[test]
    fn parse_line_valid() {
        let entry = parse_line("*/5 * * * * /usr/bin/test").unwrap();
        assert!(entry.min.matches(0));
        assert!(entry.min.matches(5));
        assert!(!entry.min.matches(3));
        assert!(entry.hour.matches(0));
        assert!(entry.command == "/usr/bin/test");
    }

    #[test]
    fn parse_line_insufficient_fields() {
        assert!(parse_line("* * * * ").is_none());
        assert!(parse_line("* * *").is_none());
    }

    #[test]
    fn parse_line_specific_time() {
        let entry = parse_line("30 14 * * * /usr/bin/backup").unwrap();
        assert!(entry.min.matches(30));
        assert!(!entry.min.matches(29));
        assert!(entry.hour.matches(14));
        assert!(!entry.hour.matches(13));
        assert_eq!(entry.command, "/usr/bin/backup");
    }
}
