//! rm — remove files or directories

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut recursive = false;
    let mut force = false;
    let mut verbose = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'r' | 'R' => recursive = true,
                    'f' => force = true,
                    'v' => verbose = true,
                    _ => {}
                }
            }
        } else {
            files.push(arg.clone());
        }
    }

    if files.is_empty() && !force {
        eprintln!("rm: missing operand");
        std::process::exit(1);
    }

    let mut exit_code = 0i32;
    for f in &files {
        let path = PathBuf::from(f);
        let result = if path.is_dir() {
            if recursive {
                if verbose {
                    println!("removed directory '{}'", path.display());
                }
                std::fs::remove_dir_all(&path)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::IsADirectory,
                    format!("is a directory (use -r to remove)"),
                ))
            }
        } else {
            if verbose {
                println!("removed '{}'", path.display());
            }
            std::fs::remove_file(&path)
        };
        if let Err(e) = result {
            if !force {
                eprintln!("rm: cannot remove '{}': {}", path.display(), e);
                exit_code = 1;
            }
        }
    }
    std::process::exit(exit_code);
}
