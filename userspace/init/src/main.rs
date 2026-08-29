#![no_std]
#![no_main]

use blackwall_shared::syscall::*;
use core::arch::asm;
use core::panic::PanicInfo;

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

fn open_file(path: &str) -> i64 {
    let mut path_buf = [0u8; 128];
    let copy_len = path.len().min(127);
    path_buf[..copy_len].copy_from_slice(&path.as_bytes()[..copy_len]);
    path_buf[copy_len] = 0; // null terminator
    
    let fd: u64;
    unsafe {
        asm!(
            "syscall",
            in("rax") SYS_OPEN,
            in("rdi") path_buf.as_ptr() as u64,
            in("rsi") 0, // flags
            out("rcx") _,
            out("r11") _,
            lateout("rax") fd,
            options(nostack)
        );
    }
    fd as i64
}

fn close_file(fd: i64) {
    unsafe {
        asm!(
            "syscall",
            in("rax") SYS_CLOSE,
            in("rdi") fd as u64,
            out("rcx") _,
            out("r11") _,
            options(nostack)
        );
    }
}

fn read_file(fd: i64, buf: &mut [u8]) -> i64 {
    let bytes_read: u64;
    unsafe {
        asm!(
            "syscall",
            in("rax") SYS_READ,
            in("rdi") fd as u64,
            in("rsi") buf.as_mut_ptr() as u64,
            in("rdx") buf.len() as u64,
            out("rcx") _,
            out("r11") _,
            lateout("rax") bytes_read,
            options(nostack)
        );
    }
    bytes_read as i64
}

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

fn starts_with(s1: &[u8], prefix: &str) -> bool {
    if s1.len() < prefix.len() {
        return false;
    }
    for i in 0..prefix.len() {
        if s1[i] != prefix.as_bytes()[i] {
            return false;
        }
    }
    true
}

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

        if strcmp(cmd, "help") {
            write_str("Available commands:\n");
            write_str("  help          Show this message\n");
            write_str("  ls            List files in the VFS root and mountpoints\n");
            write_str("  cat <file>    Display file contents (e.g. /mnt/hello.txt)\n");
            write_str("  clear         Clear the terminal screen\n");
            write_str("  exit          Terminate userspace shell\n");
        } else if strcmp(cmd, "ls") {
            write_str("Filesystem hierarchy:\n");
            write_str("  / (RamFs):\n");
            write_str("    bin/\n");
            write_str("      init (ELF executable)\n");
            write_str("    mnt/ (mount point)\n");
            write_str("  /mnt (Ext2):\n");
            write_str("    hello.txt (text file)\n");
        } else if strcmp(cmd, "clear") {
            write_str("\x1B[2J\x1B[H");
        } else if starts_with(cmd, "cat ") {
            let path_bytes = &cmd[4..];
            if let Ok(path) = core::str::from_utf8(path_bytes) {
                let fd = open_file(path);
                if fd < 0 || fd == u64::MAX as i64 {
                    write_str("cat: file not found or permission denied\n");
                } else {
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
            }
        } else if strcmp(cmd, "exit") {
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
