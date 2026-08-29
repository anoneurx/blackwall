/// PS/2 Keyboard and Mouse driver
/// Communicates via I/O ports 0x60 (data) and 0x64 (command/status).
extern crate alloc;

use crate::arch::x86_64::serial;
use crate::sync::spin::SpinLock;
use alloc::collections::VecDeque;
use core::arch::asm;

const PS2_DATA: u16 = 0x60;
const PS2_STATUS: u16 = 0x64;
const PS2_CMD: u16 = 0x64;

const PS2_OUTPUT_FULL: u8 = 0x01;
const PS2_INPUT_FULL: u8 = 0x02;

// PS/2 controller commands
const CMD_DISABLE_PORT1: u8 = 0xAD;
const CMD_ENABLE_PORT1: u8 = 0xAE;
const CMD_DISABLE_PORT2: u8 = 0xA7;
const CMD_ENABLE_PORT2: u8 = 0xA8;
const CMD_TEST_PS2: u8 = 0xAA;
const CMD_TEST_PORT1: u8 = 0xAB;
const CMD_READ_CFG: u8 = 0x20;
const CMD_WRITE_CFG: u8 = 0x60;

// Keyboard commands
const KBD_RESET: u8 = 0xFF;
const KBD_ACK: u8 = 0xFA;
const KBD_SET_LEDS: u8 = 0xED;
const KBD_SET_SCANCODE: u8 = 0xF0;
const KBD_ENABLE: u8 = 0xF4;

/// Decoded key event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed(u8), // ASCII character
    Released(u8),
    Special(SpecialKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Escape,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    CapsLock,
    NumLock,
    ScrollLock,
    Enter,
    Backspace,
    Tab,
}

/// Scan code set 1 → ASCII (unshifted)
static SCANCODE_MAP: [u8; 128] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', // 0x00-0x09
    b'9', b'0', b'-', b'=', 8, 9, b'q', b'w', b'e', b'r', // 0x0A-0x13
    b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', 13, 0, // 0x14-0x1D
    b'a', b's', b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';', // 0x1E-0x27
    b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v', b'b', b'n', // 0x28-0x31
    b'm', b',', b'.', b'/', 0, b'*', 0, b' ', 0, 0, // 0x32-0x3B
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x3C-0x45
    0, 0, 0, 0, b'-', 0, 0, 0, b'+', 0, // 0x46-0x4F
    0, 0, 0, 127, 0, 0, 0, 0, 0, 0, // 0x50-0x59
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x5A-0x63
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x64-0x6D
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x6E-0x77
    0, 0, 0, 0, 0, 0, 0, 0, // 0x78-0x7F
];

pub static KEY_QUEUE: SpinLock<VecDeque<KeyEvent>> = SpinLock::new(VecDeque::new());

/// Read one byte from an x86 I/O port.
///
/// # Safety
/// Caller must supply a valid, ring-0-accessible I/O port address.
unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    // SAFETY: I/O port reads are CPL=0 only; we are always in kernel mode here.
    asm!("in al, dx", in("dx") port, out("al") v, options(nostack, preserves_flags));
    v
}
/// Write one byte to an x86 I/O port.
///
/// # Safety
/// Caller must supply a valid, ring-0-accessible I/O port address.
unsafe fn outb(port: u16, val: u8) {
    // SAFETY: I/O port writes are CPL=0 only; we are always in kernel mode here.
    asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
}

fn wait_write() {
    // SAFETY: Reads PS/2 status port (0x64) in a spin loop; bounded by
    // hardware protocol — the PS/2 controller always clears INPUT_FULL after
    // accepting a byte. Port 0x64 is the standard PS/2 status register.
    unsafe { while (inb(PS2_STATUS) & PS2_INPUT_FULL) != 0 {} }
}
fn wait_read() -> bool {
    let mut retries = 0;
    // SAFETY: Same as wait_write; reads PS/2 status register. The retry limit
    // (100,000) prevents infinite spin if the controller is absent or faulty.
    unsafe {
        while (inb(PS2_STATUS) & PS2_OUTPUT_FULL) == 0 {
            retries += 1;
            if retries > 100_000 {
                return false;
            }
        }
    }
    true
}

