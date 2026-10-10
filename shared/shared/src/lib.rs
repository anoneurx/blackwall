#![cfg_attr(not(test), no_std)]

pub mod messages {
    pub const BOOT_BANNER: &str = "Black Wall Core v1.0";
    pub const KERNEL_STARTING: &str = "Kernel Starting...";
    pub const CPU_INITIALIZED: &str = "CPU Initialized";
    pub const KERNEL_INITIALIZED: &str = "Kernel Initialized";
    pub const CPU_DETECTED: &str = "CPU Detected";
    pub const MEMORY_INITIALIZED: &str = "Memory Initialized";
    pub const INTERRUPTS_LOADED: &str = "Interrupts Loaded";
    pub const TIMER_RUNNING: &str = "Timer Running";
    pub const PHYSICAL_MEMORY_MANAGER_READY: &str = "Physical Memory Manager Ready";
    pub const VIRTUAL_MEMORY_MANAGER_READY: &str = "Virtual Memory Manager Ready";
    pub const HEAP_INITIALIZED: &str = "Heap Initialized";
    pub const MEMORY_PROTECTION_ENABLED: &str = "Memory Protection Enabled";
    pub const SYSTEM_READY_PHASE_3: &str = "System Ready For Phase 3";
    pub const MEMORY_MANAGER_READY: &str = "Memory Manager Ready";
    pub const SCHEDULER_READY: &str = "Scheduler Ready";
    pub const KERNEL_THREADS_ENABLED: &str = "Kernel Threads Enabled";
    pub const MULTITASKING_ENABLED: &str = "Multitasking Enabled";
    pub const SYSTEM_READY_PHASE_4: &str = "System Ready For Phase 4";
    pub const PID_0_IDLE: &str = "PID 0 idle";
    pub const PID_1_INIT: &str = "PID 1 init";
}

pub mod serial {
    use core::arch::asm;

    const COM1_PORT: u16 = 0x3f8;

    pub fn init() {
        // SAFETY: Configures the COM1 UART (0x3F8) by writing to its standard
        // register offsets. These I/O ports are always available on x86_64 PC
        // hardware in ring-0. The sequence follows the standard 16550A init
        // protocol: disable IRQs, set divisor latch, configure 8N1, enable FIFO.
        unsafe {
            outb(COM1_PORT + 1, 0x00);
            outb(COM1_PORT + 3, 0x80);
            outb(COM1_PORT + 0, 0x03);
            outb(COM1_PORT + 1, 0x00);
            outb(COM1_PORT + 3, 0x03);
            outb(COM1_PORT + 2, 0xc7);
            outb(COM1_PORT + 4, 0x0b);
        }
    }

    pub fn write_byte(byte: u8) {
        // SAFETY: Polls COM1 line-status register (offset +5) until the
        // transmit-holding register is empty (bit 5 set), then writes to the
        // transmit buffer (offset +0). Both ports are valid COM1 register addresses.
        unsafe {
            while (inb(COM1_PORT + 5) & 0x20) == 0 {}
            outb(COM1_PORT, byte);
        }
    }

    pub fn write_str(text: &str) {
        for byte in text.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
    }

    pub fn write_line(text: &str) {
        write_str(text);
        write_str("\n");
    }

    pub fn read_byte() -> Option<u8> {
        // SAFETY: Reads COM1 line-status register (offset +5) to check if data
        // is available (bit 0 set). If so, reads one byte from the receive
        // buffer (offset +0). Both are valid, accessible COM1 I/O port addresses.
        unsafe {
            if (inb(COM1_PORT + 5) & 0x01) != 0 {
                Some(inb(COM1_PORT))
            } else {
                None
            }
        }
    }

    #[inline(always)]
    unsafe fn outb(port: u16, value: u8) {
        // SAFETY: The caller ensures the port is a valid UART register address.
        asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
    }

    #[inline(always)]
    unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        // SAFETY: The caller ensures the port is a valid UART register address.
        asm!("in al, dx", in("dx") port, out("al") value, options(nostack, preserves_flags));
        value
    }
}

pub mod syscall {
    // Standard x86_64 System V Syscall Numbers
    pub const SYS_READ: u64 = 0;
    pub const SYS_WRITE: u64 = 1;
    pub const SYS_OPEN: u64 = 2;
    pub const SYS_CLOSE: u64 = 3;
    pub const SYS_YIELD: u64 = 24;
    pub const SYS_SLEEP: u64 = 35;
    pub const SYS_GETPID: u64 = 39;
    pub const SYS_FORK: u64 = 57;
    pub const SYS_EXECVE: u64 = 59;
    pub const SYS_EXIT: u64 = 60;
    pub const SYS_GETPPID: u64 = 110;

    // Error codes
    pub const ENOSYS: i64 = -38;
}
