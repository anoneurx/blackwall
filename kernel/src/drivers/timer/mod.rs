use crate::arch::x86_64::serial;
/// PIT (Programmable Interval Timer) and APIC Timer driver
/// Configures 8253/8254 PIT at ~100 Hz and provides tick counting.
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

const PIT_CH0_DATA: u16 = 0x40;
#[allow(dead_code)]
const PIT_CH2_DATA: u16 = 0x42;
const PIT_CMD: u16 = 0x43;
const PIT_BASE_HZ: u32 = 1_193_182;

pub static TICKS: AtomicU64 = AtomicU64::new(0);

/// Write one byte to an x86 I/O port.
///
/// # Safety
/// Caller must supply a valid, ring-0-accessible I/O port address.
/// On x86_64 all I/O port access is restricted to CPL=0.
unsafe fn outb(port: u16, val: u8) {
    // SAFETY: I/O port access is CPL=0 only; we are in kernel mode.
    asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
}
/// Read one byte from an x86 I/O port.
///
/// # Safety
/// Same as `outb` — caller ensures a valid, accessible port address.
unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    // SAFETY: I/O port access is CPL=0 only; we are in kernel mode.
    asm!("in al, dx", in("dx") port, out("al") v, options(nostack, preserves_flags));
    v
}

/// Configure PIT channel 0 to fire IRQ0 at `freq_hz` Hz.
pub fn set_frequency(freq_hz: u32) {
    let divisor = (PIT_BASE_HZ / freq_hz) as u16;
    // SAFETY: PIT_CMD (0x43) is the PIT mode/command register and PIT_CH0_DATA
    // (0x40) is channel 0's data port — both are standard PC hardware registers
    // available in ring-0. The mode byte 0x36 selects channel 0, lobyte/hibyte
    // access, square-wave (mode 3), binary counting.
    unsafe {
        outb(PIT_CMD, 0x36);
        outb(PIT_CH0_DATA, (divisor & 0xFF) as u8);
        outb(PIT_CH0_DATA, (divisor >> 8) as u8);
    }
}

/// Sleep for approximately `ms` milliseconds using PIT ticks.
pub fn sleep_ms(ms: u64) {
    let start = TICKS.load(Ordering::Relaxed);
    let ticks_per_ms = 100; // assuming 100 Hz PIT
    let end = start + ms * ticks_per_ms / 1000;
    while TICKS.load(Ordering::Relaxed) < end {
        core::hint::spin_loop();
    }
}

/// Read the current PIT counter value (for polling)
pub fn read_counter() -> u16 {
    // SAFETY: Latches and reads PIT channel 0 counter via the standard latch
    // command (write 0x00 to PIT_CMD) followed by two reads from PIT_CH0_DATA.
    // These are the standard 8253/8254 register addresses, safe in ring-0.
    unsafe {
        outb(PIT_CMD, 0x00);
        let lo = inb(PIT_CH0_DATA);
        let hi = inb(PIT_CH0_DATA);
        ((hi as u16) << 8) | lo as u16
    }
}

/// Call this from the IRQ 0 (timer) handler.
pub fn handle_irq() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn init() {
    set_frequency(100); // 100 Hz
    serial::line("[TIMER] PIT initialized at 100 Hz. IRQ0 armed.");
}
