//! Unified trap entry/exit path.
//!
//! Every way into the kernel — hardware interrupts, CPU exceptions, the
//! `syscall` instruction and voluntary `yield`s — normalises onto one stack
//! frame layout ([`IrqFrame`]) and one epilogue (restore + `iretq`).  This
//! makes preemption work identically for ring-0 kernel threads and ring-3
//! user processes: a context switch is nothing more than swapping the frame
//! pointer the epilogue restores from.
//!
//! Frame layout (low address → high address, 168 bytes):
//!
//! ```text
//! +0    r15   +64 rdi  +120 err   +152 rsp
//! +8    r14   +72 rsi  +128 rip   +160 ss
//! +16   r13   +80 rbp  +136 cs
//! +24   r12   +88 rbx  +144 rflags
//! +32   r11   +96 rdx
//! +40   r10   +104 rcx
//! +48   r9    +112 rax
//! +56   r8
//! ```
//!
//! The `err` slot packs the exception status as `(error_code << 16) | vector`
//! so the Rust dispatcher can tell every vector apart without extra state.

use core::arch::{asm, global_asm};

/// Size of a saved trap frame in bytes.
pub const FRAME_SIZE: usize = 21 * 8;

/// Canonical kernel code / data selectors (see `gdt.rs`).
pub const KERNEL_CODE_SEL: u64 = 0x08;
pub const KERNEL_DATA_SEL: u64 = 0x10;
/// Canonical user code / data selectors (RPL 3).
pub const USER_CODE_SEL: u64 = 0x23;
pub const USER_DATA_SEL: u64 = 0x1B;

/// Interrupt flag (RFLAGS bit 9).
pub const RFLAGS_IF: u64 = 1 << 9;
/// Reserved bit that must always be set in RFLAGS.
pub const RFLAGS_RESERVED: u64 = 1 << 1;

