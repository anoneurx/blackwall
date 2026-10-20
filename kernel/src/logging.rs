use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU8, Ordering};

#[cfg(target_os = "uefi")]
use crate::arch::x86_64::serial;

/// Log levels in order of verbosity (lower = more verbose)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Off = 5,
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

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

pub fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn get_level() -> LogLevel {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Trace,
        1 => LogLevel::Debug,
        2 => LogLevel::Info,
        3 => LogLevel::Warn,
        4 => LogLevel::Error,
        5 => LogLevel::Off,
        _ => LogLevel::Info,
    }
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {
        if $crate::logging::get_level() <= $crate::logging::LogLevel::Trace {
            $crate::logging::print(format_args!("[TRACE] {}\n", format_args!($($arg)*)));
        }
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::logging::get_level() <= $crate::logging::LogLevel::Debug {
            $crate::logging::print(format_args!("[DEBUG] {}\n", format_args!($($arg)*)));
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::logging::get_level() <= $crate::logging::LogLevel::Info {
            $crate::logging::print(format_args!("[INFO] {}\n", format_args!($($arg)*)));
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::logging::get_level() <= $crate::logging::LogLevel::Warn {
            $crate::logging::print(format_args!("[WARN] {}\n", format_args!($($arg)*)));
        }
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        if $crate::logging::get_level() <= $crate::logging::LogLevel::Error {
            $crate::logging::print(format_args!("[ERR] {}\n", format_args!($($arg)*)));
        }
    };
}

pub fn trace(message: &str) {
    if get_level() <= LogLevel::Trace {
        print(format_args!("[TRACE] {message}\n"));
    }
}

pub fn debug(message: &str) {
    if get_level() <= LogLevel::Debug {
        print(format_args!("[DEBUG] {message}\n"));
    }
}

pub fn info(message: &str) {
    if get_level() <= LogLevel::Info {
        print(format_args!("[KERNEL] {message}\n"));
    }
}

pub fn warn(message: &str) {
    if get_level() <= LogLevel::Warn {
        print(format_args!("[WARN] {message}\n"));
    }
}

pub fn error(message: &str) {
    if get_level() <= LogLevel::Error {
        print(format_args!("[ERR] {message}\n"));
    }
}

/// Parse log level from boot arguments (e.g., "log=debug")
pub fn parse_boot_arg(arg: &str) {
    if let Some(level_str) = arg.strip_prefix("log=") {
        match level_str.to_lowercase().as_str() {
            "trace" => set_level(LogLevel::Trace),
            "debug" => set_level(LogLevel::Debug),
            "info" => set_level(LogLevel::Info),
            "warn" => set_level(LogLevel::Warn),
            "error" => set_level(LogLevel::Error),
            "off" => set_level(LogLevel::Off),
            _ => {}
        }
    }
}
