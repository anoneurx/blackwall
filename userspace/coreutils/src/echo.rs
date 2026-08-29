//! echo — write arguments to stdout
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut no_newline = false;
    let mut interpret_escapes = false;
    let mut start = 1;

    if args.len() > 1 {
        match args[1].as_str() {
            "-n" => {
                no_newline = true;
                start = 2;
            }
            "-e" => {
                interpret_escapes = true;
                start = 2;
            }
            "-ne" | "-en" => {
                no_newline = true;
                interpret_escapes = true;
                start = 2;
            }
            _ => {}
        }
    }

    let output = args[start..].join(" ");
    let output = if interpret_escapes { unescape(&output) } else { output };

    if no_newline {
        print!("{}", output);
    } else {
        println!("{}", output);
    }
}

fn unescape(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('0') => result.push('\0'),
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}
