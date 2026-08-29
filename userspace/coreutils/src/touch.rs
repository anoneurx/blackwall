//! touch — change file timestamps / create empty files
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("touch: missing file operand");
        std::process::exit(1);
    }
    for file in args.iter().skip(1) {
        let path = Path::new(file);
        if path.exists() {
            // Bump mtime by opening for append (no-op write).
            let _ = std::fs::OpenOptions::new().append(true).open(path);
        } else {
            // Create the file.
            if let Err(e) = std::fs::File::create(path) {
                eprintln!("touch: cannot touch '{}': {}", file, e);
            }
        }
    }
}
