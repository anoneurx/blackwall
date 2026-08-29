use crate::arch::x86_64::serial;
use core::arch::asm;

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
struct TaskStateSegment {
    reserved1: u32,
    privilege_stack_table: [u64; 3],
    reserved2: u64,
    interrupt_stack_table: [u64; 7],
    reserved3: u64,
    reserved4: u16,
    io_map_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            reserved1: 0,
            privilege_stack_table: [0; 3],
            reserved2: 0,
            interrupt_stack_table: [0; 7],
            reserved3: 0,
            reserved4: 0,
            io_map_base: core::mem::size_of::<Self>() as u16,
        }
    }
}

#[repr(align(16))]
struct GdtState {
    entries: [u64; 7],
    tss: TaskStateSegment,
}

static mut GDT_STATE: GdtState = GdtState { entries: [0; 7], tss: TaskStateSegment::new() };

static mut INTERRUPT_STACK: [u8; 4096 * 2] = [0; 4096 * 2];

pub fn init() {
    unsafe {
        // SAFETY: This initializes the early boot GDT and TSS once during kernel startup.
        setup_tables();
        load_tables();
    }
    serial::line("GDT Initialized");
}

unsafe fn setup_tables() {
    let stack_base = core::ptr::addr_of!(INTERRUPT_STACK) as *const u8 as u64;
    let stack_top = stack_base + core::mem::size_of::<[u8; 4096 * 2]>() as u64;
    GDT_STATE.tss.interrupt_stack_table[0] = stack_top;

    GDT_STATE.entries[0] = 0;
    GDT_STATE.entries[1] = 0x00af9a000000ffff; // Kernel Code (0x08)
    GDT_STATE.entries[2] = 0x00af92000000ffff; // Kernel Data (0x10)
    GDT_STATE.entries[3] = 0x00aff2000000ffff; // User Data   (0x18)
    GDT_STATE.entries[4] = 0x00affa000000ffff; // User Code   (0x20)

    let tss_base = core::ptr::addr_of!(GDT_STATE.tss) as *const _ as u64;
    let tss_limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;

    GDT_STATE.entries[5] = tss_low_descriptor(tss_base, tss_limit);
    GDT_STATE.entries[6] = tss_high_descriptor(tss_base);
}

unsafe fn load_tables() {
    let descriptor = DescriptorTablePointer {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: core::ptr::addr_of!(GDT_STATE.entries) as *const _ as u64,
    };

    asm!("lgdt [{ptr}]", ptr = in(reg) &descriptor, options(readonly, nostack, preserves_flags));

    let data_selector: u16 = 2 << 3;
    asm!(
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "mov fs, ax",
        "mov gs, ax",
        in("ax") data_selector,
        options(nostack, preserves_flags)
    );

    let tss_selector: u16 = 5 << 3;
    asm!("ltr ax", in("ax") tss_selector, options(nostack, preserves_flags));
}

const fn tss_low_descriptor(base: u64, limit: u64) -> u64 {
    let mut low = (limit & 0xffff) | ((base & 0x00ff_ffff) << 16);
    low |= 0x89u64 << 40;
    low |= ((limit >> 16) & 0x0f) << 48;
    low |= ((base >> 24) & 0xff) << 56;
    low
}

const fn tss_high_descriptor(base: u64) -> u64 {
    base >> 32
}

pub unsafe fn set_tss_stack(stack_ptr: u64) {
    // Sets RSP0 so the CPU knows where the kernel stack is when an interrupt
    // occurs while executing in Ring 3.
    GDT_STATE.tss.privilege_stack_table[0] = stack_ptr;
}
