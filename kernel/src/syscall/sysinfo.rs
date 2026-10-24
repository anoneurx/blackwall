//! `SYS_SYSINFO` — machine information + power-down control plane.
//!
//! All operations are selected by `a0`:
//! - `SYSINFO_OP_TIME`      → `rax` = Unix epoch seconds (UTC), from the RTC
//! - `SYSINFO_OP_UPTIME`    → `rax` = uptime in whole seconds (PIT ticks/100)
//! - `SYSINFO_OP_PCI`       → packed device records into `[a1, a1+a2)`;
//!   returns the device count in `rax`
//! - `SYSINFO_OP_POWERDOWN` → signals QEMU's `isa-debug-exit` device; a
//!   harmless I/O write on real hardware
//!
//! Every user-pointer write goes through the same `access_ok` gate the other
//! syscalls use.

use crate::memory::user_access::access_ok;
use alloc::vec::Vec;
use blackwall_shared::syscall::*;

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm),
/// valid for years far outside 1970–2100 for formatting stability.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32; // [0, 399]
    let doy = ((153 * (if m > 2 { m - 3 } else { m + 9 }) as i64 + 2) / 5 + d as i64 - 1) as u32;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

fn epoch_seconds_utc() -> u64 {
    let dt = crate::drivers::rtc::read_datetime();
    let days = days_from_civil(dt.year as i64, dt.month as u32, dt.day as u32);
    let secs = days * 86400 + dt.hour as i64 * 3600 + dt.minute as i64 * 60 + dt.second as i64;
    secs.max(0) as u64
}

pub fn handle(op: u64, a1: u64, a2: u64) -> u64 {
    match op {
        SYSINFO_OP_TIME => epoch_seconds_utc(),
        SYSINFO_OP_UPTIME => crate::arch::x86_64::timer::ticks() / 100,
        SYSINFO_OP_PCI => pci_list(a1, a2),
        SYSINFO_OP_POWERDOWN => {
            powerdown();
            0
        }
        _ => EINVAL as u64,
    }
}

/// Pack the cached PCI device list into `[a1, a1+a2)` and return the count.
fn pci_list(buf_ptr: u64, len: u64) -> u64 {
    if !access_ok(buf_ptr, len, true) {
        return EFAULT as u64;
    }
    let devices: Vec<crate::drivers::pci::PciDevice> = crate::drivers::pci::cached_devices();
    // SAFETY: access_ok proved `[buf_ptr, buf_ptr+len)` is a mapped, writable
    // user range for the duration of this call.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    let mut written = 0usize;
    let mut count = 0u64;
    for dev in &devices {
        if written + 10 > buf.len() {
            break;
        }
        buf[written] = dev.bus;
        buf[written + 1] = dev.slot;
        buf[written + 2] = dev.func;
        buf[written + 3] = (dev.vendor_id & 0xFF) as u8;
        buf[written + 4] = (dev.vendor_id >> 8) as u8;
        buf[written + 5] = (dev.device_id & 0xFF) as u8;
        buf[written + 6] = (dev.device_id >> 8) as u8;
        buf[written + 7] = dev.class_code;
        buf[written + 8] = dev.subclass;
        buf[written + 9] = dev.prog_if;
        written += 10;
        count += 1;
    }
    count
}

/// QEMU's `isa-debug-exit` device at `iobase=0xf4` terminates the VM with the
/// value written & 0xff (0 = clean test exit). On real hardware the port
/// write is a no-op, so this is safe to issue unconditionally.
fn powerdown() {
    // SAFETY: 0xf4 is the isa-debug-exit I/O port registered in the QEMU
    // test command line; on missing devices the write sinks nowhere.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0xf4u16, in("al") 0u8, options(nostack, preserves_flags))
    };
}
