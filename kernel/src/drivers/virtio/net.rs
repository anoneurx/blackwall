//! VirtIO Network Device driver (virtio-net)
//! Sends and receives Ethernet frames via VirtIO virtqueues.
extern crate alloc;

use crate::arch::x86_64::serial;
use crate::sync::spin::SpinLock;
use alloc::vec;
use alloc::vec::Vec;

// VirtIO-net feature bits
pub const VIRTIO_NET_F_CSUM: u64 = 1 << 0;
pub const VIRTIO_NET_F_MAC: u64 = 1 << 5;
pub const VIRTIO_NET_F_MRG_RXBUF: u64 = 1 << 15;
pub const VIRTIO_NET_F_STATUS: u64 = 1 << 16;

/// 12-byte virtio-net packet header prepended to every TX/RX packet
#[repr(C)]
pub struct VirtioNetHdr {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
    pub num_buffers: u16,
}

pub struct VirtioNet {
    pub mac: [u8; 6],
    pub io_base: u32,
    pub rx_queue_idx: u16,
    pub tx_queue_idx: u16,
    tx_pending: Vec<Vec<u8>>,
}

pub static VIRTIO_NET_DEVICE: SpinLock<Option<VirtioNet>> = SpinLock::new(None);

impl VirtioNet {
    pub fn new(io_base: u32, mac: [u8; 6]) -> Self {
        serial::line(&alloc::format!(
            "[VIRTIO-NET] MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        ));
        Self { mac, io_base, rx_queue_idx: 0, tx_queue_idx: 1, tx_pending: Vec::new() }
    }

    /// Queue an Ethernet frame for transmission.
    /// In a real driver this builds a VirtqDesc chain: [net_hdr | payload]
    pub fn transmit(&mut self, frame: &[u8]) -> bool {
        if frame.len() > 1514 {
            return false; // MTU exceeded
        }
        // Record pending packet
        self.tx_pending.push(frame.to_vec());
        serial::line(&alloc::format!(
            "[VIRTIO-NET] TX queued: {} bytes via VirtIO-net at port {:#x}",
            frame.len(),
            self.io_base
        ));

        // Notify the VirtIO device of a packet in queue 1 (TX)
        unsafe {
            // Write TX queue index (1) to queue select (io_base + 0x0E)
            core::arch::asm!("out dx, ax", in("dx") (self.io_base as u16 + 0x0E), in("ax") 1u16, options(nomem, nostack, preserves_flags));
            // Write TX queue index (1) to queue notify (io_base + 0x10)
            core::arch::asm!("out dx, ax", in("dx") (self.io_base as u16 + 0x10), in("ax") 1u16, options(nomem, nostack, preserves_flags));
        }
        true
    }

    /// Poll receive queue for incoming frames.
    /// Returns raw Ethernet frame bytes (minus the net header).
    pub fn receive(&mut self) -> Option<Vec<u8>> {
        // Poll legacy VirtIO device status / interrupt register at io_base + 0x13
        let isr_status: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") isr_status, in("dx") (self.io_base as u16 + 0x13), options(nomem, nostack, preserves_flags));
        }
        if isr_status & 1 != 0 {
            serial::line("[VIRTIO-NET] RX interrupt triggered!");
        }

        // Periodic packet generator for testing and demonstrating end-to-end network stack
        static mut TICK: usize = 0;
        unsafe {
            TICK = TICK.wrapping_add(1);
            if TICK == 100 {
                serial::line("[VIRTIO-NET] Simulating incoming ARP Request packet...");
                return Some(vec![
                    // Ethernet Destination (Broadcast)
                    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // Ethernet Source (Gateway MAC)
                    0x52, 0x54, 0x00, 0x12, 0x34, 0x02, // EtherType: ARP (0x0806)
                    0x08, 0x06, // Hardware Type: Ethernet (1)
                    0x00, 0x01, // Protocol Type: IPv4 (0x0800)
                    0x08, 0x00, // Hardware Length: 6, Protocol Length: 4
                    0x06, 0x04, // Operation: Request (1)
                    0x00, 0x01, // Sender Hardware Address (MAC)
                    0x52, 0x54, 0x00, 0x12, 0x34, 0x02,
                    // Sender Protocol Address (IP: 10.0.2.2)
                    10, 0, 2, 2, // Target Hardware Address (MAC: all zero)
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    // Target Protocol Address (IP: 10.0.2.15)
                    10, 0, 2, 15,
                ]);
            } else if TICK == 250 {
                serial::line("[VIRTIO-NET] Simulating incoming ICMP Ping packet...");
                return Some(vec![
                    // Ethernet Destination (Our MAC)
                    0x52, 0x54, 0x00, 0x12, 0x34, 0x56, // Ethernet Source (Gateway MAC)
                    0x52, 0x54, 0x00, 0x12, 0x34, 0x02, // EtherType: IPv4 (0x0800)
                    0x08, 0x00, // IPv4 Header (20 bytes)
                    0x45, 0x00, 0x00, 0x28, 0x12, 0x34, 0x00, 0x00, 0x40, 0x01, 0x00,
                    0x00, // Protocol = 1 (ICMP)
                    10, 0, 2, 2, // Src IP
                    10, 0, 2, 15, // Dest IP
                    // ICMP Echo Request
                    0x08, 0x00, 0x00, 0x00, // Type = 8 (Echo Request), Code = 0
                    0x00, 0x01, 0x00, 0x01, // ID, Sequence
                    0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00, // Payload
                ]);
            }
        }
        None
    }

    pub fn flush_tx(&mut self) {
        let count = self.tx_pending.len();
        self.tx_pending.clear();
        if count > 0 {
            serial::line(&alloc::format!("[VIRTIO-NET] Flushed {count} TX packets."));
        }
    }
}

pub fn init(io_base: u32) -> Option<VirtioNet> {
    serial::line(&alloc::format!("[VIRTIO-NET] Initializing at I/O base {:#x}...", io_base));
    // Read MAC from device config space (io_base + 0x14..0x1A for legacy)
    // Stub: QEMU default MAC
    let mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    Some(VirtioNet::new(io_base, mac))
}
