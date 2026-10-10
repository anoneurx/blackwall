//! AHCI (SATA) driver.
//!
//! Implements port enumeration and real FIS-based DMA read/write against the
//! AHCI command engine. All DMA structures (command list, received-FIS area,
//! command table, bounce buffer) are heap-allocated and identity-mapped, so
//! their virtual addresses are usable as physical addresses for the controller.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::hint::spin_loop;

use crate::arch::x86_64::serial;
use crate::sync::spin::SpinLock;
use blackwall_fs::block::BlockDevice;
use blackwall_fs::vfs::VfsError;

// AHCI Global HBA Registers (relative to BAR5 MMIO base)
pub const HBA_CAP: usize = 0x00; // Host Capabilities
pub const HBA_GHC: usize = 0x04; // Global Host Control
pub const HBA_IS: usize = 0x08; // Interrupt Status
pub const HBA_PI: usize = 0x0C; // Ports Implemented bitmask
pub const HBA_VS: usize = 0x10; // AHCI Version

// Per-port register offsets (port N starts at base + 0x100 + N*0x80)
pub const PORT_CLB: usize = 0x00; // Command List Base Address
pub const PORT_CLBU: usize = 0x04; // Command List Base Address Upper
pub const PORT_FB: usize = 0x08; // FIS Base Address
pub const PORT_FBU: usize = 0x0C; // FIS Base Address Upper
pub const PORT_IS: usize = 0x10; // Interrupt Status
pub const PORT_IE: usize = 0x14; // Interrupt Enable
pub const PORT_CMD: usize = 0x18; // Command and Status
pub const PORT_TFD: usize = 0x20; // Task File Data
pub const PORT_SIG: usize = 0x24; // Signature
pub const PORT_SSTS: usize = 0x28; // SATA Status (SCR0)
pub const PORT_SERR: usize = 0x30; // SATA Error
pub const PORT_CI: usize = 0x38; // Command Issue

// Signatures
pub const SATA_SIG_ATA: u32 = 0x0000_0101;
pub const SATA_SIG_ATAPI: u32 = 0xEB14_0101;

// Global Host Control bits
const GHC_AE: u32 = 1 << 31;
// Port Command/Status bits
const CMD_ST: u32 = 1 << 0;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
// Task File Data error bits
const TFD_ERR: u32 = 1 << 0;
const TFD_DF: u32 = 1 << 5;
// Port Interrupt Status: Task File Error
const IS_TFES: u32 = 1 << 30;

const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

