//! ls — list directory contents

use colored::Colorize;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut show_all = false;
    let mut long_format = false;
    let mut human_sizes = false;
    let mut paths: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'a' => show_all = true,
                    'l' => long_format = true,
                    'h' => human_sizes = true,
                    _ => {}
                }
            }
        } else {
            paths.push(arg.clone());
        }
    }

    if paths.is_empty() {
        paths.push(".".to_string());
    }

    let multi = paths.len() > 1;

    for (i, path) in paths.iter().enumerate() {
        if multi {
            if i > 0 {
                println!();
            }
            println!("{}:", path);
        }

        let p = PathBuf::from(path);
        if p.is_dir() {
            list_dir(&p, show_all, long_format, human_sizes);
        } else {
            print_entry(&p, long_format, human_sizes);
        }
    }
}

fn list_dir(dir: &PathBuf, show_all: bool, long: bool, human: bool) {
    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            eprintln!("ls: {}: {}", dir.display(), e);
            return;
        }
    };

    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !show_all && name_str.starts_with('.') {
            continue;
        }
        print_entry(&entry.path(), long, human);
    }
    if !long {
        println!();
    }
}

fn print_entry(path: &PathBuf, long: bool, human: bool) {
    let meta = path.symlink_metadata();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    if long {
        if let Ok(m) = meta {
            let mode = m.permissions().mode();
            let size = m.size();
            let is_dir = m.is_dir();
            let is_link = m.file_type().is_symlink();

            let type_char = if is_dir {
                'd'
            } else if is_link {
                'l'
            } else {
                '-'
            };
            let mode_str = format_mode(mode);
            let size_str = if human { human_size(size) } else { size.to_string() };

            let colored_name = if is_dir {
                name.bright_blue().bold().to_string()
            } else if mode & 0o111 != 0 {
                name.bright_green().to_string()
            } else if is_link {
                name.bright_cyan().to_string()
            } else {
                name.clone()
            };

            println!("{}{} {:>8}  {}", type_char, mode_str, size_str, colored_name);
        } else {
            println!("?--------- {:>8}  {}", "?", name);
        }
    } else {
        // Short format — just color-coded names.
        if let Ok(m) = path.metadata() {
            if m.is_dir() {
                print!("{}  ", name.bright_blue().bold());
            } else if m.permissions().mode() & 0o111 != 0 {
                print!("{}  ", name.bright_green());
            } else {
                print!("{}  ", name);
            }
        } else {
            print!("{}  ", name);
        }
    }
}

fn format_mode(mode: u32) -> String {
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    bits.iter().map(|(b, c)| if mode & b != 0 { *c } else { '-' }).collect()
}

fn human_size(bytes: u64) -> String {
    const K: u64 = 1024;
    if bytes < K {
        format!("{}B", bytes)
    } else if bytes < K * K {
        format!("{:.1}K", bytes as f64 / K as f64)
    } else if bytes < K * K * K {
        format!("{:.1}M", bytes as f64 / (K * K) as f64)
    } else {
        format!("{:.1}G", bytes as f64 / (K * K * K) as f64)
    }
}
