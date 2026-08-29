//! mv — move (rename) files

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut verbose = false;
    let mut force = false;
    let mut operands: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'v' => verbose = true,
                    'f' => force = true,
                    _ => {}
                }
            }
        } else {
            operands.push(arg.clone());
        }
    }

    if operands.len() < 2 {
        eprintln!("mv: missing operand\nUsage: mv [-v] SOURCE DEST");
        std::process::exit(1);
    }

    let dest = PathBuf::from(operands.pop().unwrap());
    for src_str in &operands {
        let src = PathBuf::from(src_str);
        let target = if dest.is_dir() {
            dest.join(src.file_name().unwrap_or_default())
        } else {
            dest.clone()
        };
        if !force && target.exists() {
            eprint!("mv: overwrite '{}'? ", target.display());
        }
        if verbose {
            println!("'{}' -> '{}'", src.display(), target.display());
        }
        if let Err(e) = std::fs::rename(&src, &target) {
            // Cross-device: fallback to copy + remove.
            if e.kind() == std::io::ErrorKind::CrossesDevices
                || e.raw_os_error() == Some(18 /*EXDEV*/)
            {
                if let Err(e2) = std::fs::copy(&src, &target) {
                    eprintln!("mv: cannot move '{}': {}", src.display(), e2);
                    continue;
                }
                let _ = std::fs::remove_file(&src);
            } else {
                eprintln!("mv: cannot move '{}' to '{}': {}", src.display(), target.display(), e);
            }
        }
    }
}
