#![allow(bad_asm_style)]
use crate::arch::x86_64::serial;
use core::arch::{asm, global_asm};

global_asm!(
    r#"
    .intel_syntax noprefix

    .global blackwall_divide_by_zero_handler
blackwall_divide_by_zero_handler:
    cli
    hlt
    jmp blackwall_divide_by_zero_handler

    .global blackwall_invalid_opcode_handler
blackwall_invalid_opcode_handler:
    cli
    hlt
    jmp blackwall_invalid_opcode_handler

    .global blackwall_double_fault_handler
blackwall_double_fault_handler:
    cli
    hlt
    jmp blackwall_double_fault_handler

    .global blackwall_general_protection_handler
blackwall_general_protection_handler:
    cli
    hlt
    jmp blackwall_general_protection_handler

    .global blackwall_page_fault_handler
blackwall_page_fault_handler:
    cli
    mov rax, cr2
    mov rdi, rax
    mov rsi, [rsp]
    mov rdx, [rsp + 8]   // RIP of faulting instruction
    mov rcx, [rsp + 32]  // RSP at time of fault
    call blackwall_page_fault_rust
    hlt
    jmp blackwall_page_fault_handler

    .global blackwall_timer_handler
blackwall_timer_handler:
    push rax
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11

    call blackwall_timer_tick_rust

    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rax
    iretq
"#
);

extern "C" {
    fn blackwall_divide_by_zero_handler();
    fn blackwall_invalid_opcode_handler();
    fn blackwall_double_fault_handler();
    fn blackwall_general_protection_handler();
    fn blackwall_page_fault_handler();
    fn blackwall_page_fault_rust(address: u64, error_code: u64, rip: u64, rsp: u64) -> !;
    fn blackwall_timer_handler();
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
        // SAFETY: The IDT is initialized once during startup before interrupts are enabled.
        setup_table();
        load_table();
    }
    serial::line("Interrupt Descriptor Table Loaded");
    serial::line("Exception Handlers Ready");
}

unsafe fn setup_table() {
    IDT.entries[0].set_handler(blackwall_divide_by_zero_handler as *const () as usize);
    IDT.entries[6].set_handler(blackwall_invalid_opcode_handler as *const () as usize);
    IDT.entries[8].set_handler(blackwall_double_fault_handler as *const () as usize);
    IDT.entries[13].set_handler(blackwall_general_protection_handler as *const () as usize);
    IDT.entries[14].set_handler(blackwall_page_fault_handler as *const () as usize);
    IDT.entries[0x20].set_handler(blackwall_timer_handler as *const () as usize);
}

unsafe fn load_table() {
    let pointer = IdtPointer {
        limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: core::ptr::addr_of!(IDT.entries) as *const _ as u64,
    };

    asm!("lidt [{ptr}]", ptr = in(reg) &pointer, options(readonly, nostack, preserves_flags));
}
