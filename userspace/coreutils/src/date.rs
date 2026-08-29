//! date — print or set the system date/time
use chrono::{Local, Utc};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut utc = false;
    let mut format = String::from("%a %b %e %T %Z %Y");

    for arg in args.iter().skip(1) {
        if arg == "-u" || arg == "--utc" {
            utc = true;
        } else if arg.starts_with('+') {
            format = arg[1..].to_string();
        }
    }

    let formatted = if utc {
        Utc::now().format(&format).to_string()
    } else {
        Local::now().format(&format).to_string()
    };
    println!("{}", formatted);
}
