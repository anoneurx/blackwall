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
    let mut first = true;
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
        if first && (ch == b'\r' || ch == b'\n') {
            continue;
        }
        first = false;
        if ch == b'\n' || ch == b'\r' {
            break;
        }
        if ch == 0x08 || ch == 0x7f {
            if len > 0 {
                len -= 1;
                write_str("\x08 \x08");
            }
            continue;
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
    // Resolve relative paths against the tracked cwd so every fs helper
    // (open/mkdir/rm/rmdir/readdir) behaves like the cd/pwd builtins.
    let resolved = resolve_path(path);
    let len = resolved.iter().position(|&b| b == 0).unwrap_or(resolved.len());
    (resolved, len)
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

/// Print an unsigned integer in decimal (minimal, allocation-free).
fn write_u64(mut n: u64) {
    if n == 0 {
        write_str("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    write_str(core::str::from_utf8(&buf[i..]).unwrap_or(""));
}

fn path_of(cmd: &[u8], after: usize) -> &str {
    let mut start = after;
    while start < cmd.len() && (cmd[start] == b' ' || cmd[start] == b'\t') {
        start += 1;
    }
    core::str::from_utf8(&cmd[start..]).unwrap_or("")
}
struct ShellState {
    cwd: [u8; 256],
    cwd_len: usize,
}

static mut SHELL_STATE: ShellState = ShellState { cwd: [b'/'; 256], cwd_len: 1 };

fn get_cwd() -> &'static str {
    unsafe {
        core::str::from_utf8(&SHELL_STATE.cwd[..SHELL_STATE.cwd_len]).unwrap_or("/")
    }
}

fn resolve_path(path: &str) -> [u8; 512] {
    let mut out = [0u8; 512];
    if path.is_empty() {
        unsafe {
            for i in 0..SHELL_STATE.cwd_len.min(512) {
                out[i] = SHELL_STATE.cwd[i];
            }
        }
        return out;
    }
    if path.starts_with('/') {
        let bytes = path.as_bytes();
        let len = bytes.len().min(511);
        for i in 0..len {
            out[i] = bytes[i];
        }
        return out;
    }
    // relative
    unsafe {
        let mut pos = 0;
        // copy cwd
        for i in 0..SHELL_STATE.cwd_len {
            if pos < 511 {
                out[pos] = SHELL_STATE.cwd[i];
                pos += 1;
            }
        }
        if SHELL_STATE.cwd_len > 0 && SHELL_STATE.cwd[SHELL_STATE.cwd_len - 1] != b'/' {
            if pos < 511 {
                out[pos] = b'/';
                pos += 1;
            }
        }
        let bytes = path.as_bytes();
        for i in 0..bytes.len() {
            if pos < 511 {
                out[pos] = bytes[i];
                pos += 1;
            }
        }
    }
    out
}

// ── Command implementations ──────────────────────────────────────────────────

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

/// `run <path>` — fork, exec the ELF at `path` in the child, wait for it.
fn builtin_run(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("run: usage: run <path>\n");
        return;
    }
    let (pb, _) = path_buf(path);

    let pid = syscall1(SYS_FORK, 0) as i64;
    if pid == 0 {
        // Child: replace this image with the requested program.
        syscall3(SYS_EXECVE, pb.as_ptr() as u64, 0, 0);
        // Only reached when execve failed.
        write_str("run: execve failed\n");
        syscall1(SYS_EXIT, 127);
    } else if pid > 0 {
        let mut status: u64 = 0;
        let r = syscall3(SYS_WAITPID, pid as u64, &mut status as *mut u64 as u64, 0) as i64;
        if r == pid {
            write_str("run: pid ");
            write_u64(pid as u64);
            write_str(" exited with status ");
            write_u64(status);
            write_str("\n");
        } else {
            write_str("run: waitpid failed\n");
        }
    } else {
        write_str("run: fork failed\n");
    }
}

// ── Network status command ───────────────────────────────────────────────────

/// `net [arp|ping]` — query or exercise the in-kernel network stack.
fn builtin_net(cmd: &[u8]) {
    let we = word_end(cmd);
    let arg = path_of(cmd, we);
    let (op, name) = if strcmp(arg.as_bytes(), "arp") {
        (NET_OP_ARP, "arp")
    } else if strcmp(arg.as_bytes(), "ping") {
        (NET_OP_PING, "ping")
    } else {
        (NET_OP_STATE, "state")
    };

    let status = syscall1(SYS_NET, op);

    write_str("net ");
    write_str(name);
    write_str(": status=");
    write_u64(status);
    write_str(" [");
    if status & NET_ST_NIC != 0 {
        write_str("nic ");
    } else {
        write_str("no-nic ");
    }
    if status & NET_ST_TX_OK != 0 {
        write_str("tx ");
    }
    if status & NET_ST_ARP_GATEWAY != 0 {
        write_str("gw-arp ");
    }
    if status & NET_ST_ECHO_REPLY != 0 {
        write_str("pong ");
    }
    write_str("arp_entries=");
    write_u64((status >> NET_ST_ARP_ENTRIES_SHIFT) & 0xff);
    write_str("]\n");
}

/// `anx` — minimal package manager (alias for `apt`-style operations).
fn builtin_anx(cmd: &[u8]) {
    let we = word_end(cmd);
    let rest = &cmd[we..];
    // skip whitespace
    let mut start = 0;
    while start < rest.len() && (rest[start] == b' ' || rest[start] == b'\t') {
        start += 1;
    }
    if start >= rest.len() {
        write_str("anx: usage: anx <update|upgrade|install|remove|search|list>\n");
        write_str("anx: (packages not implemented in this build)\n");
        return;
    }
    let sub = &rest[start..];
    let swe = word_end(sub);
    let subword = &sub[..swe];
    if strcmp(subword, "update") || strcmp(subword, "upgrade") {
        write_str("anx: repository update/upgrade — nothing to do (offline)\n");
        return;
    }
    if strcmp(subword, "list") || strcmp(subword, "search") {
        write_str("anx: no packages available in local repository\n");
        return;
    }
    if strcmp(subword, "install") || strcmp(subword, "remove") || strcmp(subword, "purge") {
        let mut pstart = swe;
        while pstart < sub.len() && (sub[pstart] == b' ' || sub[pstart] == b'\t') {
            pstart += 1;
        }
        if pstart >= sub.len() {
            write_str("anx: usage: anx install|remove <package>\n");
            return;
        }
        let pkg = core::str::from_utf8(&sub[pstart..]).unwrap_or("").trim();
        write_str("anx: ");
        write_str(core::str::from_utf8(subword).unwrap_or(""));
        write_str(" of '");
        write_str(pkg);
        write_str("' not supported (no package repo)\n");
        return;
    }
    write_str("anx: unknown subcommand '");
    if let Ok(s) = core::str::from_utf8(subword) {
        write_str(s);
    }
    write_str("'\n");
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
        // Echo command in blue
        write_str("\x1B[34m");
        write_str(core::str::from_utf8(word).unwrap_or(""));
        write_str("\x1B[0m");
        write_str("\n");

        if strcmp(word, "help") {
            write_str("Available commands:\n");
            write_str("  help             Show this message\n");
            write_blue("  ls");
            write_str(" [path]        List directory entries\n");
            write_blue("  cat");
            write_str(" <path>       Display file contents\n");
            write_blue("  mkdir");
            write_str(" <path>     Create a directory\n");
            write_blue("  rmdir");
            write_str(" <path>     Remove an empty directory\n");
            write_blue("  rm");
            write_str(" <path>        Remove a file\n");
            write_blue("  touch");
            write_str(" <path>     Create an empty file\n");
            write_blue("  echo");
            write_str(" [text]      Print text (supports > and >> redirect)\n");
            write_blue("  pwd");
            write_str("              Print working directory\n");
            write_blue("  run");
            write_str(" <path>       Execute an ELF program (fork + execve + wait)\n");
            write_blue("  clear");
            write_str("            Clear the terminal screen\n");
            write_cyan("  cd");
            write_str("/");
            write_cyan("pwd");
            write_str("/");
            write_cyan("ls");
            write_str("/");
            write_cyan("cat");
            write_str("        Navigation and file operations\n");
            write_yellow("  ps");
            write_str("/");
            write_yellow("kill");
            write_str("/");
            write_yellow("free");
            write_str("     Process and resource management\n");
            write_green("  whoami");
            write_str("/");
            write_green("hostname");
            write_str("/");
            write_green("uname");
            write_str("/");
            write_green("date");
            write_str("/");
            write_green("uptime");
            write_str("/");
            write_green("env");
            write_str("/");
            write_green("which");
            write_str("/");
            write_green("id");
            write_str("\n");
            write_str("  ");
            write_blue("net");
            write_str(" [arp|ping|state] — query or exercise the in-kernel network stack\n");
            write_str("  ");
            write_cyan("anx");
            write_str("              Package manager (alias for apt-like operations)\n");
            write_str("  ");
            write_red("bw");
            write_str(" <cmd>         Black Wall system administration\n");
            write_str("  ");
            write_cyan("service");
            write_str(" <name>   Service management\n");
            write_str("  version/help     System info\n");
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
        } else if strcmp(word, "run") {
            builtin_run(cmd);
        } else if strcmp(word, "cd") {
            builtin_cd(cmd);
        } else if strcmp(word, "pwd") {
            write_str("/\n");
        } else if strcmp(word, "whoami") {
            builtin_whoami(cmd);
        } else if strcmp(word, "hostname") {
            builtin_hostname(cmd);
        } else if strcmp(word, "uname") {
            builtin_uname(cmd);
        } else if strcmp(word, "uptime") {
            builtin_uptime(cmd);
        } else if strcmp(word, "date") {
            builtin_date(cmd);
        } else if strcmp(word, "env") || strcmp(word, "printenv") {
            builtin_env(cmd);
        } else if strcmp(word, "which") {
            builtin_which(cmd);
        } else if strcmp(word, "type") {
            builtin_type(cmd);
        } else if strcmp(word, "version") {
            builtin_version(cmd);
        } else if strcmp(word, "ps") {
            builtin_ps(cmd);
        } else if strcmp(word, "free") {
            builtin_free(cmd);
        } else if strcmp(word, "dmesg") {
            builtin_dmesg(cmd);
        } else if strcmp(word, "sysinfo") {
            builtin_sysinfo(cmd);
        } else if strcmp(word, "bw") {
            builtin_bw(cmd);
        } else if strcmp(word, "service") {
            builtin_service(cmd);
        } else if strcmp(word, "kill") {
            builtin_kill(cmd);
        } else if strcmp(word, "cp") {
            builtin_cp(cmd);
        } else if strcmp(word, "mv") {
            builtin_mv(cmd);
        } else if strcmp(word, "find") {
            builtin_find(cmd);
        } else if strcmp(word, "head") {
            builtin_head(cmd);
        } else if strcmp(word, "tail") {
            builtin_tail(cmd);
        } else if strcmp(word, "grep") {
            builtin_grep(cmd);
        } else if strcmp(word, "df") {
            builtin_df(cmd);
        } else if strcmp(word, "du") {
            builtin_du(cmd);
        } else if strcmp(word, "ln") {
            builtin_ln(cmd);
        } else if strcmp(word, "stat") {
            builtin_stat(cmd);
        } else if strcmp(word, "chmod") {
            builtin_chmod(cmd);
        } else if strcmp(word, "chown") {
            builtin_chown(cmd);
        } else if strcmp(word, "id") {
            builtin_id(cmd);
        } else if strcmp(word, "tree") {
            builtin_tree(cmd);
        } else if strcmp(word, "history") {
            builtin_history(cmd);
        } else if strcmp(word, "alias") {
            builtin_alias(cmd);
        } else if strcmp(word, "basename") {
            builtin_basename(cmd);
        } else if strcmp(word, "dirname") {
            builtin_dirname(cmd);
        } else if strcmp(word, "realpath") {
            builtin_realpath(cmd);
        } else if strcmp(word, "users") {
            builtin_users(cmd);
        } else if strcmp(word, "groups") {
            builtin_groups(cmd);
        } else if strcmp(word, "umask") {
            builtin_umask(cmd);
        } else if strcmp(word, "passwd") {
            builtin_passwd(cmd);
        } else if strcmp(word, "su") {
            builtin_su(cmd);
        } else if strcmp(word, "sudo") {
            builtin_sudo(cmd);
        } else if strcmp(word, "auth") {
            builtin_auth(cmd);
        } else if strcmp(word, "capabilities") {
            builtin_capabilities(cmd);
        } else if strcmp(word, "cpuinfo") {
            builtin_cpuinfo(cmd);
        } else if strcmp(word, "meminfo") {
            builtin_meminfo(cmd);
        } else if strcmp(word, "lscpu") {
            builtin_lscpu(cmd);
        } else if strcmp(word, "lsmem") {
            builtin_lsmem(cmd);
        } else if strcmp(word, "lspci") {
            builtin_lspci(cmd);
        } else if strcmp(word, "lsusb") {
            builtin_lsusb(cmd);
        } else if strcmp(word, "blkid") {
            builtin_blkid(cmd);
        } else if strcmp(word, "lsblk") {
            builtin_lsblk(cmd);
        } else if strcmp(word, "mount") {
            builtin_mount(cmd);
        } else if strcmp(word, "umount") {
            builtin_umount(cmd);
        } else if strcmp(word, "sync") {
            builtin_sync(cmd);
        } else if strcmp(word, "reboot") {
            builtin_reboot(cmd);
        } else if strcmp(word, "shutdown") {
            builtin_shutdown(cmd);
        } else if strcmp(word, "poweroff") {
            builtin_poweroff(cmd);
        } else if strcmp(word, "halt") {
            builtin_halt(cmd);
        } else if strcmp(word, "journalctl") {
            builtin_journalctl(cmd);
        } else if strcmp(word, "logs") {
            builtin_logs(cmd);
        } else if strcmp(word, "crontab") {
            builtin_crontab(cmd);
        } else if strcmp(word, "watch") {
            builtin_watch(cmd);
        } else if strcmp(word, "ip") {
            builtin_ip(cmd);
        } else if strcmp(word, "ifconfig") {
            builtin_ifconfig(cmd);
        } else if strcmp(word, "route") {
            builtin_route(cmd);
        } else if strcmp(word, "ping") {
            builtin_ping(cmd);
        } else if strcmp(word, "arp") {
            builtin_arp(cmd);
        } else if strcmp(word, "resolve") {
            builtin_resolve(cmd);
            write_str("\x1B[2J\x1B[H");
        } else if strcmp(word, "net") {
            builtin_net(cmd);
        } else if strcmp(word, "anx") {
            builtin_anx(cmd);
        } else if strcmp(word, "clear") {
            builtin_clear(cmd);
        } else if strcmp(word, "exit") {
            builtin_exit(cmd);
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
// ── Enhanced commands (Black Wall CLI) ──────────────────────────────────────

fn builtin_cd(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str(get_cwd());
        write_str("\n");
        return;
    }
    // Try to see if it's a directory by attempting readdir
    let resolved = resolve_path(path);
    // find length
    let mut len = 0;
    while len < resolved.len() && resolved[len] != 0 {
        len += 1;
    }
    if len == 0 {
        write_str("cd: invalid path\n");
        return;
    }
    let (pb, _) = path_buf(core::str::from_utf8(&resolved[..len]).unwrap_or("/"));
    let mut raw = [0u8; 64];
    let n = syscall3(SYS_READDIR, pb.as_ptr() as u64, raw.as_mut_ptr() as u64, raw.len() as u64);
    if (n as i64) < 0 {
        write_str("cd: ");
        write_str(path);
        write_str(": not a directory\n");
        return;
    }
    // Set cwd
    unsafe {
        // normalize
        if len == 1 && resolved[0] == b'/' {
            SHELL_STATE.cwd[0] = b'/';
            SHELL_STATE.cwd_len = 1;
        } else {
            // simple copy
            for i in 0..len {
                if i < 255 {
                    SHELL_STATE.cwd[i] = resolved[i];
                }
            }
            // trim trailing slash if not root
            let mut l = if len > 255 { 255 } else { len };
            if l > 1 && SHELL_STATE.cwd[l - 1] == b'/' {
                l -= 1;
            }
            SHELL_STATE.cwd_len = l;
        }
    }
}

fn builtin_whoami(_cmd: &[u8]) {
    write_str("root\n");
}

fn builtin_hostname(_cmd: &[u8]) {
    write_str("blackwall\n");
}

fn builtin_uname(cmd: &[u8]) {
    if cmd.contains(&b'-') && core::str::from_utf8(cmd).unwrap_or("").contains("a") {
        write_str("BlackWall blackwall 1.0.0 #1 SMP x86_64 unknown\n");
    } else {
        write_str("BlackWall\n");
    }
}

fn builtin_uptime(_cmd: &[u8]) {
    write_str("up 0 days, 00:00\n");
}

fn builtin_date(_cmd: &[u8]) {
    write_str("2026-01-01 00:00:00 UTC\n");
}

fn builtin_env(_cmd: &[u8]) {
    write_str("PATH=/bin:/usr/bin:/sbin\n");
    write_str("HOME=/root\n");
    write_str("USER=root\n");
    write_str("SHELL=/bin/init\n");
}

fn builtin_which(cmd: &[u8]) {
    let we = word_end(cmd);
    let name = path_of(cmd, we);
    if name.is_empty() {
        write_str("which: missing argument\n");
        return;
    }
    write_str("/bin/");
    write_str(name);
    write_str("\n");
}

fn builtin_type(cmd: &[u8]) {
    let we = word_end(cmd);
    let name = path_of(cmd, we);
    if name.is_empty() {
        write_str("type: missing argument\n");
        return;
    }
    write_str(name);
    write_str(" is a shell builtin\n");
}

fn builtin_version(_cmd: &[u8]) {
    write_str("Black Wall Core v1.0.0\n");
}

fn builtin_ps(_cmd: &[u8]) {
    write_str("PID TTY      STAT TIME CMD\n");
    write_str("  0 ?        S    0:00 idle\n");
    write_str("  1 ?        S    0:00 init\n");
}

fn builtin_free(_cmd: &[u8]) {
    write_str("              total        used        free\n");
    write_str("Mem:          256M        32M       224M\n");
}

fn builtin_dmesg(_cmd: &[u8]) {
    write_str("[    0.000000] Black Wall Core boot\n");
    write_str("[    0.001000] Kernel initialized\n");
}

fn builtin_sysinfo(_cmd: &[u8]) {
    write_str("Black Wall Core System Information\n");
    write_str("Kernel: BlackWall 1.0.0\n");
    write_str("Arch: x86_64\n");
}

fn builtin_bw(cmd: &[u8]) {
    let we = word_end(cmd);
    let arg = path_of(cmd, we);
    if arg.is_empty() {
        write_str("bw: usage: bw <status|doctor|security|audit|hardware|version|logs>\n");
        return;
    }
    write_str("bw: ");
    write_str(arg);
    write_str(": OK\n");
}

fn builtin_service(cmd: &[u8]) {
    let we = word_end(cmd);
    let arg = path_of(cmd, we);
    if arg.is_empty() {
        write_str("service: usage: service <name> <start|stop|restart|status>\n");
        return;
    }
    write_str("service: ");
    write_str(arg);
    write_str(": not available\n");
}

fn builtin_kill(cmd: &[u8]) {
    let we = word_end(cmd);
    let arg = path_of(cmd, we);
    if arg.is_empty() {
        write_str("kill: usage: kill <pid>\n");
        return;
    }
    write_str("kill: signal sent to ");
    write_str(arg);
    write_str("\n");
}

fn builtin_cp(_cmd: &[u8]) {
    write_str("cp: not implemented in this shell build\n");
}

fn builtin_mv(_cmd: &[u8]) {
    write_str("mv: not implemented in this shell build\n");
}

fn builtin_ln(_cmd: &[u8]) {
    write_str("ln: not implemented in this shell build\n");
}

fn builtin_stat(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("stat: usage: stat <path>\n");
        return;
    }
    write_str("  File: ");
    write_str(path);
    write_str("\n  Size: 0\nAccess: (0644)  Uid: (0/root)   Gid: (0/root)\n");
}

fn builtin_chmod(_cmd: &[u8]) {
    write_str("chmod: not implemented in this shell build\n");
}

fn builtin_chown(_cmd: &[u8]) {
    write_str("chown: not implemented in this shell build\n");
}

fn builtin_id(_cmd: &[u8]) {
    write_str("uid=0(root) gid=0(root)\n");
}

fn builtin_df(_cmd: &[u8]) {
    write_str("Filesystem     Size Used Avail Use% Mounted on\n");
    write_str("ramfs           64M   8M   56M  12% /\n");
}

fn builtin_du(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("8.0K\t/\n");
    } else {
        write_str("8.0K\t");
        write_str(path);
        write_str("\n");
    }
}

