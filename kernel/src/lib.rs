#![cfg_attr(target_os = "uefi", no_std)]

#[cfg(target_os = "uefi")]
extern crate alloc;

#[cfg(target_os = "uefi")]
pub mod arch;
#[cfg(target_os = "uefi")]
pub mod containers;
#[cfg(target_os = "uefi")]
pub mod drivers;
#[cfg(target_os = "uefi")]
pub mod fs;
pub mod init;
#[cfg(target_os = "uefi")]
pub mod ipc;
pub mod logging;
#[cfg(target_os = "uefi")]
pub mod memory;
#[cfg(target_os = "uefi")]
pub mod net;
#[cfg(target_os = "uefi")]
pub mod panic;
#[cfg(target_os = "uefi")]
pub mod process;
#[cfg(target_os = "uefi")]
pub mod scheduler;
#[cfg(target_os = "uefi")]
pub mod security;
#[cfg(target_os = "uefi")]
pub mod smp;
#[cfg(target_os = "uefi")]
pub mod sync;
#[cfg(target_os = "uefi")]
pub mod syscall;
#[cfg(target_os = "uefi")]
pub mod thread;

#[cfg(target_os = "uefi")]
pub use init::start as kernel_main;
