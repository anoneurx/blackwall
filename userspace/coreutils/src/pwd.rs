//! pwd — print working directory
fn main() {
    match std::env::current_dir() {
        Ok(p) => println!("{}", p.display()),
        Err(e) => {
            eprintln!("pwd: {}", e);
            std::process::exit(1);
        }
    }
}
