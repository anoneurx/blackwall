#[cfg(target_os = "uefi")]
use crate::arch::x86_64::{interrupts, serial};
#[cfg(target_os = "uefi")]
use core::panic::PanicInfo;

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    serial::line("KERNEL PANIC");
    crate::logging::print(format_args!("{}\n", info.message()));
    if let Some(location) = info.location() {
        crate::logging::print(format_args!("{}:{}\n", location.file(), location.line()));
    }
    interrupts::halt_forever()
}