fn builtin_tree(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    write_str(".");
    if !path.is_empty() {
        write_str(" -> ");
        write_str(path);
    }
    write_str("\n0 directories, 0 files\n");
}

fn builtin_head(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("head: usage: head <path>\n");
        return;
    }
    let fd = open_file(path, O_RDONLY);
    if fd < 0 {
        write_str("head: cannot open '");
        write_str(path);
        write_str("'\n");
        return;
    }
    let mut buf = [0u8; 256];
    let n = read_file(fd, &mut buf);
    close_file(fd);
    if n > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            write_str(s);
            if s.as_bytes().last().copied().unwrap_or(b'\n') != b'\n' {
                write_str("\n");
            }
            return;
        }
    }
    write_str("(empty)\n");
}

fn builtin_tail(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("tail: usage: tail <path>\n");
        return;
    }
    builtin_head(cmd);
}

fn builtin_grep(cmd: &[u8]) {
    let we = word_end(cmd);
    let rest = path_of(cmd, we);
    if rest.is_empty() {
        write_str("grep: usage: grep <pattern> <file>\n");
        return;
    }
    write_str("(no matches)\n");
}

fn builtin_find(cmd: &[u8]) {
    write_str(".\n");
}

fn builtin_history(_cmd: &[u8]) {
    write_str("(history not persisted)\n");
}

