//! mkdir — make directories

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut parents = false;
    let mut verbose = false;
    let mut mode: Option<u32> = None;
    let mut dirs: Vec<String> = Vec::new();
    let mut skip_next = false;

    for (i, arg) in args.iter().skip(1).enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        match arg.as_str() {
            "-p" | "--parents" => parents = true,
            "-v" | "--verbose" => verbose = true,
            "-m" | "--mode" => {
                if let Some(m) = args.get(i + 2) {
                    mode = u32::from_str_radix(m, 8).ok();
                    skip_next = true;
                }
            }
            a if a.starts_with('-') => {
                for c in a.chars().skip(1) {
                    match c {
                        'p' => parents = true,
                        'v' => verbose = true,
                        _ => {}
                    }
                }
            }
            _ => dirs.push(arg.clone()),
        }
    }

    for dir in &dirs {
        let result = if parents { std::fs::create_dir_all(dir) } else { std::fs::create_dir(dir) };
        match result {
            Ok(_) => {
                if verbose {
                    println!("mkdir: created directory '{}'", dir);
                }
                // Apply mode if specified.
                if let Some(m) = mode {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(m));
                }
            }
            Err(e) => eprintln!("mkdir: cannot create directory '{}': {}", dir, e),
        }
    }
}
