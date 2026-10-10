//! SMP — Symmetric Multi-Processing support.
//!
//! The MADT (parsed by the ACPI driver) enumerates the application
//! processors. Each AP is woken with the standard INIT–SIPI–SIPI sequence; a
//! small position-independent trampoline copied to physical `0x8000` brings it
//! from 16-bit real mode into long mode and calls [`ap_entry`].

extern crate alloc;

use crate::arch::x86_64::serial;
use crate::arch::x86_64::{apic, idt};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Physical address the trampoline is copied to (must be below 1 MiB and on a
/// 4 KiB boundary). The SIPI vector is `0x8000 >> 12 == 0x08`.
const TRAMPOLINE_BASE: usize = 0x8000;
const TRAMPOLINE_VECTOR: u8 = (TRAMPOLINE_BASE >> 12) as u8;

/// One 16 KiB stack per AP (up to 8 APs supported).
const AP_STACK_SIZE: usize = 16 * 1024;
const MAX_APS: usize = 8;

#[repr(align(16))]
#[allow(dead_code)]
struct AlignedStack([u8; AP_STACK_SIZE]);

static mut AP_STACKS: [AlignedStack; MAX_APS] =
    [const { AlignedStack([0; AP_STACK_SIZE]) }; MAX_APS];

/// Logical CPU descriptor.
#[derive(Debug, Clone)]
pub struct Cpu {
    pub apic_id: u8,
    pub is_bsp: bool,
    pub online: bool,
}

/// Global CPU table (index 0 is the bootstrap processor).
pub static CPU_TABLE: crate::sync::spin::SpinLock<Vec<Cpu>> =
    crate::sync::spin::SpinLock::new(Vec::new());

/// Number of CPUs that have reached [`ap_entry`].
static ONLINE_APS: AtomicUsize = AtomicUsize::new(0);

// ── Trampoline symbols (defined in the global_asm block below) ────────────────
extern "C" {
    static __ap_trampoline_start: u8;
    static __ap_trampoline_end: u8;
    static __ap_gdt_ptr: u8;
    static __ap_gdt: u8;
    static __ap_gdt_end: u8;
    static __ap_cr3: u8;
    static __ap_entry64: u8;
    static __ap_stacks_base: u8;
    static __ap_apic_base: u8;
}

// Written in AT&T syntax (via `options(att_syntax)`) because GAS Intel syntax
// treats a bare symbol in an operand as a memory reference, which would turn
// the constant `0x8000 + offset` additions into loads from address 0.
//
// Each AP discovers its own identity by reading its local APIC ID from the
// MMIO ID register, then selects its private stack from the stack table. This
// avoids any shared state that the BSP could race against.
core::arch::global_asm!(
    r#"
    .section .text
    .align 16
    .global __ap_trampoline_start
    .global __ap_trampoline_end
    .global __ap_gdt_ptr
    .global __ap_gdt
    .global __ap_gdt_end
    .global __ap_cr3
    .global __ap_entry64
    .global __ap_stacks_base
    .global __ap_apic_base

__ap_trampoline_start:
    jmp __ap_entry16

    // ---- data area (BSP patches these from Rust) ----
    .align 16
__ap_gdt_ptr:
    .word 0
    .long 0
__ap_cr3:
    .quad 0
__ap_entry64:
    .quad 0
__ap_stacks_base:
    .quad 0
__ap_apic_base:
    .quad 0

    .align 16
__ap_gdt:
    .quad 0x0000000000000000
    .quad 0x00cf9a000000ffff
    .quad 0x00af9a000000ffff
    .quad 0x00cf92000000ffff
__ap_gdt_end:

    .set OFF_GDT_PTR, __ap_gdt_ptr - __ap_trampoline_start
    .set OFF_CR3, __ap_cr3 - __ap_trampoline_start
    .set OFF_ENTRY, __ap_entry64 - __ap_trampoline_start
    .set OFF_STACKS, __ap_stacks_base - __ap_trampoline_start
    .set OFF_APIC, __ap_apic_base - __ap_trampoline_start

    .align 16
    .code16
__ap_entry16:
    cli
    cld
    xor %ax, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %ax, %ss
    mov $0x7c00, %sp

    in $0x92, %al
    or $0x02, %al
    out %al, $0x92

    mov $0x8000, %bx
    add $OFF_GDT_PTR, %bx
    lgdt (%bx)

    mov %cr0, %eax
    or $1, %eax
    mov %eax, %cr0

    .byte 0x66, 0xea
    .long 0x8000 + (__ap_pm32 - __ap_trampoline_start)
    .word 0x08

    .code32
__ap_pm32:
    mov $0x18, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov %ax, %fs
    mov %ax, %gs

    mov %cr4, %eax
    or $0x20, %eax
    mov %eax, %cr4

    mov $0x8000, %ebx
    add $OFF_CR3, %ebx
    mov (%ebx), %eax
    mov %eax, %cr3

    mov $0xC0000080, %ecx
    rdmsr
    or $0x100, %eax
    wrmsr

    mov %cr0, %eax
    or $0x80000000, %eax
    mov %eax, %cr0

    .byte 0xea
    .long 0x8000 + (__ap_lm64 - __ap_trampoline_start)
    .word 0x10

    .code64
__ap_lm64:
    mov $0x18, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov %ax, %fs
    mov %ax, %gs

    // Match the BSP's FPU/SSE context so generated Rust code (which may use
    // XMM registers) does not fault with #UD.
    mov %cr0, %rax
    or $0x22, %rax
    btr $3, %rax
    btr $2, %rax
    mov %rax, %cr0
    mov %cr4, %rax
    or $0x600, %rax
    mov %rax, %cr4

    // Read this CPU's APIC ID from the local APIC ID register.
    mov $0x8000, %rbx
    add $OFF_APIC, %rbx
    mov (%rbx), %rbx
    mov 0x20(%rbx), %ecx
    shr $24, %ecx

    // rsp = stacks_base + (apic_id + 1) * AP_STACK_SIZE
    mov $0x8000, %rbx
    add $OFF_STACKS, %rbx
    mov (%rbx), %rbx
    lea 1(%rcx), %rax
    shl $14, %rax
    add %rax, %rbx
    mov %rbx, %rsp

    // Call ap_entry(apic_id) — first argument in RCX (Win64/UEFI ABI).
    mov $0x8000, %rbx
    add $OFF_ENTRY, %rbx
    mov (%rbx), %rax
    call *%rax

1:
    cli
    hlt
    jmp 1b

__ap_trampoline_end:
"#,
    options(att_syntax)
);

/// Runtime address of a trampoline symbol once the blob lives at `0x8000`.
fn blob_addr(sym: *const u8) -> usize {
    let start = core::ptr::addr_of!(__ap_trampoline_start) as usize;
    TRAMPOLINE_BASE + (sym as usize - start)
}

unsafe fn write_u32(sym: *const u8, value: u32) {
    core::ptr::write_unaligned(blob_addr(sym) as *mut u32, value);
}

unsafe fn write_u64(sym: *const u8, value: u64) {
    core::ptr::write_unaligned(blob_addr(sym) as *mut u64, value);
}

/// Copy the trampoline blob to `0x8000` and patch in the fixed fields.
unsafe fn install_trampoline(cr3: u64, apic_base: u64) {
    let start = core::ptr::addr_of!(__ap_trampoline_start) as *const u8;
    let end = core::ptr::addr_of!(__ap_trampoline_end) as *const u8;
    let size = end as usize - start as usize;
    core::ptr::copy_nonoverlapping(start, TRAMPOLINE_BASE as *mut u8, size);

    let gdt_base = blob_addr(core::ptr::addr_of!(__ap_gdt)) as u64;
    let gdt_limit = (core::ptr::addr_of!(__ap_gdt_end) as usize
        - core::ptr::addr_of!(__ap_gdt) as usize
        - 1) as u16;

    core::ptr::write_unaligned(blob_addr(core::ptr::addr_of!(__ap_gdt_ptr)) as *mut u16, gdt_limit);
    write_u32(core::ptr::addr_of!(__ap_gdt_ptr).add(2), gdt_base as u32);
    write_u64(core::ptr::addr_of!(__ap_cr3), cr3);
    write_u64(core::ptr::addr_of!(__ap_entry64), ap_entry as *const () as u64);
    write_u64(core::ptr::addr_of!(__ap_stacks_base), core::ptr::addr_of!(AP_STACKS) as u64);
    write_u64(core::ptr::addr_of!(__ap_apic_base), apic_base);
}

/// Estimated TSC frequency in MHz (from CPUID leaf 0x16, else a sane default).
fn tsc_mhz() -> u64 {
    let r = core::arch::x86_64::__cpuid(0x16);
    if r.eax != 0 {
        r.eax as u64
    } else {
        2000
    }
}

/// Busy-wait approximately `us` microseconds using the time-stamp counter.
fn delay_us(us: u64, mhz: u64) {
    let target = us * mhz;
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    while unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(start) < target {
        core::hint::spin_loop();
    }
}

/// APIC base address from ACPI, falling back to the architectural default.
fn apic_base() -> u64 {
    let acpi = crate::drivers::acpi::ACPI.lock();
    match acpi.as_ref() {
        Some(a) if a.local_apic_addr != 0 => a.local_apic_addr as u64,
        _ => apic::XAPIC_DEFAULT_BASE,
    }
}

/// Enumerate the enabled CPUs from the MADT.
pub fn detect_cpus() -> Vec<Cpu> {
    let bsp_apic_id = apic::id() as u8;
    let mut cpus = Vec::new();

    let ids = {
        let acpi = crate::drivers::acpi::ACPI.lock();
        acpi.as_ref().map(|a| a.local_apic_ids.clone()).unwrap_or_default()
    };

    if ids.is_empty() {
        cpus.push(Cpu { apic_id: bsp_apic_id, is_bsp: true, online: true });
        return cpus;
    }

    for id in ids {
        let is_bsp = id == bsp_apic_id;
        cpus.push(Cpu { apic_id: id, is_bsp, online: is_bsp });
    }
    cpus
}

/// Entry point executed by every application processor (long mode, IF=0).
///
/// # Safety
/// Called only from the AP trampoline with the processor's own APIC ID.
#[no_mangle]
pub extern "C" fn ap_entry(apic_id: u64) -> ! {
    // Give this CPU a valid IDT and enable its local APIC.
    unsafe {
        idt::reload();
    }
    apic::init(apic_base());

    let id = apic_id as u8;
    let mut slot = usize::MAX;
    {
        let mut table = CPU_TABLE.lock();
        for (i, cpu) in table.iter_mut().enumerate() {
            if cpu.apic_id == id {
                cpu.online = true;
                slot = i;
            }
        }
    }
    ONLINE_APS.fetch_add(1, Ordering::SeqCst);

    serial::line(&alloc::format!("[SMP] CPU {} (APIC ID {}) online.", slot, id));

    loop {
        core::hint::spin_loop();
    }
}

/// Wake a single AP with the INIT–SIPI–SIPI sequence.
unsafe fn startup_ap(apic_id: u8, mhz: u64) {
    let dest = apic_id as u32;
    apic::send_init_assert(dest);
    delay_us(10_000, mhz);
    apic::send_init_deassert(dest);
    delay_us(10_000, mhz);
    apic::send_sipi(dest, TRAMPOLINE_VECTOR);
    delay_us(200, mhz);
    apic::send_sipi(dest, TRAMPOLINE_VECTOR);
    delay_us(200, mhz);
}

pub fn init() {
    serial::line("[SMP] Initializing SMP subsystem...");

    let cpus = detect_cpus();
    let ap_count = cpus.iter().filter(|c| !c.is_bsp).count();
    {
        let mut table = CPU_TABLE.lock();
        *table = cpus;
    }
    serial::line(&alloc::format!(
        "[SMP] Detected {} CPU(s), BSP APIC ID {}",
        CPU_TABLE.lock().len(),
        apic::id()
    ));

    if ap_count == 0 {
        serial::line("[SMP] No application processors to start.");
        return;
    }

    let base = apic_base();
    apic::init(base);

    let cr3 = crate::arch::x86_64::trap::current_cr3();
    unsafe {
        install_trampoline(cr3, base);
    }

    let mhz = tsc_mhz();
    let bsp_id = apic::id() as u8;
    let ap_ids: Vec<u8> = {
        let table = CPU_TABLE.lock();
        table
            .iter()
            .filter(|c| !c.is_bsp && c.apic_id != bsp_id)
            .take(MAX_APS)
            .map(|c| c.apic_id)
            .collect()
    };

    for apic_id in ap_ids {
        serial::line(&alloc::format!("[SMP] Starting APIC ID {}...", apic_id));
        unsafe {
            startup_ap(apic_id, mhz);
        }
    }

    // Give the APs a moment to report in.
    let mut waited = 0u64;
    while ONLINE_APS.load(Ordering::SeqCst) < ap_count && waited < 1000 {
        delay_us(500, mhz);
        waited += 1;
    }

    let online = {
        let table = CPU_TABLE.lock();
        table.iter().filter(|c| c.online).count()
    };
    serial::line(&alloc::format!("[SMP] {} of {} CPU(s) online.", online, ap_count + 1));
}