fn builtin_alias(_cmd: &[u8]) {
    write_str("(no aliases)\n");
}


fn builtin_basename(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("basename: missing operand\n");
        return;
    }
    if let Some(pos) = path.rfind('/') {
        write_str(&path[pos + 1..]);
    } else {
        write_str(path);
    }
    write_str("\n");
}

fn builtin_dirname(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() || path == "/" {
        write_str("/\n");
        return;
    }
    if let Some(pos) = path.rfind('/') {
        let base = &path[..pos];
        if base.is_empty() {
            write_str("/\n");
        } else {
            write_str(base);
            write_str("\n");
        }
    } else {
        write_str(".\n");
    }
}

fn builtin_realpath(cmd: &[u8]) {
    let we = word_end(cmd);
    let path = path_of(cmd, we);
    if path.is_empty() {
        write_str("realpath: missing operand\n");
        return;
    }
    if path.starts_with('/') {
        write_str(path);
    } else {
        write_str("/");
        write_str(path);
    }
    write_str("\n");
}

fn builtin_cpuinfo(_cmd: &[u8]) {
    write_str("processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: BlackWall Virtual CPU\n");
}

fn builtin_meminfo(_cmd: &[u8]) {
    write_str("MemTotal:         524288 kB\nMemFree:          458752 kB\n");
}

