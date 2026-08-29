//! cp — copy files and directories

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut recursive = false;
    let mut _force = false;
    let mut verbose = false;
    let mut sources: Vec<String> = Vec::new();

    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            for c in arg.chars().skip(1) {
                match c {
                    'r' | 'R' => recursive = true,
                    'f' => _force = true,
                    'v' => verbose = true,
                    _ => {}
                }
            }
        } else {
            sources.push(arg.clone());
        }
    }

    if sources.len() < 2 {
        eprintln!("cp: missing file operand");
        eprintln!("Usage: cp [-r] [-f] [-v] SOURCE... DEST");
        std::process::exit(1);
    }

    let dest = PathBuf::from(sources.pop().unwrap());
    let multi_src = sources.len() > 1;

    for src_str in &sources {
        let src = PathBuf::from(src_str);

        let target = if dest.is_dir() {
            dest.join(src.file_name().unwrap_or_default())
        } else if multi_src {
            eprintln!("cp: target '{}' is not a directory", dest.display());
            std::process::exit(1);
        } else {
            dest.clone()
        };

        if src.is_dir() {
            if !recursive {
                eprintln!("cp: -r not specified; omitting directory '{}'", src.display());
                continue;
            }
            if let Err(e) = copy_dir(&src, &target, verbose) {
                eprintln!("cp: {}: {}", src.display(), e);
            }
        } else {
            if verbose {
                println!("'{}' -> '{}'", src.display(), target.display());
            }
            if let Err(e) = std::fs::copy(&src, &target) {
                eprintln!("cp: cannot copy '{}' to '{}': {}", src.display(), target.display(), e);
            }
        }
    }
}

fn copy_dir(src: &PathBuf, dest: &PathBuf, verbose: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir(&src_path, &dest_path, verbose)?;
        } else {
            if verbose {
                println!("'{}' -> '{}'", src_path.display(), dest_path.display());
            }
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}
