use crate::arch::x86_64::serial;
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    serial::line("[DEBUG] Entering timer::init()...");
    unsafe {
        // Remap PIC to avoid exception conflict (map IRQs to 0x20-0x2F)
        remap_pic();
        serial::line("[DEBUG] PIC remapped.");
        // SAFETY: This programs the legacy PIT channel 0 for early boot timing.
        program_pit(100);
        serial::line("[DEBUG] PIT programmed.");
    }
    TICKS.store(0, Ordering::Relaxed);
    serial::line("Timer Initialized");
    serial::line("System Tick: 100Hz");
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn blackwall_timer_tick_rust() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;

    // Send End of Interrupt (EOI) to Master PIC
    unsafe {
        outb(0x20, 0x20);
    }

    // Call scheduler tick
    if let Some(mut sched) = crate::scheduler::SCHEDULER.try_lock() {
        if let Some(ref mut s) = *sched {
            s.tick(ticks);
        }
    }
}

unsafe fn remap_pic() {
    // ICW1: initialization command
    outb(0x20, 0x11);
    outb(0xA0, 0x11);

    // ICW2: Master offset = 0x20, Slave offset = 0x28
    outb(0x21, 0x20);
    outb(0xA1, 0x28);

    // ICW3: cascade configuration
    outb(0x21, 0x04);
    outb(0xA1, 0x02);

    // ICW4: 8086 mode
    outb(0x21, 0x01);
    outb(0xA1, 0x01);

    // Mask all interrupts except PIT (IRQ0)
    outb(0x21, 0xfe);
    outb(0xA1, 0xff);
}

unsafe fn program_pit(hz: u32) {
    let divisor = 1_193_182u32 / hz;
    outb(0x43, 0x36);
    outb(0x40, (divisor & 0xff) as u8);
    outb(0x40, ((divisor >> 8) & 0xff) as u8);
}

unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}
