#![no_std]
#![no_main]

use blackwall_shared::syscall::*;
use core::arch::asm;
use core::panic::PanicInfo;

// ── Raw syscalls (System V ABI: rdi, rsi, rdx …) ──────────────────────────────

fn write_str(s: &str) {
    unsafe {
        asm!(
            "syscall",
            in("rax") SYS_WRITE,
            in("rdi") 1, // stdout
            in("rsi") s.as_ptr() as u64,
            in("rdx") s.len() as u64,
            out("rcx") _,
            out("r11") _,
            options(nostack)
        );
    }
}

fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0;
    loop {
        let mut b = [0u8; 1];
        let bytes_read = unsafe {
            let r: u64;
            asm!(
                "syscall",
                in("rax") SYS_READ,
                in("rdi") 0, // stdin
                in("rsi") b.as_mut_ptr() as u64,
                in("rdx") 1,
                out("rcx") _,
                out("r11") _,
                lateout("rax") r,
                options(nostack)
            );
            r
        };
        if bytes_read == 0 || bytes_read == u64::MAX {
            continue;
        }
        let ch = b[0];
        if ch == b'\n' {
            break;
        }
        if len < buf.len() {
            buf[len] = ch;
            len += 1;
        }
    }
    len
}

fn syscall3(number: u64, a0: u64, a1: u64, a2: u64) -> u64 {
    let r: u64;
    unsafe {
        asm!(
            "syscall",
            in("rax") number,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            out("rcx") _,
            out("r11") _,
            lateout("rax") r,
            options(nostack)
        );
    }
    r
}

fn syscall1(number: u64, a0: u64) -> u64 {
    let r: u64;
    unsafe {
        asm!(
            "syscall",
            in("rax") number,
            in("rdi") a0,
            out("rcx") _,
            out("r11") _,
            lateout("rax") r,
            options(nostack)
        );
    }
    r
}

// ── Filesystem helper wrappers ───────────────────────────────────────────────

fn path_buf(path: &str) -> ([u8; 512], usize) {
    let mut buf = [0u8; 512];
    let copy_len = path.len().min(511);
    buf[..copy_len].copy_from_slice(&path.as_bytes()[..copy_len]);
    buf[copy_len] = 0;
    (buf, copy_len)
}

fn open_file(path: &str, flags: u64) -> i64 {
    let (pb, _) = path_buf(path);
    let fd = syscall3(SYS_OPEN, pb.as_ptr() as u64, flags, 0);
    fd as i64
}

fn close_file(fd: i64) {
    syscall1(SYS_CLOSE, fd as u64);
}

fn read_file(fd: i64, buf: &mut [u8]) -> i64 {
    let n = syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    n as i64
}

fn write_file(fd: i64, buf: &[u8]) -> i64 {
    let n = syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
    n as i64
}

fn mkdir_path(path: &str) -> i64 {
    let (pb, _) = path_buf(path);
    syscall1(SYS_MKDIR, pb.as_ptr() as u64) as i64
}

fn unlink_path(path: &str) -> i64 {
    let (pb, _) = path_buf(path);
    syscall1(SYS_UNLINK, pb.as_ptr() as u64) as i64
}

fn rmdir_path(path: &str) -> i64 {
    let (pb, _) = path_buf(path);
    syscall1(SYS_RMDIR, pb.as_ptr() as u64) as i64
}

/// List entries of `path`; prints them like a minimal `ls`.
fn ls_path(path: &str) {
    let (pb, _) = path_buf(path);
    let mut raw = [0u8; 4096];
    let n = syscall3(SYS_READDIR, pb.as_ptr() as u64, raw.as_mut_ptr() as u64, raw.len() as u64);
    if (n as i64) < 0 {
        write_str("ls: cannot list '");
        write_str(path);
        write_str("'\n");
        return;
    }
    let n = n as usize;
    let mut pos = 0usize;
    while pos + 2 <= n {
        let vtype = raw[pos];
        let nlen = raw[pos + 1] as usize;
        if pos + 2 + nlen > n {
            break;
        }
        let name = core::str::from_utf8(&raw[pos + 2..pos + 2 + nlen]).unwrap_or("?");
        if vtype == DT_DIR {
            write_str(name);
            write_str("/\n");
        } else {
            write_str(name);
            write_str("\n");
        }
        pos += 2 + nlen;
    }
}

// ── String helpers ───────────────────────────────────────────────────────────

fn strcmp(s1: &[u8], s2: &str) -> bool {
    if s1.len() != s2.len() {
        return false;
    }
    for i in 0..s1.len() {
        if s1[i] != s2.as_bytes()[i] {
            return false;
        }
    }
    true
}

/// Return the first space-separated word and the index just past its end.
fn word_end(s: &[u8]) -> usize {
    let mut i = 0;
    while i < s.len() && s[i] != b' ' && s[i] != b'\t' {
        i += 1;
    }
    i
}

fn path_of(cmd: &[u8], after: usize) -> &str {
    let mut start = after;
    while start < cmd.len() && (cmd[start] == b' ' || cmd[start] == b'\t') {
        start += 1;
    }
    core::str::from_utf8(&cmd[start..]).unwrap_or("")
}

// ── Command implementations ──────────────────────────────────────────────────

fn builtin_ls(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    ls_path(if path.is_empty() { "/" } else { path });
}