/// AHCI Command Header (32 bytes, part of Command List)
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CmdHeader {
    pub dw0: u32,   // [4:0] cfl, [6] W, [10] C; bits 16-31 = prdtl
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

/// DMA-visible buffers shared by all commands issued to one port.
///
/// Layout is fixed and 1 KiB aligned so the command list and command table
/// meet AHCI's alignment requirements.
#[repr(C, align(1024))]
struct AhciDmaBufs {
    cmd_list: [u8; 1024],
    recv_fis: [u8; 256],
    cmd_table: [u8; 4096],
    bounce: [u8; 16384],
}

pub struct AhciPort {
    pub index: usize,
    pub port_type: PortType,
    pub mmio_base: u64,
    pub sector_count: u64,
    pub model: [u8; 40],
    dma: Box<UnsafeCell<AhciDmaBufs>>,
    engaged: bool,
}

impl AhciPort {
    pub fn new(index: usize, mmio_base: u64, port_type: PortType) -> Self {
        // SAFETY: AhciDmaBufs is a plain array-of-bytes aggregate for which an
        // all-zero bit pattern is valid and is the required initial state.
        let bufs: AhciDmaBufs = unsafe { core::mem::zeroed() };
        Self {
            index,
            port_type,
            mmio_base,
            sector_count: 0,
            model: [0u8; 40],
            dma: Box::new(UnsafeCell::new(bufs)),
            engaged: false,
        }
    }

    fn port_base(&self) -> usize {
        self.mmio_base as usize + 0x100 + self.index * 0x80
    }

    fn dma(&self) -> *mut AhciDmaBufs {
        self.dma.get()
    }

    /// # Safety
    /// `self.mmio_base` must be a valid, kernel-mapped AHCI BAR5 MMIO region.
    unsafe fn read32(&self, off: usize) -> u32 {
        core::ptr::read_volatile((self.port_base() + off) as *const u32)
    }

    /// # Safety
    /// Same preconditions as `read32`.
    unsafe fn write32(&self, off: usize, val: u32) {
        core::ptr::write_volatile((self.port_base() + off) as *mut u32, val);
    }

    pub fn is_device_present(&self) -> bool {
        // SAFETY: read32 preconditions hold — mmio_base is a valid BAR5 region.
        let ssts = unsafe { self.read32(PORT_SSTS) };
        (ssts & 0x0F) == 3 // DET = 3: device present, communication established
    }

    fn clear_status(&self) {
        // SAFETY: write32 preconditions hold.
        unsafe {
            self.write32(PORT_IS, 0xFFFF_FFFF);
            self.write32(PORT_SERR, 0xFFFF_FFFF);
        }
    }

    /// Bring the port command engine up, pointing it at our DMA buffers.
    fn setup_engine(&mut self) {
        let (clb, fb) = {
            let d = self.dma();
            // SAFETY: `d` points to our heap DMA buffer, exclusively owned by
            // this port; we only read the array base addresses here.
            unsafe { ((*d).cmd_list.as_ptr() as u64, (*d).recv_fis.as_ptr() as u64) }
        };

        // SAFETY: MMIO writes to this port's command registers.
        unsafe {
            let mut cmd = self.read32(PORT_CMD);
            cmd &= !CMD_ST;
            self.write32(PORT_CMD, cmd);
            for _ in 0..1_000_000 {
                if self.read32(PORT_CMD) & CMD_CR == 0 {
                    break;
                }
                spin_loop();
            }
            cmd &= !CMD_FRE;
            self.write32(PORT_CMD, cmd);
            for _ in 0..1_000_000 {
                if self.read32(PORT_CMD) & CMD_FR == 0 {
                    break;
                }
                spin_loop();
            }

            self.write32(PORT_CLB, clb as u32);
            self.write32(PORT_CLBU, (clb >> 32) as u32);
            self.write32(PORT_FB, fb as u32);
            self.write32(PORT_FBU, (fb >> 32) as u32);
            self.write32(PORT_IS, 0xFFFF_FFFF);
            self.write32(PORT_SERR, 0xFFFF_FFFF);

            cmd = self.read32(PORT_CMD);
            cmd |= CMD_FRE;
            self.write32(PORT_CMD, cmd);
            for _ in 0..1_000_000 {
                if self.read32(PORT_CMD) & CMD_FR != 0 {
                    break;
                }
                spin_loop();
            }
            cmd |= CMD_ST;
            self.write32(PORT_CMD, cmd);
            for _ in 0..1_000_000 {
                if self.read32(PORT_CMD) & CMD_CR != 0 {
                    break;
                }
                spin_loop();
            }
        }
        self.engaged = true;
    }

    /// Poll until the command engine clears the slot-0 busy bit and report
    /// whether the command succeeded.
    fn wait_complete(&self) -> bool {
        let mut completed = false;
        for _ in 0..200_000_000 {
            // SAFETY: read32 preconditions hold.
            if unsafe { self.read32(PORT_CI) } & 1 == 0 {
                completed = true;
                break;
            }
            spin_loop();
        }
        // SAFETY: read32 preconditions hold.
        let tfd = unsafe { self.read32(PORT_TFD) };
        let is = unsafe { self.read32(PORT_IS) };
        self.clear_status();
        completed && (tfd & (TFD_ERR | TFD_DF)) == 0 && (is & IS_TFES) == 0
    }

    /// Build and submit a single-slot command. `count` sectors move between
    /// the device and the internal bounce buffer.
    fn issue(&mut self, write: bool, lba: u64, count: u16) -> bool {
        let count = count.max(1);
        let bytes = count as u32 * 512;
        let d = self.dma();

        // SAFETY: `d` is our exclusive DMA buffer; we fill the command header,
        // command FIS and PDTR via raw pointers. `bufs` does not borrow `self`,
        // so the MMIO calls below do not conflict.
        unsafe {
            let clb = (*d).cmd_list.as_mut_ptr();
            let ctba = (*d).cmd_table.as_mut_ptr();
            let dba = (*d).bounce.as_ptr() as u64;

            core::ptr::write_bytes(clb, 0, 32);
            put_u32(clb, 0, 5 | (if write { 1 << 6 } else { 0 }) | (1 << 16));
            put_u32(clb, 4, 0);
            put_u32(clb, 8, ctba as u32);
            put_u32(clb, 12, ((ctba as u64) >> 32) as u32);

            core::ptr::write_bytes(ctba, 0, 128);
            *ctba.add(0) = 0x27; // FIS type: host-to-device
            *ctba.add(1) = 0x80; // C=1 (command)
            *ctba.add(2) = if write { ATA_CMD_WRITE_DMA_EXT } else { ATA_CMD_READ_DMA_EXT };
            *ctba.add(4) = (lba & 0xFF) as u8;
            *ctba.add(5) = ((lba >> 8) & 0xFF) as u8;
            *ctba.add(6) = ((lba >> 16) & 0xFF) as u8;
            *ctba.add(7) = 0x40; // device: LBA mode
            *ctba.add(8) = ((lba >> 24) & 0xFF) as u8;
            *ctba.add(9) = ((lba >> 32) & 0xFF) as u8;
            *ctba.add(10) = ((lba >> 40) & 0xFF) as u8;
            *ctba.add(12) = (count & 0xFF) as u8;
            *ctba.add(13) = ((count >> 8) & 0xFF) as u8;

            let prdt = ctba.add(128);
            put_u32(prdt, 0, dba as u32);
            put_u32(prdt, 4, (dba >> 32) as u32);
            put_u32(prdt, 8, 0);
            put_u32(prdt, 12, (bytes - 1) | (1 << 31));
        }

        self.clear_status();
        // SAFETY: write32 preconditions hold; issue slot 0.
        unsafe { self.write32(PORT_CI, 1) };
        self.wait_complete()
    }

    /// Issue ATA IDENTIFY DEVICE and decode the sector count and model string.
    fn identify(&mut self) -> bool {
        if !self.engaged {
            self.setup_engine();
        }
        let d = self.dma();
        // SAFETY: exclusive DMA buffer; build IDENTIFY command with a 512-byte
        // PRDT and no LBA.
        unsafe {
            let clb = (*d).cmd_list.as_mut_ptr();
            let ctba = (*d).cmd_table.as_mut_ptr();
            let dba = (*d).bounce.as_ptr() as u64;

            core::ptr::write_bytes(clb, 0, 32);
            put_u32(clb, 0, 5 | (1 << 16));
            put_u32(clb, 8, ctba as u32);
            put_u32(clb, 12, ((ctba as u64) >> 32) as u32);

            core::ptr::write_bytes(ctba, 0, 128);
            *ctba.add(0) = 0x27;
            *ctba.add(1) = 0x80;
            *ctba.add(2) = ATA_CMD_IDENTIFY;
            *ctba.add(7) = 0x00;
            *ctba.add(12) = 1;

            let prdt = ctba.add(128);
            put_u32(prdt, 0, dba as u32);
            put_u32(prdt, 4, (dba >> 32) as u32);
            put_u32(prdt, 8, 0);
            put_u32(prdt, 12, 511 | (1 << 31));

            core::ptr::write_bytes((*d).bounce.as_mut_ptr(), 0, 512);
        }

        self.clear_status();
        // SAFETY: write32 preconditions hold; issue slot 0.
        unsafe { self.write32(PORT_CI, 1) };
        if !self.wait_complete() {
            return false;
        }

        // SAFETY: the DMA buffer was exclusively owned and populated by the
        // controller via the PRDT above. We read it through a raw pointer to
        // avoid materialising a reference into the UnsafeCell.
        let id = unsafe { (*d).bounce.as_ptr() };
        let w = |i: usize| unsafe { u16::from_le_bytes([*id.add(i * 2), *id.add(i * 2 + 1)]) };
        let lba48 = w(83) & (1 << 10) != 0;
        self.sector_count = if lba48 {
            (w(100) as u64)
                | ((w(101) as u64) << 16)
                | ((w(102) as u64) << 32)
                | ((w(103) as u64) << 48)
        } else {
            (w(60) as u64) | ((w(61) as u64) << 16)
        };
        for i in 0..20 {
            let word = w(27 + i);
            self.model[i * 2] = (word >> 8) as u8;
            self.model[i * 2 + 1] = (word & 0xFF) as u8;
        }
        true
    }

    /// Read `buf.len()` bytes (a multiple of 512) starting at sector `lba`.
    pub fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), VfsError> {
        self.transfer(false, lba, buf.as_mut_ptr(), buf.len())
    }

    /// Write `buf.len()` bytes (a multiple of 512) starting at sector `lba`.
    pub fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), VfsError> {
        // SAFETY: with `write == true`, `transfer` only *reads* through the
        // pointer, so handing it a shared slice's pointer is sound.
        self.transfer(true, lba, buf.as_ptr() as *mut u8, buf.len())
    }

    fn transfer(
        &mut self,
        write: bool,
        mut lba: u64,
        buf: *mut u8,
        len: usize,
    ) -> Result<(), VfsError> {
        if len % 512 != 0 {
            return Err(VfsError::IOError);
        }
        if !self.engaged {
            self.setup_engine();
        }
        let d = self.dma();
        // SAFETY: bounce length for chunk sizing.
        let max_chunk = unsafe { (*d).bounce.len() } & !511usize;

        let mut done = 0usize;
        while done < len {
            let chunk = (len - done).min(max_chunk);
            let sectors = (chunk / 512) as u16;

            // SAFETY: `d` is our exclusive DMA buffer; `bptr` addresses the
            // bounce region. `buf` is valid for `len` bytes per the caller.
            unsafe {
                let bptr = (*d).bounce.as_mut_ptr();
                if write {
                    core::ptr::copy_nonoverlapping(buf.add(done), bptr, chunk);
                } else {
                    core::ptr::write_bytes(bptr, 0, chunk);
                }
            }

            if !self.issue(write, lba, sectors) {
                return Err(VfsError::IOError);
            }

            if !write {
                // SAFETY: as above; the controller filled `bptr` with `chunk`
                // bytes via DMA and `buf` is `len` bytes long.
                unsafe {
                    let bptr = (*d).bounce.as_ptr();
                    core::ptr::copy_nonoverlapping(bptr, buf.add(done), chunk);
                }
            }

            done += chunk;
            lba += sectors as u64;
        }
        Ok(())
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
    unsafe fn read32(&self, off: usize) -> u32 {
        core::ptr::read_volatile((self.mmio_base as usize + off) as *const u32)
    }

    /// # Safety
    /// Same as `read32`.
    unsafe fn write32(&self, off: usize, val: u32) {
        core::ptr::write_volatile((self.mmio_base as usize + off) as *mut u32, val);
    }

    /// Set GHC.AE (AHCI Enable).
    fn enable(&mut self) {
        // SAFETY: mmio_base is a valid BAR5 region.
        unsafe {
            let ghc = self.read32(HBA_GHC);
            self.write32(HBA_GHC, ghc | GHC_AE);
        }
    }

    pub fn enumerate_ports(&mut self) {
        // SAFETY: read32 preconditions hold — mmio_base is the AHCI BAR5 base.
        let pi = unsafe { self.read32(HBA_PI) };
        for i in 0..32u32 {
            if (pi & (1 << i)) == 0 {
                continue;
            }
            let port_base = self.mmio_base + 0x100 + (i as u64) * 0x80;
            // SAFETY: port_base is within the AHCI BAR5 MMIO region.
            let sig =
                unsafe { core::ptr::read_volatile((port_base as usize + PORT_SIG) as *const u32) };
            let port_type = match sig {
                SATA_SIG_ATA => PortType::Sata,
                SATA_SIG_ATAPI => PortType::Satapi,
                _ => PortType::None,
            };
            let mut port = AhciPort::new(i as usize, self.mmio_base, port_type);
            let present = port.is_device_present();
            if present && port_type == PortType::Sata {
                if port.identify() {
                    let model = core::str::from_utf8(&port.model).unwrap_or("?").trim();
                    serial::line(&alloc::format!(
                        "[AHCI] Port {} — ATA, {} sectors, model '{}'",
                        i,
                        port.sector_count,
                        model
                    ));
                } else {
                    serial::line(&alloc::format!("[AHCI] Port {} — ATA, IDENTIFY failed", i));
                }
            } else if present {
                serial::line(&alloc::format!("[AHCI] Port {} — {:?}, present", i, port_type));
            }
            self.ports.push(port);
        }
    }
}

