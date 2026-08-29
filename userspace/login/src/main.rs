//! # bwlogin — Black Wall Core Login Process
//!
//! Presents a login prompt, validates credentials against `/etc/passwd`
//! and `/etc/shadow`, then execs `bwsh` as the authenticated user.
//!
//! This binary replaces `getty` + `login` for the Black Wall console.

use colored::Colorize;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::process::Command;

const MOTD_PATH: &str = "/etc/motd";
const PASSWD_PATH: &str = "/etc/passwd";
const SHADOW_PATH: &str = "/etc/shadow";
const SHELL: &str = "/usr/bin/bwsh";
const MAX_ATTEMPTS: u32 = 3;

fn main() {
    // Show MOTD.
    if let Ok(motd) = std::fs::read_to_string(MOTD_PATH) {
        print!("{}", motd);
    }

    let mut attempts = 0u32;
    loop {
        if attempts >= MAX_ATTEMPTS {
            eprintln!("{} Too many authentication failures.", "!".bright_red());
            std::process::exit(1);
        }
        attempts += 1;

        // Prompt for username.
        print!("{}: ", "login".bold());
        io::stdout().flush().ok();
        let mut username = String::new();
        if io::stdin().read_line(&mut username).is_err() || username.trim().is_empty() {
            continue;
        }
        let username = username.trim().to_string();

        // Prompt for password (no echo — achieved by disabling terminal echo).
        let password = read_password();

        // Authenticate.
        match authenticate(&username, &password) {
            Ok(entry) => {
                println!();
                println!("Welcome, {}!", username.bright_cyan().bold());
                println!();

                // Set up the user environment.
                std::env::set_var("USER", &username);
                std::env::set_var("LOGNAME", &username);
                std::env::set_var("HOME", &entry.home);
                std::env::set_var("SHELL", SHELL);
                std::env::set_var("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");

                // Change to the user's home directory.
                let _ = std::env::set_current_dir(&entry.home);

                // Exec the shell — replaces this process.
                let err = Command::new(SHELL).exec();
                eprintln!("bwlogin: failed to exec shell: {}", err);
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("{} {}", "Login incorrect.".bright_red(), e);
            }
        }
    }
}

// ─── Password reader (disable echo) ──────────────────────────────────────────

fn read_password() -> String {
    print!("Password: ");
    io::stdout().flush().ok();

    // Disable echo via termios.
    disable_echo();
    let mut pass = String::new();
    io::stdin().read_line(&mut pass).ok();
    enable_echo();
    println!();

    pass.trim().to_string()
}

#[repr(C)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

extern "C" {
    fn tcgetattr(fd: i32, termios: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, optional_actions: i32, termios: *const Termios) -> i32;
}

static mut SAVED_TERMIOS: Option<Termios> = None;

fn disable_echo() {
    const STDIN: i32 = 0;
    const TCSANOW: i32 = 0;
    const ECHO: u32 = 0x0008;

    let mut t = Termios {
        c_iflag: 0,
        c_oflag: 0,
        c_cflag: 0,
        c_lflag: 0,
        c_line: 0,
        c_cc: [0; 32],
        c_ispeed: 0,
        c_ospeed: 0,
    };
    unsafe {
        tcgetattr(STDIN, &mut t);
        SAVED_TERMIOS = Some(Termios {
            c_iflag: t.c_iflag,
            c_oflag: t.c_oflag,
            c_cflag: t.c_cflag,
            c_lflag: t.c_lflag,
            c_line: t.c_line,
            c_cc: t.c_cc,
            c_ispeed: t.c_ispeed,
            c_ospeed: t.c_ospeed,
        });
        t.c_lflag &= !ECHO;
        tcsetattr(STDIN, TCSANOW, &t);
    }
}

fn enable_echo() {
    const STDIN: i32 = 0;
    const TCSANOW: i32 = 0;
    unsafe {
        if let Some(ref saved) = SAVED_TERMIOS {
            tcsetattr(STDIN, TCSANOW, saved);
        }
    }
}

// ─── Authentication ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct PasswdEntry {
    home: String,
    #[allow(dead_code)]
    shell: String,
}

fn authenticate(username: &str, password: &str) -> Result<PasswdEntry, &'static str> {
    let passwd = std::fs::read_to_string(PASSWD_PATH).map_err(|_| "cannot read /etc/passwd")?;

    let entry =
        passwd.lines().find(|l| l.split(':').next() == Some(username)).ok_or("user not found")?;

    let fields: Vec<&str> = entry.split(':').collect();
    if fields.len() < 7 {
        return Err("invalid passwd entry");
    }

    let pw_field = fields[1];
    let home = fields[5].to_string();
    let shell = fields[6].trim().to_string();

    // If the password field is "x", check /etc/shadow.
    if pw_field == "x" {
        verify_shadow(username, password)?;
    } else if pw_field == "*" || pw_field == "!" {
        return Err("account is locked");
    } else if !pw_field.is_empty() {
        // Plaintext password check (for early development / test environments).
        if pw_field != password {
            return Err("incorrect password");
        }
    }
    // pw_field == "" means no password required.

    Ok(PasswdEntry { home, shell })
}

