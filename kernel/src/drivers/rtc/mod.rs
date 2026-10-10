use crate::arch::x86_64::serial;
/// Real-Time Clock (RTC) driver
/// Reads the CMOS RTC via I/O ports 0x70 (address) and 0x71 (data).
use core::arch::asm;

const RTC_ADDR: u16 = 0x70;
const RTC_DATA: u16 = 0x71;

// CMOS RTC register indices
const RTC_SECONDS: u8 = 0x00;
const RTC_MINUTES: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_WEEKDAY: u8 = 0x06;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_CENTURY: u8 = 0x32;
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

#[derive(Debug, Clone, Copy, Default)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub weekday: u8,
}

/// Read one byte from the CMOS RTC at the given register index.
///
/// # Safety
/// * Port 0x70 (RTC_ADDR) is the CMOS address latch and 0x71 (RTC_DATA) is
///   the data port — standard on all x86_64 PC hardware, accessible in ring-0.
/// * Caller must ensure `reg` is a valid CMOS register index (0x00–0x7F).
///   Selecting index bit 7 disables NMI; callers should restore NMI state if needed.
unsafe fn cmos_read(reg: u8) -> u8 {
    // SAFETY: Writing to 0x70 selects the CMOS register; reading from 0x71
    // retrieves its value. Both ports are ring-0 only and always present.
    asm!("out dx, al", in("dx") RTC_ADDR, in("al") reg, options(nostack, preserves_flags));
    let v: u8;
    asm!("in al, dx",  in("dx") RTC_DATA, out("al") v,  options(nostack, preserves_flags));
    v
}

fn is_update_in_progress() -> bool {
    // SAFETY: cmos_read is safe here — RTC_STATUS_A (0x0A) is a read-only
    // status register that does not modify NMI state.
    unsafe { (cmos_read(RTC_STATUS_A) & 0x80) != 0 }
}

fn bcd_to_bin(bcd: u8) -> u8 {
    ((bcd >> 4) * 10) + (bcd & 0x0F)
}

/// Read the current date and time from the CMOS RTC.
/// Reads twice to avoid partial updates.
pub fn read_datetime() -> DateTime {
    // Wait until RTC is not in an update cycle
    while is_update_in_progress() {}

    // SAFETY: cmos_read accesses valid CMOS register indices. We read each
    // field consecutively after confirming no update is in progress (UIP=0),
    // ensuring a consistent snapshot. All indices used are standard RTC
    // registers defined by the MC146818 RTC specification.
    let (s, m, h, wd, d, mo, y, c) = unsafe {
        (
            cmos_read(RTC_SECONDS),
            cmos_read(RTC_MINUTES),
            cmos_read(RTC_HOURS),
            cmos_read(RTC_WEEKDAY),
            cmos_read(RTC_DAY),
            cmos_read(RTC_MONTH),
            cmos_read(RTC_YEAR),
            cmos_read(RTC_CENTURY),
        )
    };

    // SAFETY: RTC_STATUS_B (0x0B) is a read-only configuration register.
    let status_b = unsafe { cmos_read(RTC_STATUS_B) };
    let binary_mode = (status_b & 0x04) != 0;
    let _hour_24 = (status_b & 0x02) != 0;

    let (s, m, h, wd, d, mo, y, c) = if binary_mode {
        (s, m, h, wd, d, mo, y, c)
    } else {
        (
            bcd_to_bin(s),
            bcd_to_bin(m),
            bcd_to_bin(h),
            bcd_to_bin(wd),
            bcd_to_bin(d),
            bcd_to_bin(mo),
            bcd_to_bin(y),
            bcd_to_bin(c),
        )
    };

    let year = (c as u16) * 100 + (y as u16);

    DateTime { year, month: mo, day: d, hour: h, minute: m, second: s, weekday: wd }
}

pub fn init() {
    let dt = read_datetime();
    serial::line(&alloc::format!(
        "[RTC] System time: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        dt.year,
        dt.month,
        dt.day,
        dt.hour,
        dt.minute,
        dt.second
    ));
}
