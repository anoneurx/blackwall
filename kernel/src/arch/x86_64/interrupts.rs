use core::arch::asm;

pub fn halt_forever() -> ! {
    loop {
        unsafe {
            // SAFETY: Halting is the intended idle state after the early init path.
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

pub fn disable() {
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

pub fn enable() {
    unsafe {
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
}