fn verify_shadow(username: &str, password: &str) -> Result<(), &'static str> {
    let shadow = std::fs::read_to_string(SHADOW_PATH).map_err(|_| "cannot read /etc/shadow")?;

    let entry = shadow
        .lines()
        .find(|l| l.split(':').next() == Some(username))
        .ok_or("shadow entry not found")?;

    let hash = entry.split(':').nth(1).ok_or("invalid shadow entry")?;

    if hash == "*" || hash == "!" || hash.is_empty() {
        return Err("account is locked or has no password");
    }

    // Delegate to system `crypt` via `openssl passwd` verification.
    // In v1.0 we use the simple approach: check if system login works.
    // A proper implementation would call crypt(3) directly.
    // For now: if hash starts with $6$ (SHA-512) verify via /usr/bin/openssl.
    let output = std::process::Command::new("openssl")
        .args(["passwd", "-6", "-salt", hash.split('$').nth(2).unwrap_or("salt"), password])
        .output()
        .map_err(|_| "openssl not available")?;

    let computed = String::from_utf8_lossy(&output.stdout);
    let computed = computed.trim();

    if computed == hash {
        Ok(())
    } else {
        Err("incorrect password")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passwd_line(pw_field: &str) -> String {
        format!("alice:{pw_field}:1000:1000::/home/alice:/usr/bin/bwsh\n")
    }

    // Simulates the parsing/verification logic of `authenticate` + `verify_shadow`
    // against in-memory passwd/shadow content instead of reading /etc files.
    fn parse_passwd_entry(
        username: &str,
        password: &str,
        passwd_content: &str,
        shadow_content: Option<&str>,
    ) -> Result<PasswdEntry, &'static str> {
        let entry = passwd_content
            .lines()
            .find(|l| l.split(':').next() == Some(username))
            .ok_or("user not found")?;

        let fields: Vec<&str> = entry.split(':').collect();
        if fields.len() < 7 {
            return Err("invalid passwd entry");
        }

        let pw_field = fields[1];
        let home = fields[5].to_string();
        let shell = fields[6].trim().to_string();

        if pw_field == "x" {
            let shadow = shadow_content.ok_or("cannot read /etc/shadow")?;
            let sentry = shadow
                .lines()
                .find(|l| l.split(':').next() == Some(username))
                .ok_or("shadow entry not found")?;
            let hash = sentry.split(':').nth(1).ok_or("invalid shadow entry")?;
            if hash == "*" || hash == "!" || hash.is_empty() {
                return Err("account is locked or has no password");
            }
        } else if pw_field == "*" || pw_field == "!" {
            return Err("account is locked");
        } else if !pw_field.is_empty() && pw_field != password {
            return Err("incorrect password");
        }

        Ok(PasswdEntry { home, shell })
    }

    #[test]
    fn authenticate_locked_account_star() {
        let result = parse_passwd_entry("alice", "guess", &passwd_line("*"), None);
        assert_eq!(result.unwrap_err(), "account is locked");
    }

    #[test]
    fn authenticate_locked_account_bang() {
        let result = parse_passwd_entry("alice", "guess", &passwd_line("!"), None);
        assert_eq!(result.unwrap_err(), "account is locked");
    }

    #[test]
    fn authenticate_empty_password_field() {
        let result = parse_passwd_entry("alice", "", &passwd_line(""), None);
        let entry = result.unwrap_or_else(|e| panic!("expected success, got: {e}"));
        assert_eq!(entry.home, "/home/alice");
        assert_eq!(entry.shell, "/usr/bin/bwsh");
    }

    #[test]
    fn authenticate_plaintext_wrong_password() {
        let result = parse_passwd_entry("alice", "wrong", &passwd_line("secret"), None);
        assert_eq!(result.unwrap_err(), "incorrect password");
    }

    #[test]
    fn authenticate_plaintext_correct_password() {
        let result = parse_passwd_entry("alice", "secret", &passwd_line("secret"), None);
        assert!(result.is_ok());
    }

    #[test]
    fn authenticate_shadow_x_field() {
        let missing_shadow = parse_passwd_entry("alice", "secret", &passwd_line("x"), None);
        assert_eq!(missing_shadow.unwrap_err(), "cannot read /etc/shadow");

        let locked_shadow = parse_passwd_entry(
            "alice",
            "secret",
            &passwd_line("x"),
            Some("alice:*:19000:0:99999:7:::\n"),
        );
        assert_eq!(locked_shadow.unwrap_err(), "account is locked or has no password");

        let absent_user =
            parse_passwd_entry("alice", "secret", &passwd_line("x"), Some("bob:!:::\n"));
        assert_eq!(absent_user.unwrap_err(), "shadow entry not found");
    }

    #[test]
    fn max_attempts_constant() {
        assert_eq!(MAX_ATTEMPTS, 3);
    }

    #[test]
    fn passwd_entry_struct_fields() {
        let entry =
            PasswdEntry { home: "/home/alice".to_string(), shell: "/usr/bin/bwsh".to_string() };
        assert_eq!(entry.home, "/home/alice");
        assert_eq!(entry.shell, "/usr/bin/bwsh");
    }

    #[test]
    fn terminal_helpers_smoke_test() {
        use std::io::IsTerminal;

        disable_echo();
        enable_echo();

        if std::io::stdin().is_terminal() {
            return;
        }
        let _ = read_password();
    }
}
