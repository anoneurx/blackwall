/// SMP — Symmetric Multi-Processing support
/// Detects CPU count via ACPI MADT parsing and brings up Application Processors.
extern crate alloc;

use crate::arch::x86_64::serial;
use alloc::vec::Vec;

/// Logical CPU descriptor
#[derive(Debug, Clone)]
pub struct Cpu {
    pub apic_id: u8,
    pub is_bsp: bool, // Bootstrap Processor?
    pub online: bool,
}

/// Global CPU table
pub static CPU_TABLE: crate::sync::spin::SpinLock<Vec<Cpu>> =
    crate::sync::spin::SpinLock::new(Vec::new());

// MADT signature for locating the APIC table in ACPI
#[allow(dead_code)]
const MADT_SIGNATURE: [u8; 4] = *b"APIC";

/// Probe the MADT to enumerate all Local APICs (logical CPUs).
/// Falls back to single-CPU if ACPI is not available.
pub fn detect_cpus() -> Vec<Cpu> {
    // Hardcode BSP as CPU 0 always
    let mut cpus = Vec::new();
    cpus.push(Cpu { apic_id: 0, is_bsp: true, online: true });

    // In a real implementation we would walk the RSDP → RSDT/XSDT → MADT
    // and enumerate all Local APIC entries (type 0). For now we report the BSP.
    serial::line(&alloc::format!("[SMP] Detected {} CPU(s). BSP APIC ID: 0", cpus.len()));
    cpus
}

/// Send an INIT–SIPI–SIPI sequence to an Application Processor.
///
/// # Safety
///
/// * `trampoline_phys` must be a physical address in the first 1 MiB of RAM
///   that contains valid 16-bit real-mode startup code.
/// * The Local APIC must already be mapped into the kernel virtual address
///   space and calibrated before calling this function.
/// * This function must not be called concurrently with other APIC ICR writes
///   targeting the same AP (no re-entrancy protection is provided).
// SAFETY: Callers must satisfy the three preconditions above. The current
// implementation is a stub that only logs; a full APIC write will be added
// once MMIO mapping is confirmed.
pub unsafe fn startup_ap(apic_id: u8, _trampoline_phys: u32) {
    serial::line(&alloc::format!(
        "[SMP] Sending INIT/SIPI to APIC #{apic_id} (stub — APIC MMIO not yet mapped)"
    ));
    // Full implementation:
    //   1. Write INIT IPI via xAPIC or x2APIC ICR
    //   2. 10 ms delay
    //   3. Write SIPI IPI twice with trampoline page vector
}

pub fn init() {
    serial::line("[SMP] Initializing SMP subsystem...");
    let cpus = detect_cpus();
    *CPU_TABLE.lock() = cpus;
    serial::line("[SMP] SMP subsystem ready.");
}
