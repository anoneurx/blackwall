#![allow(bad_asm_style)]
use crate::arch::x86_64::serial;
use core::arch::{asm, global_asm};

const MSR_EFER: u32 = 0xC0000080;
const MSR_STAR: u32 = 0xC0000081;
const MSR_LSTAR: u32 = 0xC0000082;
const MSR_FMASK: u32 = 0xC0000084;

#[no_mangle]
pub static mut SYSCALL_KERNEL_STACK: u64 = 0;
#[no_mangle]
pub static mut SYSCALL_USER_STACK: u64 = 0;

global_asm!(
    r#"
    .intel_syntax noprefix

    .global blackwall_syscall_handler
blackwall_syscall_handler:
    // 1. Save user stack pointer and load kernel stack
    mov [rip + SYSCALL_USER_STACK], rsp
    mov rsp, [rip + SYSCALL_KERNEL_STACK]

    // 2. Save registers (System V ABI syscall args: rdi, rsi, rdx, r10, r8, r9)
    // rcx contains user rip, r11 contains user rflags
    push rcx
    push r11
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9

    // 3. Call Rust dispatcher
    // rax contains the syscall number. We pass it as the first argument (rdi),
    // and shift the other arguments.
    // Wait, let's just pass arguments as they are, but Rust needs them in:
    // rdi, rsi, rdx, rcx, r8, r9.
    // Original: rax (num), rdi (arg1), rsi (arg2), rdx (arg3), r10 (arg4), r8 (arg5), r9 (arg6)
    // We will set: rdi=rax, rsi=rdi, rdx=rsi, rcx=rdx, r8=r10, r9=r8, stack=r9
    // Actually, it's easier to just push them to the stack and pass a pointer to a struct,
    // or just move them around carefully.
    
    // Let's pass a pointer to a struct containing the saved registers!
    // push all GP registers
    push rax
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    
    // Pass rsp (which points to the saved registers) as first argument
    mov rdi, rsp
    
    call blackwall_syscall_rust

    // pop all GP registers
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    pop rax
    
    // pop syscall args
    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    pop r11
    pop rcx

    // Restore user stack
    mov rsp, [rip + SYSCALL_USER_STACK]
    
    // Return to userspace
    sysretq
"#
);

#[allow(dead_code)]
extern "C" {
    fn blackwall_syscall_handler();
    fn blackwall_syscall_rust(regs: *mut SyscallRegs) -> u64;
}

#[repr(C)]
pub struct SyscallRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rax: u64,
    pub r9: u64,
    pub r8: u64,
    pub r10: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r11: u64, // rflags
    pub rcx: u64, // rip
}

pub fn init() {
    serial::line("[DEBUG] Entering syscall::init()...");
    unsafe {
        // Enable SCE (System Call Enable) in EFER (bit 0)
        let mut efer_low: u32;
        let mut efer_high: u32;
        asm!("rdmsr", in("ecx") MSR_EFER, out("eax") efer_low, out("edx") efer_high, options(nomem, nostack));
        efer_low |= 1;
        asm!("wrmsr", in("ecx") MSR_EFER, in("eax") efer_low, in("edx") efer_high, options(nostack));

        // STAR
        // STAR[63:48] = sysret CS/SS base = 0x10 (Kernel Data)
        // SYSRET CS = 0x10 + 16 = 0x20 (User Code)
        // SYSRET SS = 0x10 + 8 = 0x18 (User Data)
        // STAR[47:32] = syscall CS/SS base = 0x08 (Kernel Code)
        let star_high: u32 = (0x10 << 16) | 0x08;
        let star_low: u32 = 0;
        asm!("wrmsr", in("ecx") MSR_STAR, in("eax") star_low, in("edx") star_high, options(nostack));

        // LSTAR
        let lstar = blackwall_syscall_handler as *const () as u64;
        let lstar_low = lstar as u32;
        let lstar_high = (lstar >> 32) as u32;
        asm!("wrmsr", in("ecx") MSR_LSTAR, in("eax") lstar_low, in("edx") lstar_high, options(nostack));

        // FMASK
        // Mask Interrupt Flag (bit 9) and Direction Flag (bit 10) on syscall entry.
        // This disables interrupts when we enter the syscall handler.
        let fmask_low: u32 = 0x200 | 0x400;
        let fmask_high: u32 = 0;
        asm!("wrmsr", in("ecx") MSR_FMASK, in("eax") fmask_low, in("edx") fmask_high, options(nostack));
    }
    serial::line("Syscall Interface Initialized");
}
