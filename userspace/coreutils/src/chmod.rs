//! chmod — change file mode bits
use std::os::unix::fs::PermissionsExt;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut recursive = false;
    let mut operands: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg == "-R" || arg == "--recursive" {
            recursive = true;
        } else {
            operands.push(arg.clone());
        }
    }

    if operands.len() < 2 {
        eprintln!("chmod: usage: chmod [-R] MODE FILE...");
        std::process::exit(1);
    }

    let mode_str = &operands[0];
    let mode = parse_mode(mode_str);

    for file in &operands[1..] {
        if let Err(e) = apply_chmod(file, mode, recursive) {
            eprintln!("chmod: {}: {}", file, e);
        }
    }
}

fn apply_chmod(path: &str, mode: u32, recursive: bool) -> std::io::Result<()> {
    let p = std::path::Path::new(path);
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))?;
    if recursive && p.is_dir() {
        for entry in std::fs::read_dir(p)?.filter_map(|e| e.ok()) {
            apply_chmod(&entry.path().display().to_string(), mode, recursive)?;
        }
    }
    Ok(())
}

fn parse_mode(s: &str) -> u32 {
    // Octal mode string like "755" or "0644".
    u32::from_str_radix(s.trim_start_matches('0'), 8).unwrap_or(0o644)
}
