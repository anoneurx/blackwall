fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("build") => {
            std::process::Command::new("make").arg("build").status().unwrap();
        }
        Some("run") => {
            std::process::Command::new("make").arg("run").status().unwrap();
        }
        Some("test") => {
            std::process::Command::new("make").arg("test").status().unwrap();
        }
        Some("boot") => {
            std::process::Command::new("make").arg("build").status().unwrap();
            std::process::Command::new("make").arg("run").status().unwrap();
        }
        _ => println!("usage: cargo xtask <build|run|test|boot>"),
    }
}
