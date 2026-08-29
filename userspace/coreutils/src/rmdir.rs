//! rmdir — remove empty directories
fn main() {
    let args: Vec<String> = std::env::args().collect();
    for dir in args.iter().skip(1) {
        if let Err(e) = std::fs::remove_dir(dir) {
            eprintln!("rmdir: failed to remove '{}': {}", dir, e);
        }
    }
}
