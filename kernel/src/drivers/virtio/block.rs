/// VirtIO Block Device driver
/// Reads and writes 512-byte sectors via virtqueue requests.
extern crate alloc;

use super::{VirtqDesc, VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE};
use crate::arch::x86_64::serial;
use alloc::boxed::Box;
use alloc::vec::Vec;

/// VirtIO block request types
pub const VIRTIO_BLK_T_IN: u32 = 0; // Read
pub const VIRTIO_BLK_T_OUT: u32 = 1; // Write
pub const VIRTIO_BLK_T_FLUSH: u32 = 4;

pub const VIRTIO_BLK_S_OK: u8 = 0;
pub const VIRTIO_BLK_S_IOERR: u8 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;

#[repr(C)]
pub struct BlkReqHeader {
    pub req_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

pub struct VirtioBlock {
    pub device_id: u32,
    pub capacity: u64, // in 512-byte sectors
    pub io_base: u32,  // legacy I/O port base (for I/O port BAR0)
}

impl VirtioBlock {
    pub fn new(io_base: u32, capacity: u64) -> Self {
        serial::line(&alloc::format!(
            "[VIRTIO-BLK] Initialized block device: {capacity} sectors ({} MiB)",
            capacity / 2048
        ));
        Self { device_id: 0, capacity, io_base }
    }

    /// Read a 512-byte sector from the block device.
    /// In a real driver this submits a VirtIO blk request descriptor chain
    /// and polls the used ring. Here we stub to zero-fill the buffer.
    pub fn read_sector(&self, lba: u64, buf: &mut [u8; 512]) -> bool {
        if lba >= self.capacity {
            serial::line("[VIRTIO-BLK] read_sector: LBA out of range");
            return false;
        }
        // Stub: fill with 0 (real implementation would use DMA descriptor chain)
        buf.fill(0);
        true
    }

    /// Write a 512-byte sector to the block device.
    pub fn write_sector(&self, lba: u64, buf: &[u8; 512]) -> bool {
        if lba >= self.capacity {
            serial::line("[VIRTIO-BLK] write_sector: LBA out of range");
            return false;
        }
        // Stub: real implementation submits VIRTIO_BLK_T_OUT descriptor chain
        true
    }

    /// Issue a FLUSH command (write-back any device caches)
    pub fn flush(&self) {
        serial::line("[VIRTIO-BLK] Flush issued (stub).");
    }
}

pub fn init(io_base: u32) -> Option<VirtioBlock> {
    serial::line(&alloc::format!("[VIRTIO-BLK] Initializing at I/O base {:#x}...", io_base));
    // Negotiate features: detect capacity from the device configuration space
    // In real hardware: read capacity from I/O port (io_base + 0x14..0x1B)
    // Stub: 1 GiB virtual disk
    let capacity: u64 = 2 * 1024 * 1024; // 1 GiB in 512-byte sectors
    Some(VirtioBlock::new(io_base, capacity))
}