/// The single register-save layout shared by every kernel entry point.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IrqFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    /// Packed exception status: `(error_code << 16) | vector`.  Zero for
    /// interrupts and software entries.
    pub err: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl IrqFrame {
    /// # Safety
    /// `ptr` must point at a live, fully-formed [`IrqFrame`].
    pub unsafe fn from_ptr<'a>(ptr: *mut IrqFrame) -> &'a mut IrqFrame {
        &mut *ptr
    }

    /// True when the frame describes ring-3 execution.
    pub fn from_user(&self) -> bool {
        self.cs & 3 == 3
    }

    /// Exception vector this frame was raised with (0 for IRQs / syscalls).
    pub fn vector(&self) -> u64 {
        self.err & 0xffff
    }

    /// CPU error code (0 when the vector pushes none).
    pub fn error_code(&self) -> u64 {
        self.err >> 16
    }

    /// Install the value a syscall should return to userspace.
    pub fn set_return(&mut self, value: u64) {
        self.rax = value;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Assembly entry points
// ─────────────────────────────────────────────────────────────────────────────

global_asm!(
    r#"
    // Save every general purpose register into an [IrqFrame] and prepare the
    // stack for a System V call.  On entry the hardware/software frame
    // (err, rip, cs, rflags, rsp, ss) must already be pushed.
    .macro SAVE_REGS
        push rax
        push rcx
        push rdx
        push rbx
        push rbp
        push rsi
        push rdi
        push r8
        push r9
        push r10
        push r11
        push r12
        push r13
        push r14
        push r15
        mov rbx, rsp
        and rsp, -16
        // The UEFI target uses the Win64 ABI, which requires 32 bytes of
        // shadow space below the return address for every call.
        sub rsp, 32
    .endm

    // Restore from the frame returned by the Rust dispatcher (rax == 0 means
    // "keep running the frame we came in on", kept in callee-saved rbx).
    .macro RESTORE_REGS
        test rax, rax
        jnz 9001f
        mov rax, rbx
    9001:
        mov rsp, rax
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8
        pop rdi
        pop rsi
        pop rbp
        pop rbx
        pop rdx
        pop rcx
        pop rax
        add rsp, 8
        iretq
    .endm

    // Exception stubs.  Those whose vector pushes an error code merge it with
    // the vector number: `(error_code << 16) | vector`.
    .macro EXC_ERR vec
        push rax
        mov rax, [rsp + 8]
        shl rax, 16
        or rax, \vec
        mov [rsp + 8], rax
        pop rax
        SAVE_REGS
        mov rcx, rbx
        call blackwall_trap_exception_rust
        RESTORE_REGS
    .endm

    .macro EXC_NOERR vec
        push \vec
        SAVE_REGS
        mov rcx, rbx
        call blackwall_trap_exception_rust
        RESTORE_REGS
    .endm

    // ── Exceptions without an error code ─────────────────────────────────
    .global blackwall_trap_exc_de
blackwall_trap_exc_de:              // #DE divide error (vector 0)
    EXC_NOERR 0

    .global blackwall_trap_exc_ud
blackwall_trap_exc_ud:              // #UD invalid opcode (vector 6)
    EXC_NOERR 6

    // ── Exceptions with an error code ────────────────────────────────────
    .global blackwall_trap_exc_df
blackwall_trap_exc_df:              // #DF double fault (vector 8)
    EXC_ERR 8

    .global blackwall_trap_exc_gp
blackwall_trap_exc_gp:              // #GP general protection (vector 13)
    EXC_ERR 13

    .global blackwall_trap_exc_pf
blackwall_trap_exc_pf:              // #PF page fault (vector 14)
    EXC_ERR 14

    .global blackwall_trap_exc_ac
blackwall_trap_exc_ac:              // #AC alignment check (vector 17)
    EXC_ERR 17

    // ── IRQ0 (PIT) — no error code ───────────────────────────────────────
    .global blackwall_trap_timer
blackwall_trap_timer:
    push 0
    SAVE_REGS
    mov rcx, rbx
    call blackwall_trap_timer_rust
    RESTORE_REGS

    // ── IRQ1 (PS/2 keyboard) — no error code ─────────────────────────────
    .global blackwall_trap_keyboard
blackwall_trap_keyboard:
    push 0
    SAVE_REGS
    mov rcx, rbx
    call blackwall_trap_keyboard_rust
    RESTORE_REGS

    // ── Local APIC spurious interrupt (vector 0xFF) ──────────────────────
    // The LAPIC delivers this whenever a pending IRQ was too low in priority
    // to be handled; it is NOT an actual interrupt, must receive no EOI and
    // is simply iretq'd back.  The IDT gate for it MUST exist, otherwise the
    // delivery raises a #GP against the missing gate.
    .global blackwall_trap_spurious
blackwall_trap_spurious:
    iretq

    // ── Voluntary context switch from ring 0 ─────────────────────────────
    // In long mode `IRETQ` always pops RSP+SS (mirroring `do_interrupt64`,
    // which always pushes them), so a ring-0 yield must present the full
    // 6-word trailing frame.  A truncated 4-word frame makes `iretq` load a
    // garbage RSP/SS and fault.  We stash the stub's own RSP in the RSP slot
    // and resume at `.Lyield_resume`, whose `ret` completes the original
    // `call blackwall_trap_yield` as if it had returned normally.
    .global blackwall_trap_yield
blackwall_trap_yield:
    mov r11, rsp                    // r11 = R; [R] is our return address
    push 0x10                       // SS  (kernel data)
    push r11                        // RSP slot = R
    pushfq                          // RFLAGS
    push 0x08                       // CS  (kernel code)
    lea rax, [rip + .Lyield_resume]
    push rax                        // RIP
    push 0                          // dummy error code
    SAVE_REGS
    mov rcx, rbx
    call blackwall_trap_yield_rust
    RESTORE_REGS
.Lyield_resume:
    ret

    // ── `syscall` entry ──────────────────────────────────────────────────
    // RCX = user RIP, R11 = user RFLAGS, RSP = user stack pointer.
    // The frame is built on the *per task* kernel stack so a task blocked
    // inside a syscall keeps its own private stack while switched out.
    .global blackwall_syscall_entry
blackwall_syscall_entry:
    mov [rip + SYSCALL_SCRATCH], rsp
    mov rsp, [rip + SYSCALL_KERNEL_STACK]
    test rsp, rsp
    jnz 9002f
        // No kernel stack installed for this task — spin rather than run on
        // the user stack (this indicates a scheduler setup bug).
        cli
    9003:
        hlt
        jmp 9003b
    9002:
    push 0x1b                       // SS
    push qword ptr [rip + SYSCALL_SCRATCH]  // user RSP
    push r11                        // user RFLAGS
    push 0x23                       // CS
    push rcx                        // RIP
    push 0                          // dummy error code
    SAVE_REGS
    mov rcx, rbx
    call blackwall_syscall_rust
    RESTORE_REGS
"#
);

extern "C" {
    /// Per-vector exception stubs installed into the IDT.
    pub fn blackwall_trap_exc_de();
    pub fn blackwall_trap_exc_ud();
    pub fn blackwall_trap_exc_df();
    pub fn blackwall_trap_exc_gp();
    pub fn blackwall_trap_exc_pf();
    pub fn blackwall_trap_exc_ac();
    /// Hardware IRQ stubs installed into the IDT.
    pub fn blackwall_trap_timer();
    pub fn blackwall_trap_keyboard();
    /// Local APIC spurious interrupt (vector 0xFF).
    pub fn blackwall_trap_spurious();
    /// Voluntarily give the CPU up; called from ring 0 like a normal function.
    pub fn blackwall_trap_yield();
    /// `syscall` instruction target (installed into the LSTAR MSR).
    pub fn blackwall_syscall_entry();
}

// ─────────────────────────────────────────────────────────────────────────────
// Rust-side dispatchers
// ─────────────────────────────────────────────────────────────────────────────

/// Hardware timer tick: account for elapsed time and possibly reschedule.
///
/// Returns the frame to resume from — `null` keeps the interrupted context.
#[no_mangle]
pub extern "C" fn blackwall_trap_timer_rust(frame: *mut IrqFrame) -> *mut IrqFrame {
    let ticks = crate::arch::x86_64::timer::on_tick();
    // `try_lock`: if ordinary kernel code sits in a scheduler critical section
    // we simply skip this quantum instead of deadlocking.
    let mut guard = match crate::scheduler::SCHEDULER.try_lock() {
        Some(g) => g,
        None => return core::ptr::null_mut(),
    };
    match guard.as_mut() {
        Some(sched) => sched.on_timer_tick(ticks, frame),
        None => core::ptr::null_mut(),
    }
}

/// PS/2 keyboard IRQ: drain the controller into the input buffer.
#[no_mangle]
pub extern "C" fn blackwall_trap_keyboard_rust(frame: *mut IrqFrame) -> *mut IrqFrame {
    crate::drivers::ps2::handle_irq();
    // The keyboard never preempts: the interrupted frame is resumed as-is.
    let _ = frame;
    core::ptr::null_mut()
}

/// `syscall` instruction dispatcher.
///
/// The entry stub in the assembly above already built a full [`IrqFrame`]
/// (including the user `rip`/`rsp`/`rflags`), so a syscall is dispatched and
/// resumed through exactly the same mechanism as an interrupt — including the
/// ability to block or be preempted.
#[no_mangle]
pub extern "C" fn blackwall_syscall_rust(frame: *mut IrqFrame) -> *mut IrqFrame {
    crate::syscall::dispatch(frame)
}

/// CPU exception.  Kills the offending user task, or halts the machine for a
/// kernel-mode fault.
#[no_mangle]
pub extern "C" fn blackwall_trap_exception_rust(frame: *mut IrqFrame) -> *mut IrqFrame {
    let (vector, error, rip, rsp, rflags) = {
        let f = unsafe { IrqFrame::from_ptr(frame) };
        (f.vector(), f.error_code(), f.rip, f.rsp, f.rflags)
    };

    let from_user = unsafe { (*frame).cs & 3 == 3 };

    if from_user {
        let pid = crate::scheduler::current_pid();
        crate::logging::print(format_args!(
            "[FAULT] PID {} vector={} err={:#x} rip={:#x} rsp={:#x} cr2={:#x} — killing task\n",
            pid,
            vector,
            error,
            rip,
            rsp,
            read_cr2(),
        ));
        // Never resume a task that faulted: terminate it and switch away.
        return unsafe { kill_current_task(frame) };
    }

    crate::logging::print(format_args!(
        "Kernel exception: vector={} error_code={:#x}\n  rip={:#x} cs={:#x} rsp={:#x} rflags={:#x}\n",
        vector, error, rip, unsafe { (*frame).cs }, rsp, rflags,
    ));
    if vector == 14 {
        crate::logging::print(format_args!("  cr2={:#x}\n", read_cr2()));
    }
    crate::arch::x86_64::interrupts::halt_forever()
}

/// Voluntary yield from ring 0.
#[no_mangle]
pub extern "C" fn blackwall_trap_yield_rust(frame: *mut IrqFrame) -> *mut IrqFrame {
    reschedule_current(frame, true)
}

/// Ask the scheduler to consider a context switch for `frame`.
///
/// Returns the frame to resume (may be a different task), or `null` to keep
/// running `frame`.
pub fn reschedule_current(frame: *mut IrqFrame, force: bool) -> *mut IrqFrame {
    let mut guard = match crate::scheduler::SCHEDULER.try_lock() {
        Some(g) => g,
        None => return core::ptr::null_mut(),
    };
    match guard.as_mut() {
        Some(sched) => sched.reschedule(frame, force),
        None => core::ptr::null_mut(),
    }
}

/// Terminate the currently running task and switch to the next one.
///
/// # Safety
/// `frame` must be the live trap frame of the task being killed.
pub unsafe fn kill_current_task(frame: *mut IrqFrame) -> *mut IrqFrame {
    let mut guard = crate::scheduler::SCHEDULER.lock();
    match guard.as_mut() {
        Some(sched) => {
            let pid = sched.current_pid;
            sched.terminate_task(pid);
            sched.reschedule(frame, true)
        }
        None => core::ptr::null_mut(),
    }
}

/// Stop running until the scheduler wakes this task (sleep, blocking read).
///
/// The caller must already have marked its PCB as `Sleeping`/`Blocked` and
/// removed it from the run queue.
///
/// # Safety
/// `frame` must be the live trap frame of the current task.
pub unsafe fn block_current(frame: *mut IrqFrame) -> *mut IrqFrame {
    let mut guard = crate::scheduler::SCHEDULER.lock();
    match guard.as_mut() {
        Some(sched) => sched.reschedule(frame, true),
        None => core::ptr::null_mut(),
    }
}

/// Switch the CPU to a new page table.
///
/// # Safety
/// `cr3` must be the physical address of a valid PML4.
pub unsafe fn switch_cr3(cr3: u64) {
    if cr3 == 0 {
        return;
    }
    if current_cr3() != cr3 {
        asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags));
    }
}

/// Read the physical address of the active PML4.
pub fn current_cr3() -> u64 {
    let cr3: u64;
    unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags)) };
    cr3 & !0xfff
}

/// Read the faulting address from CR2.
pub fn read_cr2() -> u64 {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags)) };
    cr2
}

/// Issue a voluntary context switch (ring 0 only).
pub fn yield_now() {
    unsafe { blackwall_trap_yield() }
}