/// One AHCI port exposed as a portable block device.
pub struct AhciBlockDevice {
    ctrl: Arc<SpinLock<AhciController>>,
    port_index: usize,
    sectors: u64,
}

impl BlockDevice for AhciBlockDevice {
    fn num_sectors(&self) -> u64 {
        self.sectors
    }

    fn read_sectors(&self, lba: u64, buf: &mut [u8]) -> Result<(), VfsError> {
        self.ctrl.lock().ports[self.port_index].read_sectors(lba, buf)
    }

    fn write_sectors(&self, lba: u64, buf: &[u8]) -> Result<(), VfsError> {
        self.ctrl.lock().ports[self.port_index].write_sectors(lba, buf)
    }
}

// SAFETY: `AhciBlockDevice` only accesses the controller through its spin
// lock, which serialises all register and DMA access.
unsafe impl Send for AhciBlockDevice {}
unsafe impl Sync for AhciBlockDevice {}

fn put_u32(p: *mut u8, off: usize, v: u32) {
    unsafe {
        core::ptr::write_unaligned(p.add(off) as *mut u32, v);
    }
}

/// Global AHCI controller (shared with the block-device adapter).
pub static AHCI_CONTROLLER: SpinLock<Option<Arc<SpinLock<AhciController>>>> = SpinLock::new(None);

