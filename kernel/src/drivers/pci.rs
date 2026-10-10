use crate::arch::x86_64::serial;
use alloc::vec::Vec;
use core::arch::asm;

#[derive(Debug, Clone, Copy)]
pub enum PciBar {
    None,
    Memory32 { address: u32, prefetchable: bool },
    Memory64 { address: u64, prefetchable: bool },
    IO { port: u16 },
}

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

impl PciDevice {
    /// Enable I/O + memory space decoding and bus-master (DMA) for this device.
    /// Required before a storage controller may perform DMA.
    pub fn enable_bus_master(&self) {
        let cmd = pci_read(self.bus, self.slot, self.func, 0x04);
        let new = cmd | 0x02 /* Memory Space */ | 0x04 /* Bus Master */;
        if new != cmd {
            pci_write(self.bus, self.slot, self.func, 0x04, new);
        }
    }

    pub fn get_bar(&self, bar_idx: u8) -> PciBar {
        if bar_idx >= 6 {
            return PciBar::None;
        }
        let offset = 0x10 + (bar_idx * 4);
        let val = pci_read(self.bus, self.slot, self.func, offset);
        if val == 0 {
            return PciBar::None;
        }

        if (val & 0x01) != 0 {
            PciBar::IO { port: (val & 0xFFFC) as u16 }
        } else {
            let prefetchable = (val & 0x08) != 0;
            let bar_type = (val >> 1) & 0x03;
            if bar_type == 0x02 {
                if bar_idx >= 5 {
                    return PciBar::None;
                }
                let val_high = pci_read(self.bus, self.slot, self.func, offset + 4);
                let address = ((val_high as u64) << 32) | ((val & 0xFFFFFFF0) as u64);
                PciBar::Memory64 { address, prefetchable }
            } else {
                PciBar::Memory32 { address: val & 0xFFFFFFF0, prefetchable }
            }
        }
    }
}

/// # Safety
/// Caller must pass a valid x86 I/O port address. On x86_64, I/O port access
/// is always ring-0 only; we are already in kernel mode, so this is safe
/// for the standard PCI config-space ports (0xCF8/0xCFC).
#[inline(always)]
unsafe fn outd(port: u16, value: u32) {
    // SAFETY: Restricted to the PCI config address/data ports by all callers.
    asm!("out dx, eax", in("dx") port, in("eax") value, options(nostack, preserves_flags));
}

/// # Safety
/// Same as `outd` — caller ensures `port` is a valid, accessible I/O port.
#[inline(always)]
unsafe fn ind(port: u16) -> u32 {
    let value: u32;
    // SAFETY: Restricted to the PCI config data port by all callers.
    asm!("in eax, dx", in("dx") port, out("eax") value, options(nostack, preserves_flags));
    value
}

pub fn pci_read(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xfc)
        | 0x80000000;
    // SAFETY: 0xCF8 is the PCI Configuration Address Port and 0xCFC is the
    // PCI Configuration Data Port — both are universally available on x86_64
    // PC-compatible hardware. The address is masked to a 32-bit aligned offset
    // (low 2 bits cleared), preventing unaligned config-space reads.
    unsafe {
        outd(0xCF8, address);
        ind(0xCFC)
    }
}

pub fn pci_write(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let address = 0x8000_0000u32
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);
    // SAFETY: 0xCF8/0xCFC are the universal x86 PCI config address/data ports;
    // the offset is masked to a 32-bit aligned register.
    unsafe {
        outd(0xCF8, address);
        outd(0xCFC, value);
    }
}

pub fn scan_bus() -> Vec<PciDevice> {
    let mut devices = Vec::new();
    serial::line("[PCI] Initiating PCI bus scan...");

    for bus in 0..=255 {
        for slot in 0..32 {
            // Check first function of device
            let val = pci_read(bus, slot, 0, 0);
            let vendor_id = (val & 0xFFFF) as u16;
            if vendor_id == 0xFFFF {
                continue; // Device not present
            }

            // Read header type to see if it is a multi-function device
            let header_val = pci_read(bus, slot, 0, 0x0C);
            let header_type = ((header_val >> 16) & 0xFF) as u8;
            let max_func = if (header_type & 0x80) != 0 { 8 } else { 1 };

            for func in 0..max_func {
                let func_val = pci_read(bus, slot, func, 0);
                let func_vendor = (func_val & 0xFFFF) as u16;
                if func_vendor == 0xFFFF {
                    continue;
                }

                let device_id = ((func_val >> 16) & 0xFFFF) as u16;
                let class_val = pci_read(bus, slot, func, 0x08);
                let class_code = ((class_val >> 24) & 0xFF) as u8;
                let subclass = ((class_val >> 16) & 0xFF) as u8;
                let prog_if = ((class_val >> 8) & 0xFF) as u8;

                let device = PciDevice {
                    bus,
                    slot,
                    func,
                    vendor_id: func_vendor,
                    device_id,
                    class_code,
                    subclass,
                    prog_if,
                };

                log_device(&device);
                devices.push(device);
            }
        }
    }
    serial::line(&alloc::format!("[PCI] Scan complete. Found {} devices.", devices.len()));
    devices
}

fn log_device(dev: &PciDevice) {
    let class_str = match dev.class_code {
        0x01 => match dev.subclass {
            0x08 => "Mass Storage (NVMe Controller)",
            _ => "Mass Storage Device",
        },
        0x02 => "Network Controller",
        0x03 => "Display Controller",
        0x06 => "Bridge Device",
        _ => "Generic Device",
    };

    let virtio_str = if dev.vendor_id == 0x1AF4 { " (VirtIO Device)" } else { "" };

    serial::line(&alloc::format!(
        "[PCI] {:02x}:{:02x}.{} -> ID {:04x}:{:04x} | Class {:02x}:{:02x} | {}{}",
        dev.bus,
        dev.slot,
        dev.func,
        dev.vendor_id,
        dev.device_id,
        dev.class_code,
        dev.subclass,
        class_str,
        virtio_str
    ));
}
