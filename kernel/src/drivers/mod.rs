pub mod acpi;
pub mod ahci;
pub mod nvme;
pub mod pci;
pub mod ps2;
pub mod rtc;
pub mod timer;
pub mod vga;
pub mod virtio;

use crate::arch::x86_64::serial;
use pci::PciBar;

pub fn init() {
    serial::line("[DRIVERS] Initializing core system drivers...");

    // Initialize basic screen, timer, clock, ACPI tables, and keyboard input
    vga::init();
    timer::init();
    rtc::init();
    acpi::init();
    ps2::init();

    serial::line("[DRIVERS] Scanning PCI bus for high-performance devices...");
    let devices = pci::scan_bus();

    for dev in &devices {
        // NVMe Device identification: Class 01, Subclass 08
        if dev.class_code == 0x01 && dev.subclass == 0x08 {
            let bar0 = dev.get_bar(0);
            let mmio_addr = match bar0 {
                PciBar::Memory32 { address, .. } => Some(address as u64),
                PciBar::Memory64 { address, .. } => Some(address),
                _ => None,
            };

            if let Some(addr) = mmio_addr {
                nvme::init(addr);
            } else {
                serial::line("[DRIVERS] Found NVMe Controller but BAR0 is not MMIO.");
            }
        }

        // AHCI (SATA) controller: Class 01, Subclass 06, programming interface 0x01.
        // The AHCI BAR is BAR5 (ABAR), always memory-mapped.
        if dev.class_code == 0x01 && dev.subclass == 0x06 && dev.prog_if == 0x01 {
            let mmio_addr = match dev.get_bar(5) {
                PciBar::Memory32 { address, .. } => Some(address as u64),
                PciBar::Memory64 { address, .. } => Some(address),
                _ => None,
            };

            if let Some(addr) = mmio_addr {
                if let Some(ctrl) = ahci::init(addr) {
                    *ahci::AHCI_CONTROLLER.lock() = Some(ctrl);
                }
            } else {
                serial::line("[DRIVERS] Found AHCI Controller but BAR5 is not MMIO.");
            }
        }

        // VirtIO Device identification: Vendor 0x1AF4 (Red Hat)
        if dev.vendor_id == 0x1AF4 {
            let bar0 = dev.get_bar(0);
            let io_base = match bar0 {
                PciBar::IO { port } => Some(port as u32),
                _ => None,
            };

            match dev.device_id {
                0x1000 | 0x1041 => {
                    if let Some(port) = io_base {
                        if let Some(nic) = virtio::net::init(port) {
                            *virtio::net::VIRTIO_NET_DEVICE.lock() = Some(nic);
                        }
                    } else {
                        serial::line(
                            "[DRIVERS] Found VirtIO Net device but BAR0 is not I/O mapped.",
                        );
                    }
                }
                0x1001 | 0x1042 => {
                    if let Some(port) = io_base {
                        virtio::block::init(port);
                    } else {
                        serial::line(
                            "[DRIVERS] Found VirtIO Block device but BAR0 is not I/O mapped.",
                        );
                    }
                }
                _ => {
                    serial::line(&alloc::format!(
                        "[DRIVERS] Discovered unhandled VirtIO device (ID: {:#x})",
                        dev.device_id
                    ));
                }
            }
        }
    }
}
