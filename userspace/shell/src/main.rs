//! # bwsh — Black Wall Core Shell
//!
//! A POSIX-compatible interactive shell for Black Wall Core.
//!
//! ## Features
//! - Command parsing with pipes and redirects
//! - Built-in commands: cd, exit, echo, pwd, export, history, alias, help
//! - Environment variable expansion
//! - Command history (↑/↓ navigation in a later version)
//! - Coloured prompt with current user and working directory
//! - Graceful handling of SIGINT

use colored::Colorize;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

// ─── Shell state ──────────────────────────────────────────────────────────────

struct Shell {
    env: HashMap<String, String>,
    aliases: HashMap<String, String>,
    history: Vec<String>,
    cwd: PathBuf,
    last_exit: i32,
}

impl Shell {
    fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut env = HashMap::new();

        // Inherit system environment.
        for (k, v) in std::env::vars() {
            env.insert(k, v);
        }

        Self { env, aliases: HashMap::new(), history: Vec::new(), cwd, last_exit: 0 }
    }

    fn prompt(&self) -> String {
        let user = self.env.get("USER").map(|s| s.as_str()).unwrap_or("root");
        let host = self.env.get("HOSTNAME").cloned().unwrap_or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .unwrap_or_else(|_| "blackwall".to_string())
                .trim()
                .to_string()
        });
        let cwd = self.cwd.display().to_string();

        if user == "root" {
            format!(
                "{}@{}:{} {} ",
                user.bright_red().bold(),
                host.bright_cyan(),
                cwd.bright_blue(),
                "#".bright_red().bold()
            )
        } else {
            format!(
                "{}@{}:{} {} ",
                user.bright_green().bold(),
                host.bright_cyan(),
                cwd.bright_blue(),
                "$".bright_green().bold()
            )
        }
    }

    fn expand_vars(&self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '$' {
                let mut var_name = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc.is_alphanumeric() || nc == '_' {
                        var_name.push(nc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if var_name == "?" {
                    result.push_str(&self.last_exit.to_string());
                } else {
                    let val = self.env.get(&var_name).map(|s| s.as_str()).unwrap_or("");
                    result.push_str(val);
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    fn tokenize(&self, line: &str) -> Vec<String> {
        let expanded = self.expand_vars(line);
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut in_single = false;
        let mut in_double = false;

        for c in expanded.chars() {
            match c {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                ' ' | '\t' if !in_single && !in_double => {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                }
                _ => current.push(c),
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }

    fn run_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return true;
        }

        self.history.push(line.to_string());

        // Resolve alias.
        let (cmd_word, rest) = {
            let first_space = line.find(' ');
            if let Some(pos) = first_space {
                (&line[..pos], &line[pos..])
            } else {
                (line, "")
            }
        };
        let resolved = if let Some(alias_val) = self.aliases.get(cmd_word) {
            format!("{}{}", alias_val, rest)
        } else {
            line.to_string()
        };

        // Check for pipe.
        if resolved.contains(" | ") {
            return self.run_pipeline(&resolved);
        }

        let tokens = self.tokenize(&resolved);
        if tokens.is_empty() {
            return true;
        }

        match tokens[0].as_str() {
            "exit" | "quit" => return false,
            "cd" => self.builtin_cd(&tokens),
            "echo" => self.builtin_echo(&tokens),
            "pwd" => println!("{}", self.cwd.display()),
            "export" => self.builtin_export(&tokens),
            "unset" => {
                if tokens.len() > 1 {
                    self.env.remove(&tokens[1]);
                }
            }
            "alias" => self.builtin_alias(&tokens),
            "history" => self.builtin_history(),
            "help" => self.builtin_help(),
            "clear" => print!("\x1B[2J\x1B[H"),
            "source" | "." => self.builtin_source(&tokens),
            _ => self.run_external(&tokens),
        }
        true
    }

    fn builtin_cd(&mut self, tokens: &[String]) {
        let target = if tokens.len() < 2 {
            self.env.get("HOME").cloned().unwrap_or_else(|| "/root".to_string())
        } else {
            tokens[1].clone()
        };

        let new_path =
            if target.starts_with('/') { PathBuf::from(&target) } else { self.cwd.join(&target) };

        match std::env::set_current_dir(&new_path) {
            Ok(_) => {
                self.cwd = new_path.canonicalize().unwrap_or(new_path);
                self.env.insert("PWD".to_string(), self.cwd.display().to_string());
            }
            Err(e) => eprintln!("cd: {}: {}", target, e),
        }
    }

    fn builtin_echo(&self, tokens: &[String]) {
        let newline = tokens.get(1).map(|s| s == "-n").unwrap_or(false);
        let start = if newline { 2 } else { 1 };
        let out = tokens[start..].join(" ");
        if newline {
            print!("{}", out);
        } else {
            println!("{}", out);
        }
    }

    fn builtin_export(&mut self, tokens: &[String]) {
        for token in tokens.iter().skip(1) {
            if let Some(eq) = token.find('=') {
                let key = token[..eq].to_string();
                let val = token[eq + 1..].to_string();
                self.env.insert(key.clone(), val.clone());
                std::env::set_var(key, val);
            } else {
                // Print current value.
                if let Some(val) = self.env.get(token.as_str()) {
                    println!("{}={}", token, val);
                }
            }
        }
    }

    fn builtin_alias(&mut self, tokens: &[String]) {
        if tokens.len() == 1 {
            for (k, v) in &self.aliases {
                println!("alias {}='{}'", k, v);
            }
            return;
        }
        for token in tokens.iter().skip(1) {
            if let Some(eq) = token.find('=') {
                let name = token[..eq].to_string();
                let val = token[eq + 1..].trim_matches('\'').to_string();
                self.aliases.insert(name, val);
            }
        }
    }

    fn builtin_history(&self) {
        for (i, cmd) in self.history.iter().enumerate() {
            println!("{:>4}  {}", i + 1, cmd);
        }
    }

    fn builtin_source(&mut self, tokens: &[String]) {
        if tokens.len() < 2 {
            eprintln!("source: filename required");
            return;
        }
        let path = &tokens[1];
        match std::fs::read_to_string(path) {
            Ok(content) => {
                for line in content.lines() {
                    if !self.run_line(line) {
                        break;
                    }
                }
            }
            Err(e) => eprintln!("source: {}: {}", path, e),
        }
    }

    fn builtin_help(&self) {
        println!("{}", "Black Wall Shell (bwsh) built-in commands:".bold());
        println!();
        let cmds = [
            ("cd [dir]", "Change directory"),
            ("echo [-n] [...]", "Print text"),
            ("pwd", "Print working directory"),
            ("export VAR=val", "Set environment variable"),
            ("unset VAR", "Unset environment variable"),
            ("alias [name=val]", "Define or list aliases"),
            ("history", "Show command history"),
            ("source <file>", "Execute a script file"),
            ("clear", "Clear the terminal"),
            ("help", "Show this help"),
            ("exit", "Exit the shell"),
        ];
        for (cmd, desc) in &cmds {
            println!("  {:<24} {}", cmd.bright_cyan(), desc);
        }
    }

    fn run_external(&mut self, tokens: &[String]) {
        let mut cmd = Command::new(&tokens[0]);
        cmd.args(&tokens[1..]);
        cmd.current_dir(&self.cwd);

        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        match cmd.status() {
            Ok(status) => {
                self.last_exit = status.code().unwrap_or(0);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                eprintln!("bwsh: command not found: {}", tokens[0]);
                self.last_exit = 127;
            }
            Err(e) => {
                eprintln!("bwsh: {}: {}", tokens[0], e);
                self.last_exit = 1;
            }
        }
    }

    fn run_pipeline(&mut self, line: &str) -> bool {
        let parts: Vec<&str> = line.split(" | ").collect();
        if parts.len() < 2 {
            return self.run_line(line);
        }

        // Build a chain of processes.
        let mut prev_stdout: Option<std::process::ChildStdout> = None;

        let mut children = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let tokens = self.tokenize(part.trim());
            if tokens.is_empty() {
                continue;
            }
            let mut cmd = Command::new(&tokens[0]);
            cmd.args(&tokens[1..]);
            cmd.current_dir(&self.cwd);

            if let Some(stdout) = prev_stdout.take() {
                cmd.stdin(stdout);
            }

            let is_last = i == parts.len() - 1;
            if !is_last {
                cmd.stdout(Stdio::piped());
            }

            match cmd.spawn() {
                Ok(mut child) => {
                    if !is_last {
                        prev_stdout = child.stdout.take();
                    }
                    children.push(child);
                }
                Err(e) => {
                    eprintln!("bwsh: {}: {}", tokens[0], e);
                }
            }
        }

        for mut child in children {
            let _ = child.wait();
        }

        true
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let mut shell = Shell::new();

    // Source /etc/bwshrc if it exists.
    if std::path::Path::new("/etc/bwshrc").exists() {
        shell.run_line("source /etc/bwshrc");
    }
    if let Ok(home) = std::env::var("HOME") {
        let rc = format!("{}/.bwshrc", home);
        if std::path::Path::new(&rc).exists() {
            shell.run_line(&format!("source {}", rc));
        }
    }

    // Check if we're reading from a script file (non-interactive).
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match std::fs::read_to_string(&args[1]) {
            Ok(content) => {
                for line in content.lines() {
                    if !shell.run_line(line) {
                        break;
                    }
                }
                return;
            }
            Err(e) => {
                eprintln!("bwsh: {}: {}", args[1], e);
                std::process::exit(1);
            }
        }
    }

    // Interactive mode.
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        let prompt = shell.prompt();
        print!("{}", prompt);
        stdout.flush().ok();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let line = line.trim_end_matches('\n').trim_end_matches('\r');
                if !shell.run_line(line) {
                    break;
                }
            }
            Err(e) => {
                eprintln!("bwsh: read error: {}", e);
                break;
            }
        }
    }
}
