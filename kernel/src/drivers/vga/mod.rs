use crate::sync::spin::SpinLock;
/// VGA Text Mode driver
/// Writes characters and colors directly to the VGA framebuffer at 0xB8000.
use core::fmt;

const VGA_BASE: usize = 0xB8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ColorPair(u8); // (bg << 4) | fg

impl ColorPair {
    pub const fn new(fg: Color, bg: Color) -> Self {
        Self((bg as u8) << 4 | fg as u8)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct VgaChar {
    ascii: u8,
    color: ColorPair,
}

impl VgaChar {
    fn to_u16(self) -> u16 {
        (self.color.0 as u16) << 8 | self.ascii as u16
    }
}

pub struct VgaWriter {
    col: usize,
    row: usize,
    color: ColorPair,
}

impl VgaWriter {
    pub const fn new() -> Self {
        Self { col: 0, row: 0, color: ColorPair::new(Color::White, Color::Black) }
    }

    pub fn set_color(&mut self, color: ColorPair) {
        self.color = color;
    }

    fn write_at(&self, row: usize, col: usize, ch: VgaChar) {
        let offset = row * VGA_WIDTH + col;
        // SAFETY: VGA_BASE (0xB8000) is the identity-mapped VGA text framebuffer
        // present on all x86_64 PC-compatible hardware. `offset` is bounded by
        // `row < VGA_HEIGHT` and `col < VGA_WIDTH` (enforced by callers), so
        // the write is within the 80×25×2 = 4000-byte buffer. `write_volatile`
        // prevents the compiler from eliding the store.
        unsafe {
            core::ptr::write_volatile((VGA_BASE + offset * 2) as *mut u16, ch.to_u16());
        }
    }

    pub fn clear(&mut self) {
        let blank = VgaChar { ascii: b' ', color: self.color };
        for row in 0..VGA_HEIGHT {
            for col in 0..VGA_WIDTH {
                self.write_at(row, col, blank);
            }
        }
        self.col = 0;
        self.row = 0;
    }

    fn scroll(&mut self) {
        // SAFETY: Both `dst` and `src` point into the identity-mapped VGA text
        // framebuffer (0xB8000..0xB8FA0). The copy moves `VGA_WIDTH * (VGA_HEIGHT-1)`
        // u16 cells upward by one row. Source and destination do not overlap
        // (src is VGA_WIDTH*2 bytes ahead of dst), so `copy` (memmove semantics)
        // is safe. The buffer is fully writable as a MMIO region.
        unsafe {
            let dst = VGA_BASE as *mut u16;
            let src = (VGA_BASE + VGA_WIDTH * 2) as *const u16;
            core::ptr::copy(src, dst, VGA_WIDTH * (VGA_HEIGHT - 1));
        }
        // Clear the last row
        let blank = VgaChar { ascii: b' ', color: self.color };
        for col in 0..VGA_WIDTH {
            self.write_at(VGA_HEIGHT - 1, col, blank);
        }
        self.row = VGA_HEIGHT - 1;
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.col = 0;
                self.row += 1;
                if self.row >= VGA_HEIGHT {
                    self.scroll();
                }
            }
            b'\r' => {
                self.col = 0;
            }
            b'\x08' => {
                // Backspace
                if self.col > 0 {
                    self.col -= 1;
                }
                self.write_at(self.row, self.col, VgaChar { ascii: b' ', color: self.color });
            }
            byte => {
                self.write_at(self.row, self.col, VgaChar { ascii: byte, color: self.color });
                self.col += 1;
                if self.col >= VGA_WIDTH {
                    self.col = 0;
                    self.row += 1;
                    if self.row >= VGA_HEIGHT {
                        self.scroll();
                    }
                }
            }
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

pub static VGA: SpinLock<VgaWriter> = SpinLock::new(VgaWriter::new());

pub fn init() {
    let mut w = VGA.lock();
    w.clear();
    w.set_color(ColorPair::new(Color::LightCyan, Color::Black));
    w.write_str("Black Wall Core v1.0\n");
    w.set_color(ColorPair::new(Color::White, Color::Black));
    w.write_str("VGA text mode driver active.\n");
}

/// Convenience macro — mirrors the print! concept via VGA
#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => {
        {
            use core::fmt::Write;
            let _ = write!($crate::drivers::vga::VGA.lock(), $($arg)*);
        }
    };
}
