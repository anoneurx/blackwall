/// VirtIO transport layer
/// Implements the PCI-transport virtqueue infrastructure shared by all VirtIO devices.
extern crate alloc;

pub mod block;
pub mod net;

use crate::arch::x86_64::serial;
use crate::drivers::pci::{PciBar, PciDevice};
use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};

// VirtIO PCI capability types
pub const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

// VirtIO device status flags
pub const ACKNOWLEDGE: u8 = 1;
pub const DRIVER: u8 = 2;
pub const DRIVER_OK: u8 = 4;
pub const FEATURES_OK: u8 = 8;
pub const FAILED: u8 = 128;

/// Virtqueue descriptor flags
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
pub const VIRTQ_DESC_F_WRITE: u16 = 2; // device writes to this descriptor
pub const VIRTQ_DESC_F_INDIRECT: u16 = 4;

/// Virtqueue descriptor table entry (16 bytes each)
#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// Virtqueue available ring
#[repr(C)]
pub struct VirtqAvail {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; 256],
}

/// Virtqueue used ring element
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

/// Virtqueue used ring
#[repr(C)]
pub struct VirtqUsed {
    pub flags: u16,
    pub idx: u16,
    pub ring: [VirtqUsedElem; 256],
}

pub const QUEUE_SIZE: usize = 256;

/// A single split virtqueue
pub struct Virtqueue {
    pub desc: &'static mut [VirtqDesc; QUEUE_SIZE],
    pub avail: &'static mut VirtqAvail,
    pub used: &'static mut VirtqUsed,
    pub free_head: u16,
    pub last_used: u16,
}

impl Virtqueue {
    /// Allocate a new descriptor. Returns descriptor index.
    pub fn alloc_desc(&mut self) -> Option<u16> {
        let idx = self.free_head;
        if idx >= QUEUE_SIZE as u16 {
            return None;
        }
        // Walk the free list (simple linear scan for now)
        self.free_head = idx + 1;
        Some(idx)
    }

    /// Add a buffer to the available ring and notify the device.
    pub fn add_buffer(&mut self, desc_idx: u16) {
        let avail_idx = (self.avail.idx as usize) % QUEUE_SIZE;
        self.avail.ring[avail_idx] = desc_idx;
        fence(Ordering::SeqCst);
        self.avail.idx = self.avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);
    }

    /// Check if the device has completed any buffers.
    pub fn poll_used(&mut self) -> Option<(u16, u32)> {
        if self.used.idx == self.last_used {
            return None;
        }
        let elem = self.used.ring[(self.last_used as usize) % QUEUE_SIZE];
        self.last_used = self.last_used.wrapping_add(1);
        Some((elem.id as u16, elem.len))
    }
}

pub fn probe(dev: &PciDevice) -> bool {
    if dev.vendor_id != 0x1AF4 {
        return false;
    }
    match dev.device_id {
        0x1000..=0x103F | 0x1040..=0x107F => true,
        _ => false,
    }
}
