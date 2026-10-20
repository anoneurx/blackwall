#[cfg(target_os = "uefi")]
use crate::arch::x86_64::{interrupts, serial};
#[cfg(target_os = "uefi")]
use core::panic::PanicInfo;

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    interrupts::disable();
    serial::line("\n\n========================================");
    serial::line("KERNEL PANIC");
    serial::line("========================================");
    crate::logging::print(format_args!("{}\n", info.message()));
    if let Some(location) = info.location() {
        crate::logging::print(format_args!("Location: {}:{}\n", location.file(), location.line()));
    }
    serial::line("========================================");
    serial::line("System halted.");
    interrupts::halt_forever()
}
