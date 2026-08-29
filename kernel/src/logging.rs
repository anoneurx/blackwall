use core::fmt::{self, Write};

#[cfg(target_os = "uefi")]
use crate::arch::x86_64::serial;

#[cfg(target_os = "uefi")]
struct SerialWriter;

#[cfg(target_os = "uefi")]
impl Write for SerialWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        serial::write_str(text);
        Ok(())
    }
}

#[cfg(not(target_os = "uefi"))]
struct SerialWriter;

#[cfg(not(target_os = "uefi"))]
impl Write for SerialWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        eprint!("{text}");
        Ok(())
    }
}

pub fn print(args: fmt::Arguments<'_>) {
    let _ = SerialWriter.write_fmt(args);
}

pub fn info(message: &str) {
    print(format_args!("[KERNEL] {message}\n"));
}

pub fn debug(message: &str) {
    print(format_args!("[KERNEL] DEBUG: {message}\n"));
}

pub fn error(message: &str) {
    print(format_args!("[KERNEL] ERROR: {message}\n"));
}
