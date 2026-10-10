//! `syscall` / `sysret` style entry for 64-bit user programs.
//!
//! The `syscall` instruction itself is set up here (EFER.SCE, STAR, LSTAR,
//! FMASK).  The actual entry/exit code lives in [`crate::arch::x86_64::trap`]
//! so that syscalls share the exact same frame layout as interrupts — that is
//! what allows a syscall to block or be preempted like any other context.

use crate::arch::x86_64::serial;
use crate::arch::x86_64::trap::blackwall_syscall_entry;
use core::arch::asm;

const MSR_EFER: u32 = 0xC0000080;
const MSR_STAR: u32 = 0xC0000081;
const MSR_LSTAR: u32 = 0xC0000082;
const MSR_FMASK: u32 = 0xC0000084;

/// Per-task kernel stack top used by the `syscall` entry stub.
///
/// Written by the scheduler on every context switch while interrupts are
/// disabled, so a single global is safe on a uniprocessor kernel.
#[no_mangle]
pub static mut SYSCALL_KERNEL_STACK: u64 = 0;

/// Scratch slot used for a handful of instructions while the entry stub moves
/// from the user stack to the kernel stack.  Interrupts are masked by FMASK
/// for the whole sequence, so no trap can observe an intermediate value.
#[no_mangle]
pub static mut SYSCALL_SCRATCH: u64 = 0;

pub fn init() {
    serial::line("[DEBUG] Entering syscall::init()...");
    unsafe {
        // Enable SCE (System Call Enable) in EFER (bit 0)
        let mut efer_low: u32;
        let mut efer_high: u32;
        asm!("rdmsr", in("ecx") MSR_EFER, out("eax") efer_low, out("edx") efer_high, options(nomem, nostack));
        efer_low |= 1;
        asm!("wrmsr", in("ecx") MSR_EFER, in("eax") efer_low, in("edx") efer_high, options(nostack));

        // STAR[47:32] = syscall CS/SS base = 0x08 (kernel code)
        // STAR[63:48] = sysret  CS/SS base = 0x10 (kernel data)
        //   sysret CS = 0x10 + 16 = 0x20 (| 3 → 0x23, user code)
        //   sysret SS = 0x10 + 8  = 0x18 (| 3 → 0x1b, user data)
        // The kernel returns to ring 3 through `iretq` (the frame carries CS
        // and SS), so STAR only has to satisfy the CPU's `syscall` checks.
        let star_high: u32 = (0x10 << 16) | 0x08;
        let star_low: u32 = 0;
        asm!("wrmsr", in("ecx") MSR_STAR, in("eax") star_low, in("edx") star_high, options(nostack));

        // LSTAR — where `syscall` lands.
        let lstar = blackwall_syscall_entry as *const () as u64;
        let lstar_low = lstar as u32;
        let lstar_high = (lstar >> 32) as u32;
        asm!("wrmsr", in("ecx") MSR_LSTAR, in("eax") lstar_low, in("edx") lstar_high, options(nostack));

        // FMASK — clear IF (bit 9) and DF (bit 10) on syscall entry so the
        // kernel side runs with interrupts disabled until it iretq's back.
        let fmask_low: u32 = 0x200 | 0x400;
        let fmask_high: u32 = 0;
        asm!("wrmsr", in("ecx") MSR_FMASK, in("eax") fmask_low, in("edx") fmask_high, options(nostack));
    }
    serial::line("Syscall Interface Initialized");
}
