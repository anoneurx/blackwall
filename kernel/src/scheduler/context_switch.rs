use core::arch::{asm, global_asm};

// ---------------------------------------------------------------------------
// Context-switch trampoline (assembly)
// ---------------------------------------------------------------------------
//
// The x86_64 System V ABI designates the following as CALLEE-SAVED:
//   rbx, rbp, r12, r13, r14, r15
// All other registers are caller-saved and will be clobbered by any normal
// function call, so we only need to preserve the callee-saved set plus rflags.
//
// Stack layout after `switch_to` pushes state onto *old* stack:
//
//   [rsp+48]  rflags
//   [rsp+40]  rbp
//   [rsp+32]  rbx
//   [rsp+24]  r15
//   [rsp+16]  r14
//   [rsp+ 8]  r13
//   [rsp+ 0]  r12
//
// The return address (pushed by the `call` instruction that invoked
// switch_to) sits above rflags at [rsp+56] before we push anything.
// After saving and swapping rsp, we pop the new task's saved state in
// reverse order and `ret`, which continues execution in the new task at
// whatever address was on top of *its* stack.
//
// Note: SSE/MMX are disabled for this kernel target (soft-float), so we
// never need to save/restore XMM registers here.

global_asm!(
    r#"
    .global blackwall_switch_to
blackwall_switch_to:
    // rdi = *old_rsp (pointer to where we store the current rsp)
    // rsi = next_rsp (the new stack pointer to load)

    // Save callee-saved registers + rflags onto the current stack.
    pushfq
    push rbp
    push rbx
    push r15
    push r14
    push r13
    push r12

    // Store the current stack pointer into *old_rsp.
    mov [rdi], rsp

    // Load the next task's stack pointer.
    mov rsp, rsi

    // Restore the next task's saved state.
    pop r12
    pop r13
    pop r14
    pop r15
    pop rbx
    pop rbp
    popfq

    // Return into the next task (its return address is now on top of stack).
    ret
"#
);

extern "C" {
    /// Low-level context-switch routine implemented in assembly above.
    ///
    /// # Safety
    ///
    /// - `old_rsp` must point to a valid `u64` that will receive the
    ///   current stack pointer; it must remain valid for the lifetime of
    ///   the task.
    /// - `next_rsp` must be the `stack_ptr` of a task whose stack was
    ///   either initialised by `init_stack` or previously saved by a
    ///   call to this function.
    fn blackwall_switch_to(old_rsp: *mut u64, next_rsp: u64);
}

#[no_mangle]
pub unsafe extern "C" fn thread_trampoline() {
    // 1. Force unlock the scheduler lock
    crate::scheduler::SCHEDULER.force_unlock();

    // 2. Enable interrupts
    asm!("sti", options(nomem, nostack, preserves_flags));

    // 3. Get the entry function pointer from r12
    let entry: fn();
    asm!("mov {}, r12", out(reg) entry, options(nomem, nostack, preserves_flags));

    // 4. Run the entry function
    entry();

    // 5. If the entry function returns, terminate the thread
    asm!("cli", options(nomem, nostack, preserves_flags));
    if let Some(mut sched) = crate::scheduler::SCHEDULER.try_lock() {
        if let Some(ref mut s) = *sched {
            let pid = s.current_pid;
            s.terminate_task(pid);
            s.schedule();
        }
    }

    loop {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Initialise a fresh thread stack so that the first `switch_to` into it
/// will begin executing `thread_trampoline` which calls `entry`.
///
/// # Arguments
///
/// * `stack_top` — pointer one byte **past** the end of the allocated stack
///   region (i.e. the initial value of `rsp` before any pushes).
///   **Must** be 16-byte aligned.
/// * `entry` — the function the thread will start executing.
///
/// # Returns
///
/// The value that should be stored in `ProcessControlBlock::stack_ptr`.
///
/// # Safety
///
/// `stack_top` must point to a valid, writable memory region of at least
/// 7 × 8 = 56 bytes that will live for the entire lifetime of the thread.
pub unsafe fn init_stack(stack_top: *mut u8, entry: fn()) -> u64 {
    // We build the initial stack frame that `switch_to` will "pop" when
    // this thread is first scheduled.
    //
    // Frame layout (from top of stack, addresses decrease):
    //
    //   [stack_top - 8 ]  trampoline address  ← `ret` in switch_to jumps here
    //   [stack_top - 16]  r12  = entry        ← used by trampoline
    //   [stack_top - 24]  r13  = 0
    //   [stack_top - 32]  r14  = 0
    //   [stack_top - 40]  r15  = 0
    //   [stack_top - 48]  rbx  = 0
    //   [stack_top - 56]  rbp  = 0
    //   [stack_top - 64]  rflags = 0x200  (IF set — interrupts enabled)
    //                                      ← this is where rsp will point

    let mut rsp = stack_top as *mut u64;

    // Align down to 8 bytes (stack_top should already be 16-byte aligned).
    rsp = rsp.sub(1);
    // entry address — the `ret` at the end of switch_to pops this to jump to trampoline.
    rsp.write(thread_trampoline as *const () as u64);

    // Saved callee-saved registers.
    rsp = rsp.sub(1);
    rsp.write(entry as *const () as u64); // r12 stores the real entry fn
    rsp = rsp.sub(1);
    rsp.write(0); // r13
    rsp = rsp.sub(1);
    rsp.write(0); // r14
    rsp = rsp.sub(1);
    rsp.write(0); // r15
    rsp = rsp.sub(1);
    rsp.write(0); // rbx
    rsp = rsp.sub(1);
    rsp.write(0); // rbp
                  // rflags — enable interrupts (IF = bit 9 = 0x200).
    rsp = rsp.sub(1);
    rsp.write(0x200);

    rsp as u64
}

/// Perform a context switch from the current task to `next_rsp`.
///
/// Saves the current CPU state to `*old_rsp`, then restores the state
/// from `next_rsp`.
///
/// # Safety
///
/// Same requirements as `blackwall_switch_to`.
pub unsafe fn switch_to(old_rsp: *mut u64, next_rsp: u64) {
    blackwall_switch_to(old_rsp, next_rsp);
}

/// Yield the current execution context.
///
/// Useful inside the idle loop — lets the compiler know execution might
/// not continue linearly here.
#[inline(always)]
pub fn cpu_relax() {
    unsafe {
        asm!("pause", options(nostack, nomem, preserves_flags));
    }
}
