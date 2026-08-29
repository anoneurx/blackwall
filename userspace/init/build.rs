use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=init.ld");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let ld_path = Path::new(&manifest_dir).join("init.ld");

    println!("cargo:rustc-link-arg=-T{}", ld_path.display());
}
