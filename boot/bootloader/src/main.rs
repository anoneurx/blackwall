#![cfg_attr(target_os = "uefi", no_std)]
#![cfg_attr(target_os = "uefi", no_main)]

#[cfg(target_os = "uefi")]
mod efi {
    use blackwall_kernel::init;
    use uefi::prelude::*;

    #[entry]
    fn efi_main(_handle: Handle, _system_table: SystemTable<Boot>) -> Status {
        init::start(_system_table)
    }
}

#[cfg(not(target_os = "uefi"))]
fn main() {
    println!("blackwall-bootloader host stub");
}