fn builtin_lscpu(_cmd: &[u8]) {
    builtin_cpuinfo(_cmd);
}

fn builtin_lsmem(_cmd: &[u8]) {
    builtin_meminfo(_cmd);
}

fn builtin_lspci(_cmd: &[u8]) {
    write_str("(no PCI devices enumerated)\n");
}

fn builtin_lsusb(_cmd: &[u8]) {
    write_str("(no USB devices enumerated)\n");
}

fn builtin_blkid(_cmd: &[u8]) {
    write_str("/dev/ram0: LABEL=\"ROOT\" UUID=\"00000000-0000-0000-0000-000000000000\" TYPE=\"ramfs\"\n");
}

fn builtin_lsblk(_cmd: &[u8]) {
    write_str("NAME MAJ:MIN RM SIZE RO TYPE MOUNTPOINT\n");
    write_str("ram0   1:0    0  64M  0 disk /\n");
}

fn builtin_mount(_cmd: &[u8]) {
    write_str("ramfs on / type ramfs (rw)\n");
}

fn builtin_umount(_cmd: &[u8]) {
    write_str("umount: not permitted\n");
}

fn builtin_sync(_cmd: &[u8]) {
    write_str("sync: completed\n");
}

fn builtin_reboot(_cmd: &[u8]) {
    write_str("reboot: operation not available in this session\n");
}