fn builtin_cat(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("cat: usage: cat <path>\n");
        return;
    }
    let fd = open_file(path, O_RDONLY);
    if fd < 0 {
        write_str("cat: file not found or permission denied\n");
        return;
    }
    let mut file_buf = [0u8; 512];
    loop {
        let n = read_file(fd, &mut file_buf);
        if n <= 0 {
            break;
        }
        if let Ok(content) = core::str::from_utf8(&file_buf[..n as usize]) {
            write_str(content);
        } else {
            write_str("[Binary data]\n");
            break;
        }
    }
    close_file(fd);
}

fn builtin_mkdir(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("mkdir: usage: mkdir <path>\n");
        return;
    }
    let r = mkdir_path(path);
    if r < 0 {
        write_str("mkdir: cannot create '");
        write_str(path);
        write_str("'\n");
    }
}

fn builtin_rm(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("rm: usage: rm <path>\n");
        return;
    }
    let r = unlink_path(path);
    if r < 0 {
        write_str("rm: cannot remove '");
        write_str(path);
        write_str("'\n");
    }
}

fn builtin_rmdir(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("rmdir: usage: rmdir <path>\n");
        return;
    }
    let r = rmdir_path(path);
    if r < 0 {
        write_str("rmdir: cannot remove '");
        write_str(path);
        write_str("'\n");
    }
}

fn builtin_touch(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("touch: usage: touch <path>\n");
        return;
    }
    let fd = open_file(path, O_WRONLY | O_CREAT);
    if fd < 0 {
        write_str("touch: cannot create '");
        write_str(path);
        write_str("'\n");
    } else {
        close_file(fd);
    }
}

fn builtin_echo(cmd: &[u8]) {
    let mut after = 4; // skip "echo"
    let mut newline = true;
    let we = word_end(&cmd[after..]);
    let word = &cmd[after..after + we];
    if strcmp(word, "-n") {
        newline = false;
        after += we;
    }

    // Detect "text > file" / "text >> file" redirect.
    let mut redirect = None;
    let mut text_len = cmd.len();
    let mut i = after;
    while i < cmd.len() {
        if cmd[i] == b'>' {
            let append = i + 1 < cmd.len() && cmd[i + 1] == b'>';
            let path_start = i + if append { 2 } else { 1 };
            text_len = i;
            let p = core::str::from_utf8(&cmd[path_start..]).unwrap_or("");
            redirect = Some((append, p.trim_start()));
            break;
        }
        i += 1;
    }

    let body = &cmd[after..text_len];
    let body = core::str::from_utf8(body).unwrap_or("").trim();
    let mut out = [0u8; 512];
    let mut out_len = 0;
    for &b in body.as_bytes() {
        if out_len < out.len() {
            out[out_len] = b;
            out_len += 1;
        }
    }
    if newline && out_len < out.len() {
        out[out_len] = b'\n';
        out_len += 1;
    }

    match redirect {
        Some((append, path)) if !path.is_empty() => {
            let flags = O_WRONLY | O_CREAT | if append { O_APPEND } else { O_TRUNC };
            let fd = open_file(path, flags);
            if fd < 0 {
                write_str("echo: cannot open '");
                write_str(path);
                write_str("'\n");
                return;
            }
            let _ = write_file(fd, &out[..out_len]);
            close_file(fd);
        }
        _ => {
            write_str(core::str::from_utf8(&out[..out_len]).unwrap_or(""));
        }
    }
}

// ── Main loop ────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write_str("\n==================================================\n");
    write_str("               Black Wall Core                \n");
    write_str("==================================================\n\n");
    write_str("Type 'help' to see list of available commands.\n\n");

    let mut input_buf = [0u8; 128];
    loop {
        write_str("blackwall> ");
        let len = read_line(&mut input_buf);
        let cmd = &input_buf[..len];
        if cmd.is_empty() {
            continue;
        }

        if word_end(cmd) == 0 {
            continue;
        }
        let word = &cmd[..word_end(cmd)];

        if strcmp(word, "help") {
            write_str("Available commands:\n");
            write_str("  help             Show this message\n");
            write_str("  ls [path]        List directory entries\n");
            write_str("  cat <path>       Display file contents\n");
            write_str("  mkdir <path>     Create a directory\n");
            write_str("  rmdir <path>     Remove an empty directory\n");
            write_str("  rm <path>        Remove a file\n");
            write_str("  touch <path>     Create an empty file\n");
            write_str("  echo [text]      Print text (supports > and >> redirect)\n");
            write_str("  pwd              Print working directory\n");
            write_str("  clear            Clear the terminal screen\n");
            write_str("  exit             Terminate userspace shell\n");
        } else if strcmp(word, "ls") {
            builtin_ls(cmd);
        } else if strcmp(word, "cat") {
            builtin_cat(cmd);
        } else if strcmp(word, "mkdir") {
            builtin_mkdir(cmd);
        } else if strcmp(word, "rmdir") {
            builtin_rmdir(cmd);
        } else if strcmp(word, "rm") {
            builtin_rm(cmd);
        } else if strcmp(word, "touch") {
            builtin_touch(cmd);
        } else if strcmp(word, "echo") {
            builtin_echo(cmd);
        } else if strcmp(word, "pwd") {
            write_str("/\n");
        } else if strcmp(word, "clear") {
            write_str("\x1B[2J\x1B[H");
        } else if strcmp(word, "exit") {
            write_str("Exiting shell...\n");
            unsafe {
                asm!(
                    "syscall",
                    in("rax") SYS_EXIT,
                    in("rdi") 0,
                    out("rcx") _,
                    out("r11") _,
                    options(nostack)
                );
            }
        } else {
            write_str("shell: command not found: ");
            if let Ok(s) = core::str::from_utf8(cmd) {
                write_str(s);
            }
            write_str("\n");
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}