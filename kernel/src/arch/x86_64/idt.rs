#![allow(bad_asm_style)]
use crate::arch::x86_64::serial;
use crate::arch::x86_64::trap::{
    blackwall_trap_exc_de, blackwall_trap_exc_df, blackwall_trap_exc_gp, blackwall_trap_exc_pf,
    blackwall_trap_exc_ud, blackwall_trap_keyboard, blackwall_trap_timer,
};
use core::arch::asm;

#[repr(C, packed)]
#[allow(dead_code)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self { offset_low: 0, selector: 0, options: 0, offset_mid: 0, offset_high: 0, reserved: 0 }
    }

    /// Install an interrupt-gate handler (DPL 0, present, interrupt gate).
    fn set_handler(&mut self, handler: usize) {
        self.offset_low = handler as u16;
        self.selector = 0x08;
        self.options = 0x8e00;
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.reserved = 0;
    }
}

#[repr(C, align(16))]
struct InterruptDescriptorTable {
    entries: [IdtEntry; 256],
}

impl InterruptDescriptorTable {
    const fn new() -> Self {
        Self { entries: [IdtEntry::missing(); 256] }
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

pub fn init() {
    unsafe {
        // SAFETY: The IDT is initialized once during startup before interrupts
        // are enabled.
        setup_table();
        load_table();
    }
    serial::line("Interrupt Descriptor Table Loaded");
    serial::line("Exception Handlers Ready");
}

unsafe fn setup_table() {
    // Exceptions without an error code.
    IDT.entries[0].set_handler(blackwall_trap_exc_de as *const () as usize);
    IDT.entries[6].set_handler(blackwall_trap_exc_ud as *const () as usize);
    // Exceptions that push an error code.
    IDT.entries[8].set_handler(blackwall_trap_exc_df as *const () as usize);
    IDT.entries[13].set_handler(blackwall_trap_exc_gp as *const () as usize);
    IDT.entries[14].set_handler(blackwall_trap_exc_pf as *const () as usize);

    // Hardware IRQs (PIC remapped to 0x20..=0x2F).
    IDT.entries[0x20].set_handler(blackwall_trap_timer as *const () as usize);
    IDT.entries[0x21].set_handler(blackwall_trap_keyboard as *const () as usize);
    serial::line(&alloc::format!(
        "[IDT] de={:#x} timer={:#x} yield={:#x}",
        blackwall_trap_exc_de as *const () as usize,
        blackwall_trap_timer as *const () as usize,
        crate::arch::x86_64::trap::blackwall_trap_yield as *const () as usize,
    ));
}

unsafe fn load_table() {
    let pointer = IdtPointer {
        limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT.entries) as *const _ as u64,
    };

    asm!("lidt [{ptr}]", ptr = in(reg) &pointer, options(readonly, nostack, preserves_flags));
}

/// Re-load the already-built IDT into this CPU's IDTR.
///
/// Application processors start with a null IDTR; loading the shared table
/// gives them valid handlers so an unexpected exception does not triple-fault.
pub unsafe fn reload() {
    load_table();
}
