//! xAPIC (memory-mapped local APIC) driver.
//!
//! Provides the minimal functionality needed for SMP bring-up: enable the
//! local APIC, read its ID, send INIT/SIPI/fixed IPIs, and acknowledge
//! interrupts. External interrupts still come through the legacy PIC; the
//! APIC is used only for inter-processor messages.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

pub const XAPIC_DEFAULT_BASE: u64 = 0xFEE0_0000;

// Register offsets from the MMIO base.
const REG_ID: usize = 0x020;
const REG_TPR: usize = 0x080;
const REG_EOI: usize = 0x0B0;
const REG_SVR: usize = 0x0F0;
const REG_ICR_LOW: usize = 0x300;
const REG_ICR_HIGH: usize = 0x310;
const REG_LVT_LINT0: usize = 0x350;
const REG_LVT_LINT1: usize = 0x360;
const REG_LVT_ERROR: usize = 0x370;

const APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;

// ICR delivery/trigger encodings.
const ICR_DELIVERY_INIT: u32 = 0x00500;
const ICR_DELIVERY_STARTUP: u32 = 0x00600;
const ICR_TRIGGER_LEVEL: u32 = 1 << 14;
const ICR_LEVEL_ASSERT: u32 = 1 << 13;
const ICR_DELIVERY_PENDING: u32 = 1 << 12;

static APIC_BASE: AtomicU64 = AtomicU64::new(XAPIC_DEFAULT_BASE);

#[inline(always)]
fn read(off: usize) -> u32 {
    let base = APIC_BASE.load(Ordering::Relaxed);
    // SAFETY: `base` points at the identity-mapped local APIC MMIO region.
    unsafe { core::ptr::read_volatile((base + off as u64) as *const u32) }
}

#[inline(always)]
fn write(off: usize, value: u32) {
    let base = APIC_BASE.load(Ordering::Relaxed);
    // SAFETY: as above.
    unsafe { core::ptr::write_volatile((base + off as u64) as *mut u32, value) };
}

unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nostack, preserves_flags));
    ((hi as u64) << 32) | lo as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags)
    );
}

/// Enable the local APIC at `base` (physical address) and set sane defaults.
pub fn init(base: u64) {
    let base = base & 0xFFFF_FFFF_FFFF_F000;
    // Program the APIC base MSR: clear any x2APIC bit, set the address and the
    // global enable bit. This also enables the APIC for the calling CPU only.
    let mut msr = unsafe { rdmsr(APIC_BASE_MSR) };
    msr &= !0x0000_0FFF; // clear the base address field
    msr &= !(1 << 10); // force xAPIC (disable x2APIC)
    msr |= base;
    msr |= APIC_BASE_ENABLE;
    unsafe { wrmsr(APIC_BASE_MSR, msr) };
    APIC_BASE.store(base, Ordering::Relaxed);

    // Software-enable the APIC, spurious vector 0xFF, task priority 0.
    write(REG_SVR, 0x1FF);
    write(REG_TPR, 0);

    // Mask the legacy LINT0/LINT1 lines (PIC handles external IRQs).
    write(REG_LVT_LINT0, 1 << 16);
    write(REG_LVT_LINT1, 1 << 16);
    write(REG_LVT_ERROR, 0);
    // Clear any stale interrupt.
    write(REG_EOI, 0);
}

/// Read the calling CPU's local APIC ID.
pub fn id() -> u32 {
    read(REG_ID) >> 24
}

/// Signal end-of-interrupt to the local APIC.
pub fn eoi() {
    write(REG_EOI, 0);
}

fn wait_ready() {
    while read(REG_ICR_LOW) & ICR_DELIVERY_PENDING != 0 {
        core::hint::spin_loop();
    }
}

/// Send an arbitrary ICR-low value to a physical destination APIC ID.
pub fn send_ipi(dest_apic_id: u32, icr_low: u32) {
    wait_ready();
    write(REG_ICR_HIGH, dest_apic_id << 24);
    write(REG_ICR_LOW, icr_low);
    wait_ready();
}

/// Assert INIT to `dest`.
pub fn send_init_assert(dest: u32) {
    send_ipi(dest, ICR_DELIVERY_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_ASSERT);
}

/// Deassert INIT to `dest`.
pub fn send_init_deassert(dest: u32) {
    send_ipi(dest, ICR_DELIVERY_INIT | ICR_TRIGGER_LEVEL);
}

/// Send a STARTUP IPI (SIPI) to `dest` with the given 4 KiB page vector.
pub fn send_sipi(dest: u32, vector: u8) {
    send_ipi(dest, ICR_DELIVERY_STARTUP | vector as u32);
}
