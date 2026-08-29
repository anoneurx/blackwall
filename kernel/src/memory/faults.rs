use crate::{
    arch::x86_64::{interrupts, serial},
    logging,
};

pub fn install() {}

#[no_mangle]
pub extern "C" fn blackwall_page_fault_rust(
    address: u64,
    error_code: u64,
    rip: u64,
    rsp: u64,
) -> ! {
    serial::line("PAGE FAULT");
    logging::print(format_args!("Address: 0x{address:016x}\n"));
    logging::print(format_args!("Error Code: 0x{error_code:x}\n"));
    logging::print(format_args!("RIP: 0x{rip:016x}\n"));
    logging::print(format_args!("RSP: 0x{rsp:016x}\n"));
    serial::line("CPU Halted");
    interrupts::halt_forever()
}