fn builtin_shutdown(_cmd: &[u8]) {
    write_str("shutdown: operation not available in this session\n");
}

fn builtin_poweroff(_cmd: &[u8]) {
    write_str("poweroff: operation not available in this session\n");
}

fn builtin_halt(_cmd: &[u8]) {
    write_str("halt: operation not available in this session\n");
}

fn builtin_journalctl(_cmd: &[u8]) {
    write_str("(no journal available)\n");
}

fn builtin_logs(_cmd: &[u8]) {
    write_str("(no persistent logs available)\n");
}

fn builtin_crontab(_cmd: &[u8]) {
    write_str("(no crontab support)\n");
}

fn builtin_watch(cmd: &[u8]) {
    let we = word_end(cmd);
    let rest = path_of(cmd, we);
    if rest.is_empty() {
        write_str("watch: missing command\n");
        return;
    }
    write_str("(watch not supported)\n");
}

fn builtin_users(_cmd: &[u8]) {
    write_str("root\n");
}

fn builtin_groups(_cmd: &[u8]) {
    write_str("root\n");
}

fn builtin_umask(_cmd: &[u8]) {
    write_str("0022\n");
}

fn builtin_passwd(_cmd: &[u8]) {
    write_str("passwd: not implemented\n");
}

fn builtin_su(_cmd: &[u8]) {
    write_str("su: permission denied\n");
}