/// The data disk advertised to the filesystem layer, if any.
pub static AHCI_DISK: SpinLock<Option<Arc<dyn BlockDevice>>> = SpinLock::new(None);

/// Initialise the controller and enumerate/identify its ports.
pub fn init(mmio_base: u64) -> Option<Arc<SpinLock<AhciController>>> {
    serial::line("[AHCI] Initializing AHCI controller...");
    let ctrl = Arc::new(SpinLock::new(AhciController::new(mmio_base)));
    {
        let mut guard = ctrl.lock();
        guard.enable();
        guard.enumerate_ports();
        if guard.ports.is_empty() {
            return None;
        }
    }
    serial::line(&alloc::format!(
        "[AHCI] Controller ready with {} port(s).",
        ctrl.lock().ports.len()
    ));
    Some(ctrl)
}

/// Marker written to LBA 0 of the designated data disk. It lives in the Ext2
/// boot block (bytes 0..1024), which the formatter never overwrites, so it
/// survives reformatting and lets us identify our disk without ever touching
/// the boot device.
pub const DATA_DISK_MAGIC: &[u8; 8] = b"BWDISK01";

/// Find the ATA drive carrying [`DATA_DISK_MAGIC`] at LBA 0. Probing only
/// reads LBA 0, so non-matching (e.g. boot) disks are never modified.
pub fn find_data_disk(ctrl: &Arc<SpinLock<AhciController>>) -> Option<Arc<dyn BlockDevice>> {
    let mut chosen: Option<(usize, u64)> = None;
    {
        let mut guard = ctrl.lock();
        for (i, port) in guard.ports.iter_mut().enumerate() {
            if port.port_type != PortType::Sata || port.sector_count == 0 {
                continue;
            }
            let mut sector = [0u8; 512];
            if port.read_sectors(0, &mut sector).is_ok()
                && &sector[..DATA_DISK_MAGIC.len()] == DATA_DISK_MAGIC
            {
                chosen = Some((i, port.sector_count));
                break;
            }
        }
    }
    chosen.map(|(port_index, sectors)| {
        Arc::new(AhciBlockDevice { ctrl: Arc::clone(ctrl), port_index, sectors })
            as Arc<dyn BlockDevice>
    })
}
