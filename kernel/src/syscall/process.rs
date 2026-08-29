use crate::arch::x86_64::syscall::SyscallRegs;
use crate::scheduler::SCHEDULER;
use blackwall_shared::syscall::*;

pub fn sys_fork(_regs: &SyscallRegs) -> u64 {
    // In a real fork, we'd clone the address space, PCB, and registers.
    // For Phase 4, we just return ENOSYS since we don't have true userspace isolation yet.
    ENOSYS as u64
}

pub fn sys_execve(_path: u64, _argv: u64, _envp: u64) -> u64 {
    // Needs ELF loader and VFS.
    ENOSYS as u64
}

pub fn sys_exit(status: u64) -> u64 {
    crate::arch::x86_64::interrupts::disable();
    if let Some(mut sched) = SCHEDULER.try_lock() {
        if let Some(ref mut s) = *sched {
            let pid = s.current_pid;
            crate::arch::x86_64::serial::line(&alloc::format!(
                "Task {} exited with status {}",
                pid,
                status
            ));
            s.terminate_task(pid);
            s.schedule();
        }
    }

    // Should never return here.
    loop {
        crate::arch::x86_64::interrupts::halt_forever();
    }
}
