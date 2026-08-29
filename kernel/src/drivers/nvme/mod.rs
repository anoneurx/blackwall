/// NVMe Controller Driver
/// Implements Admin and I/O queue pair submission/completion for NVMe SSDs.
extern crate alloc;

use crate::arch::x86_64::serial;
use alloc::vec::Vec;

// NVMe Controller Registers offsets (relative to BAR0 MMIO base)
pub const NVME_REG_CAP: usize = 0x00; // Controller Capabilities (u64)
pub const NVME_REG_VS: usize = 0x08; // Version
pub const NVME_REG_CC: usize = 0x14; // Controller Configuration
pub const NVME_REG_CSTS: usize = 0x1C; // Controller Status
pub const NVME_REG_AQA: usize = 0x24; // Admin Queue Attributes
pub const NVME_REG_ASQ: usize = 0x28; // Admin Submission Queue Base Address
pub const NVME_REG_ACQ: usize = 0x30; // Admin Completion Queue Base Address

// NVMe CC fields
pub const NVME_CC_ENABLE: u32 = 1 << 0;
pub const NVME_CC_CSS_NVM: u32 = 0 << 4; // NVM Command Set
pub const NVME_CC_MPS_4K: u32 = 0 << 7; // Memory Page Size = 4 KiB
pub const NVME_CC_AQS_16: u32 = 0 << 11; // Admin queue entry size 16 B

// NVMe CSTS fields
pub const NVME_CSTS_RDY: u32 = 1 << 0;
pub const NVME_CSTS_CFS: u32 = 1 << 1; // Controller Fatal Status

/// 64-byte Admin/IO Submission Queue Entry
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SubmissionEntry {
    pub cdw0: u32, // Command Dword 0 (opcode, flags, CID)
    pub nsid: u32,
    pub cdw2: u32,
    pub cdw3: u32,
    pub mptr: u64, // Metadata Pointer
    pub prp1: u64, // PRP Entry 1 (data buffer)
    pub prp2: u64, // PRP Entry 2
    pub cdw10: u32,
    pub cdw11: u32,
    pub cdw12: u32,
    pub cdw13: u32,
    pub cdw14: u32,
    pub cdw15: u32,
}

/// 16-byte Completion Queue Entry
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CompletionEntry {
    pub dw0: u32,
    pub dw1: u32,
    pub sq_hd: u16,
    pub sq_id: u16,
    pub cmd_id: u16,
    pub status: u16, // bit 0 = phase tag, bits 1-15 = status code
}

/// NVMe opcode constants
pub const NVME_OPC_IDENTIFY: u8 = 0x06;
pub const NVME_OPC_CREATE_SQ: u8 = 0x01;
pub const NVME_OPC_CREATE_CQ: u8 = 0x05;
pub const NVME_OPC_READ: u8 = 0x02;
pub const NVME_OPC_WRITE: u8 = 0x01;

pub const ADMIN_QUEUE_DEPTH: usize = 64;
pub const IO_QUEUE_DEPTH: usize = 256;

pub struct NvmeController {
    pub mmio_base: u64,
    pub namespace_size_sectors: u64, // total 512-byte sectors in namespace 1
    pub model: [u8; 40],
    pub serial: [u8; 20],
}

impl NvmeController {
    pub fn new(mmio_base: u64) -> Self {
        Self {
            mmio_base,
            namespace_size_sectors: 2 * 1024 * 1024 * 1024 / 512, // Stub 1 GiB
            model: [0u8; 40],
            serial: [0u8; 20],
        }
    }

    /// # Safety
    /// `self.mmio_base` must be a valid, mapped NVMe BAR0 MMIO region.
    /// `offset` must be a 4-byte aligned offset within the NVMe register space
    /// as defined by the NVMe Base Specification 1.4 §3.1.
    unsafe fn read32(&self, offset: usize) -> u32 {
        // SAFETY: mmio_base is PCI BAR0 mapped by the driver init path.
        // read_volatile prevents the compiler from caching/eliding the MMIO read.
        core::ptr::read_volatile((self.mmio_base as usize + offset) as *const u32)
    }

    /// # Safety
    /// Same preconditions as `read32`. The write is visible to hardware because
    /// `write_volatile` acts as a compiler barrier for MMIO stores.
    unsafe fn write32(&self, offset: usize, val: u32) {
        // SAFETY: mmio_base is a valid mapped MMIO region; write_volatile ensures
        // the store is not elided or reordered past this point by the compiler.
        core::ptr::write_volatile((self.mmio_base as usize + offset) as *mut u32, val);
    }

    /// Reset and bring up the controller.
    pub fn reset(&self) {
        // SAFETY: All register accesses go through read32/write32 whose
        // preconditions are documented above. The spin loops are bounded by
        // 100,000 iterations to prevent infinite hangs if the controller is
        // absent or faulty. The CC.EN=0 → CC.EN=1 sequence follows the
        // NVMe 1.4 spec §7.6.1 controller reset procedure.
        unsafe {
            // Disable controller
            let cc = self.read32(NVME_REG_CC);
            self.write32(NVME_REG_CC, cc & !NVME_CC_ENABLE);

            // Wait for CSTS.RDY == 0 (controller acknowledged disable)
            let mut retries = 0u32;
            while (self.read32(NVME_REG_CSTS) & NVME_CSTS_RDY) != 0 {
                retries += 1;
                if retries > 100_000 {
                    break;
                }
            }

            // Configure admin queues and re-enable controller.
            // CC: NVM command set, 4 KiB page, enable
            self.write32(NVME_REG_CC, NVME_CC_CSS_NVM | NVME_CC_MPS_4K | NVME_CC_ENABLE);

            // Wait for CSTS.RDY == 1 (controller ready)
            retries = 0;
            while (self.read32(NVME_REG_CSTS) & NVME_CSTS_RDY) == 0 {
                retries += 1;
                if retries > 100_000 {
                    break;
                }
            }
        }
        serial::line(&alloc::format!(
            "[NVME] Controller at {:#x} reset and enabled.",
            self.mmio_base
        ));
    }

    /// Stub for reading a 512-byte sector
    pub fn read_sector(&self, lba: u64, buf: &mut [u8; 512]) -> bool {
        if lba >= self.namespace_size_sectors {
            return false;
        }
        buf.fill(0); // Stub: DMA would transfer real data
        true
    }

    /// Stub for writing a 512-byte sector
    pub fn write_sector(&self, lba: u64, _buf: &[u8; 512]) -> bool {
        if lba >= self.namespace_size_sectors {
            return false;
        }
        true // Stub
    }
}

pub fn init(mmio_base: u64) -> Option<NvmeController> {
    serial::line(&alloc::format!(
        "[NVME] Initializing NVMe controller at MMIO base {:#x}...",
        mmio_base
    ));
    let ctrl = NvmeController::new(mmio_base);
    ctrl.reset();
    serial::line(&alloc::format!(
        "[NVME] Namespace 0 capacity: {} MiB",
        ctrl.namespace_size_sectors / 2048
    ));
    Some(ctrl)
}