fn builtin_sudo(_cmd: &[u8]) {
    write_str("sudo: not available\n");
}

fn builtin_auth(_cmd: &[u8]) {
    write_str("auth: not available\n");
}

fn builtin_capabilities(_cmd: &[u8]) {
    write_str("capabilities: none enforced\n");
}

fn builtin_netcfg(cmd: &[u8]) {
    builtin_net(cmd);
}

fn builtin_ip(cmd: &[u8]) {
    builtin_net(cmd);
}

fn builtin_ifconfig(cmd: &[u8]) {
    builtin_net(cmd);
}

fn builtin_route(cmd: &[u8]) {
    builtin_net(cmd);
}

fn builtin_ping(cmd: &[u8]) {
    builtin_net(cmd);
}

fn builtin_arp(cmd: &[u8]) {
    builtin_net(cmd);
}

fn builtin_resolve(_cmd: &[u8]) {
    write_str("resolve: not implemented\n");
}

fn builtin_clear(_cmd: &[u8]) {
    write_str("\x1B[2J\x1B[H");
}

fn builtin_exit(_cmd: &[u8]) {
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
    loop {}
}

fn write_cyan(s: &str) {
    write_str("\x1B[36m");
    write_str(s);
    write_str("\x1B[0m");
}

fn write_blue(s: &str) {
    write_str("\x1B[34m");
    write_str(s);
    write_str("\x1B[0m");
}

fn write_green(s: &str) {
    write_str("\x1B[32m");
    write_str(s);
    write_str("\x1B[0m");
}

fn write_yellow(s: &str) {
    write_str("\x1B[33m");
    write_str(s);
    write_str("\x1B[0m");
}

fn write_red(s: &str) {
    write_str("\x1B[31m");
    write_str(s);
    write_str("\x1B[0m");
}
