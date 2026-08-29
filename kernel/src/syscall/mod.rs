use crate::arch::x86_64::syscall::SyscallRegs;
use blackwall_shared::syscall::*;

pub mod fs;
pub mod process;

#[no_mangle]
pub extern "C" fn blackwall_syscall_rust(regs: *mut SyscallRegs) -> u64 {
    let regs = unsafe { &mut *regs };
    let syscall_num = regs.rax;

    match syscall_num {
        SYS_READ => fs::sys_read(regs.rdi, regs.rsi, regs.rdx),
        SYS_WRITE => fs::sys_write(regs.rdi, regs.rsi, regs.rdx),
        SYS_OPEN => fs::sys_open(regs.rdi, regs.rsi),
        SYS_CLOSE => fs::sys_close(regs.rdi),
        SYS_FORK => process::sys_fork(regs),
        SYS_EXECVE => process::sys_execve(regs.rdi, regs.rsi, regs.rdx),
        SYS_EXIT => process::sys_exit(regs.rdi),
        _ => ENOSYS as u64,
    }
}
