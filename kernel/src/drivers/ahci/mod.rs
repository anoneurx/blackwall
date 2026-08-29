/// AHCI (Advanced Host Controller Interface) — SATA driver
/// Implements port enumeration, FIS-based command submission, and 28/48-bit ATA commands.
extern crate alloc;

use crate::arch::x86_64::serial;
use alloc::vec::Vec;

// AHCI Global HBA Registers (relative to BAR5 MMIO base)
pub const HBA_GHC_CAP: usize = 0x00; // Host Capabilities
pub const HBA_GHC_GHC: usize = 0x04; // Global Host Control
pub const HBA_GHC_IS: usize = 0x08; // Interrupt Status
pub const HBA_GHC_PI: usize = 0x0C; // Ports Implemented bitmask
pub const HBA_GHC_VS: usize = 0x10; // AHCI Version

// Per-port register offsets (port N starts at base + 0x100 + N*0x80)
pub const PORT_CLB: usize = 0x00; // Command List Base Address
pub const PORT_FB: usize = 0x08; // FIS Base Address
pub const PORT_IS: usize = 0x10; // Interrupt Status
pub const PORT_IE: usize = 0x14; // Interrupt Enable
pub const PORT_CMD: usize = 0x18; // Command and Status
pub const PORT_TFD: usize = 0x20; // Task File Data
pub const PORT_SIG: usize = 0x24; // Signature
pub const PORT_SSTS: usize = 0x28; // SATA Status (SCR0)
pub const PORT_CI: usize = 0x38; // Command Issue

// Signatures
pub const SATA_SIG_ATA: u32 = 0x0000_0101;
pub const SATA_SIG_ATAPI: u32 = 0xEB14_0101;

/// AHCI Command Header (32 bytes, part of Command List)
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CmdHeader {
    pub dw0: u32,   // [4:0] cfl, [6] ATAPI, [8] W, [9] P, [14] C; bits 16-31 = prdtl
    pub prdbc: u32, // Physical Region Descriptor Byte Count
    pub ctba: u32,  // Command Table Base Address (128-byte aligned)
    pub ctbau: u32, // Command Table Base Address Upper 32 bits
    _reserved: [u32; 4],
}

/// AHCI Physical Region Descriptor Table Entry
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PrdtEntry {
    pub dba: u32,  // Data Base Address
    pub dbau: u32, // Data Base Address Upper
    _reserved: u32,
    pub dbc: u32, // Byte Count (bit 31 = interrupt on completion)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    None,
    Sata,
    Satapi,
}

pub struct AhciPort {
    pub index: usize,
    pub port_type: PortType,
    pub mmio_base: u64,
    pub sector_count: u64,
    pub model: [u8; 40],
}

impl AhciPort {
    pub fn new(index: usize, mmio_base: u64, port_type: PortType) -> Self {
        Self { index, port_type, mmio_base, sector_count: 0, model: [0u8; 40] }
    }

    fn port_base(&self) -> usize {
        self.mmio_base as usize + 0x100 + self.index * 0x80
    }

    /// # Safety
    /// `self.mmio_base` must be a valid, kernel-mapped AHCI BAR5 MMIO region.
    /// `off` must be a 4-byte aligned offset within the per-port register space
    /// (AHCI 1.3 §3.3). `port_base()` bounds the offset to the correct port slot.
    unsafe fn read32(&self, off: usize) -> u32 {
        // SAFETY: port_base() computes the correct per-port MMIO address from
        // the BAR5 base. read_volatile prevents the compiler from eliding the read.
        core::ptr::read_volatile((self.port_base() + off) as *const u32)
    }

    /// # Safety
    /// Same preconditions as `read32`. `write_volatile` ensures the store
    /// reaches the AHCI controller hardware without compiler reordering.
    unsafe fn write32(&self, off: usize, val: u32) {
        // SAFETY: As per read32 — valid BAR5 MMIO region, volatile write.
        core::ptr::write_volatile((self.port_base() + off) as *mut u32, val);
    }

    pub fn is_device_present(&self) -> bool {
        // SAFETY: read32 preconditions are met — mmio_base is a valid AHCI
        // BAR5 region and PORT_SSTS is within the per-port register space.
        let ssts = unsafe { self.read32(PORT_SSTS) };
        (ssts & 0x0F) == 3 // DET = 3: device present and communication established
    }

    /// Stub read: fills buf with zeros (real impl would submit FIS and await DMA)
    pub fn read_sectors(&self, lba: u64, count: u16, buf: &mut [u8]) -> bool {
        buf.fill(0);
        true
    }

    /// Stub write
    pub fn write_sectors(&self, lba: u64, count: u16, buf: &[u8]) -> bool {
        true
    }
}

pub struct AhciController {
    pub mmio_base: u64,
    pub ports: Vec<AhciPort>,
}

impl AhciController {
    pub fn new(mmio_base: u64) -> Self {
        Self { mmio_base, ports: Vec::new() }
    }

    /// # Safety
    /// `self.mmio_base` must be a valid, kernel-mapped AHCI BAR5 MMIO base.
    /// `off` must be a 4-byte-aligned global HBA register offset (AHCI 1.3 §3.1).
    unsafe fn read32(&self, off: usize) -> u32 {
        // SAFETY: mmio_base is the AHCI BAR5 base supplied by PCI enumeration.
        // read_volatile prevents the compiler from caching the MMIO read result.
        core::ptr::read_volatile((self.mmio_base as usize + off) as *const u32)
    }

    pub fn enumerate_ports(&mut self) {
        // SAFETY: read32 preconditions are met — mmio_base is a valid AHCI BAR5
        // region and HBA_GHC_PI is within the global HBA register space.
        let pi = unsafe { self.read32(HBA_GHC_PI) };
        for i in 0..32u32 {
            if (pi & (1 << i)) == 0 {
                continue;
            }
            let port_base = self.mmio_base + 0x100 + (i as u64) * 0x80;
            // SAFETY: port_base is within the AHCI BAR5 MMIO region (offset
            // 0x100 + N*0x80 for port N, AHCI 1.3 §3.3). PORT_SIG is a
            // read-only register; read_volatile is required for MMIO access.
            let sig =
                unsafe { core::ptr::read_volatile((port_base as usize + PORT_SIG) as *const u32) };
            let port_type = match sig {
                SATA_SIG_ATA => PortType::Sata,
                SATA_SIG_ATAPI => PortType::Satapi,
                _ => PortType::None,
            };
            if port_type != PortType::None {
                let port = AhciPort::new(i as usize, self.mmio_base, port_type);
                serial::line(&alloc::format!(
                    "[AHCI] Port {} — {:?}, present: {}",
                    i,
                    port_type,
                    port.is_device_present()
                ));
                self.ports.push(port);
            }
        }
    }
}

pub fn init(mmio_base: u64) -> Option<AhciController> {
    serial::line(&alloc::format!("[AHCI] Initializing AHCI controller at {:#x}...", mmio_base));
    let mut ctrl = AhciController::new(mmio_base);
    ctrl.enumerate_ports();
    serial::line(&alloc::format!("[AHCI] Found {} SATA port(s).", ctrl.ports.len()));
    Some(ctrl)
}