fn send_cmd(cmd: u8) {
    wait_write();
    // SAFETY: PS2_CMD (0x64) is the PS/2 controller command port; valid after wait_write.
    unsafe {
        outb(PS2_CMD, cmd);
    }
}
fn send_data(data: u8) {
    wait_write();
    // SAFETY: PS2_DATA (0x60) is the PS/2 data port; valid after wait_write.
    unsafe {
        outb(PS2_DATA, data);
    }
}
fn recv_data() -> u8 {
    // SAFETY: Reads PS2_DATA (0x60) only after wait_read confirms OUTPUT_FULL=1,
    // guaranteeing a byte is waiting in the output buffer.
    if wait_read() {
        unsafe { inb(PS2_DATA) }
    } else {
        0
    }
}

/// Call this from the IRQ 1 (keyboard) handler.
pub fn handle_irq() {
    // SAFETY: IRQ 1 guarantees the PS/2 output buffer is full (OUTPUT_FULL=1),
    // so reading PS2_DATA (0x60) is valid and returns the pending scancode.
    let scancode = unsafe { inb(PS2_DATA) };
    let released = (scancode & 0x80) != 0;
    let code = scancode & 0x7F;
    let ch = if (code as usize) < SCANCODE_MAP.len() { SCANCODE_MAP[code as usize] } else { 0 };
    if ch != 0 {
        let ev = if released { KeyEvent::Released(ch) } else { KeyEvent::Pressed(ch) };
        KEY_QUEUE.lock().push_back(ev);
    }
}

/// Non-blocking key read. Returns `Some(char)` if a key was pressed.
pub fn read_key() -> Option<char> {
    loop {
        let ev = KEY_QUEUE.lock().pop_front()?;
        if let KeyEvent::Pressed(ch) = ev {
            return char::from_u32(ch as u32);
        }
    }
}

pub fn init() {
    serial::line("[PS/2] Initializing PS/2 controller...");
    // SAFETY: All I/O port accesses below target the standard PS/2 controller
    // registers (0x60 data, 0x64 command/status). The initialization sequence
    // follows the OSDev PS/2 controller init protocol. All writes are preceded
    // by wait_write() to ensure the input buffer is empty, preventing dropped
    // commands. All reads from the data port are preceded by wait_read().
    unsafe {
        // Disable both PS/2 ports
        send_cmd(CMD_DISABLE_PORT1);
        send_cmd(CMD_DISABLE_PORT2);
        // Flush output buffer
        while (inb(PS2_STATUS) & PS2_OUTPUT_FULL) != 0 {
            let _ = inb(PS2_DATA);
        }
        // Configure: disable IRQs and translation initially
        send_cmd(CMD_READ_CFG);
        let mut cfg = recv_data();
        cfg &= !(0x01 | 0x02 | 0x40); // disable IRQ1, IRQ12, scancode translation
        send_cmd(CMD_WRITE_CFG);
        send_data(cfg);
        // Test PS/2 controller
        send_cmd(CMD_TEST_PS2);
        let result = recv_data();
        if result != 0x55 {
            serial::line("[PS/2] Controller self-test FAILED.");
            return;
        }
        // Test port 1
        send_cmd(CMD_TEST_PORT1);
        // Enable port 1 and IRQ1
        send_cmd(CMD_ENABLE_PORT1);
        cfg |= 0x01 | 0x40; // enable IRQ1 + scancode translation
        send_cmd(CMD_WRITE_CFG);
        send_data(cfg);
        // Reset and enable keyboard
        send_data(KBD_RESET);
        let _ = recv_data(); // ACK
        let _ = recv_data(); // 0xAA (self-test passed)
        send_data(KBD_ENABLE);
        let _ = recv_data(); // ACK
    }
    serial::line("[PS/2] Keyboard initialized. Scancode set 1 active.");
}
