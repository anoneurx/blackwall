//! ACPI table parser.
//!
//! Under UEFI the RSDP is exposed through an EFI configuration table, so the
//! kernel captures its physical address before `ExitBootServices` and uses it
//! afterwards. Legacy BIOS scanning is kept as a fallback for non-UEFI boots.
//!
//! Parses the RSDT/XSDT root, then MADT (local/IO APICs), FADT (PM timer) and
//! MCFG (PCIe ECAM) tables. The decoded information is published in [`ACPI`]
//! for later subsystems (notably SMP).

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::arch::x86_64::serial;
use crate::sync::spin::SpinLock;
use uefi::prelude::SystemTable;
use uefi::table::Boot;

const RSDP_SIG: &[u8; 8] = b"RSD PTR ";
const RSDT_SIG: &[u8; 4] = b"RSDT";
const XSDT_SIG: &[u8; 4] = b"XSDT";
const MADT_SIG: &[u8; 4] = b"APIC";
const FADT_SIG: &[u8; 4] = b"FACP";
const HPET_SIG: &[u8; 4] = b"HPET";
const MCFG_SIG: &[u8; 4] = b"MCFG";

const ACPI_HEADER_LEN: usize = 36;

/// Physical address of the RSDP captured from the EFI configuration table.
static RSDP_ADDR: AtomicUsize = AtomicUsize::new(0);

#[repr(C, packed)]
pub struct AcpiHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table: [u8; 8],
    pub oem_rev: u32,
    pub creator: u32,
    pub creator_rev: u32,
}

pub fn checksum(data: &[u8]) -> bool {
    data.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

/// Record the RSDP address advertised by the firmware. Must be called while
/// boot services are still available (i.e. before `ExitBootServices`).
pub fn capture_rsdp(system_table: &SystemTable<Boot>) {
    use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID};
    let mut fallback = 0usize;
    for entry in system_table.config_table() {
        if entry.guid == ACPI2_GUID {
            RSDP_ADDR.store(entry.address as usize, Ordering::SeqCst);
            return;
        }
        if entry.guid == ACPI_GUID && fallback == 0 {
            fallback = entry.address as usize;
        }
    }
    if fallback != 0 {
        RSDP_ADDR.store(fallback, Ordering::SeqCst);
    }
}

/// Legacy BIOS scan of the EBDA and the 0xE0000–0xFFFFF region for the RSDP.
fn scan_legacy_rsdp() -> Option<usize> {
    // SAFETY: 0x40E is the standard BIOS EBDA segment pointer in the low 1 MiB.
    let ebda_base = unsafe {
        let seg = core::ptr::read_volatile(0x40E as *const u16);
        (seg as usize) << 4
    };

    let regions = [(ebda_base, ebda_base.saturating_add(1024)), (0x000E_0000, 0x000F_FFFF)];

    for (start, end) in regions {
        if start == 0 || end <= start {
            continue;
        }
        let mut ptr = start;
        while ptr + 8 <= end {
            // SAFETY: `ptr` is within the firmware scan region and all reads
            // stay within `[start, end)`.
            let sig = unsafe { core::slice::from_raw_parts(ptr as *const u8, 8) };
            if sig == RSDP_SIG {
                // SAFETY: as above; the RSDP is at least 20 bytes.
                let rsdp_bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, 20) };
                if checksum(rsdp_bytes) {
                    return Some(ptr);
                }
            }
            ptr += 16;
        }
    }
    None
}

/// Locate the RSDP, preferring the address captured from firmware.
pub fn find_rsdp() -> Option<usize> {
    let captured = RSDP_ADDR.load(Ordering::SeqCst);
    if captured != 0 {
        return Some(captured);
    }
    scan_legacy_rsdp()
}

#[derive(Debug, Clone)]
pub struct AcpiInfo {
    pub rsdp_addr: usize,
    pub has_madt: bool,
    pub has_fadt: bool,
    pub has_hpet: bool,
    pub has_mcfg: bool,
    pub local_apic_addr: u32,
    pub local_apic_ids: alloc::vec::Vec<u8>,
    pub io_apic_addr: Option<u32>,
    pub pm_timer_addr: Option<u32>,
    pub mcfg_ecam: Option<u64>,
}

impl AcpiInfo {
    fn empty() -> Self {
        Self {
            rsdp_addr: 0,
            has_madt: false,
            has_fadt: false,
            has_hpet: false,
            has_mcfg: false,
            local_apic_addr: 0,
            local_apic_ids: alloc::vec::Vec::new(),
            io_apic_addr: None,
            pm_timer_addr: None,
            mcfg_ecam: None,
        }
    }
}

fn read_u32(addr: usize, off: usize) -> u32 {
    // SAFETY: caller guarantees `addr + off` is a valid, mapped ACPI address.
    unsafe { core::ptr::read_unaligned((addr + off) as *const u32) }
}

fn read_u64(addr: usize, off: usize) -> u64 {
    unsafe { core::ptr::read_unaligned((addr + off) as *const u64) }
}

fn read_u8(addr: usize, off: usize) -> u8 {
    unsafe { core::ptr::read_unaligned((addr + off) as *const u8) }
}

/// Read the 4-byte signature at `addr` into a local array (avoids taking a
/// reference into potentially-unaligned firmware memory).
fn signature_at(addr: usize) -> [u8; 4] {
    // SAFETY: ACPI tables begin with a 4-byte signature at `addr`.
    unsafe { core::ptr::read_unaligned(addr as *const [u8; 4]) }
}

fn parse_madt(addr: usize, info: &mut AcpiInfo) {
    let length = read_u32(addr, 4) as usize;
    if length < ACPI_HEADER_LEN + 8 {
        return;
    }
    info.local_apic_addr = read_u32(addr, ACPI_HEADER_LEN);
    let mut off = ACPI_HEADER_LEN + 8; // skip header + local APIC addr + flags
    while off + 2 <= length {
        let entry_type = read_u8(addr, off);
        let entry_len = read_u8(addr, off + 1) as usize;
        if entry_len < 2 || off + entry_len > length {
            break;
        }
        match entry_type {
            // Processor Local APIC
            0 => {
                if read_u32(addr, off + 4) & 1 != 0 {
                    info.local_apic_ids.push(read_u8(addr, off + 3));
                }
            }
            // I/O APIC
            1 => {
                if info.io_apic_addr.is_none() {
                    info.io_apic_addr = Some(read_u32(addr, off + 4));
                }
            }
            // Interrupt Source Override (type 2) and others are ignored here.
            _ => {}
        }
        off += entry_len;
    }
}

fn parse_fadt(addr: usize, info: &mut AcpiInfo) {
    let length = read_u32(addr, 4) as usize;
    if length >= 80 {
        let pm = read_u32(addr, 76);
        if pm != 0 {
            info.pm_timer_addr = Some(pm);
        }
    }
}

fn parse_mcfg(addr: usize, info: &mut AcpiInfo) {
    let length = read_u32(addr, 4) as usize;
    if length >= ACPI_HEADER_LEN + 8 + 16 {
        // First allocation entry's base address.
        let base = read_u64(addr, ACPI_HEADER_LEN + 8);
        if base != 0 {
            info.mcfg_ecam = Some(base);
        }
    }
}

pub fn parse_tables() -> AcpiInfo {
    let mut info = AcpiInfo::empty();

    let rsdp_addr = match find_rsdp() {
        Some(a) => a,
        None => {
            serial::line("[ACPI] RSDP not found — running without ACPI.");
            return info;
        }
    };
    info.rsdp_addr = rsdp_addr;

    serial::line(&alloc::format!("[ACPI] RSDP at {:#x}", rsdp_addr));
    let revision = read_u8(rsdp_addr, 15);
    serial::line(&alloc::format!("[ACPI] Revision: {revision}"));

    let rsdt_addr = read_u32(rsdp_addr, 16) as usize;
    let xsdt_addr = if revision >= 2 { read_u64(rsdp_addr, 24) as usize } else { 0 };

    // Prefer the XSDT (64-bit entries) when present.
    let (root_addr, entry_size) = if xsdt_addr != 0 && &signature_at(xsdt_addr) == XSDT_SIG {
        (xsdt_addr, 8usize)
    } else if rsdt_addr != 0 && &signature_at(rsdt_addr) == RSDT_SIG {
        (rsdt_addr, 4usize)
    } else {
        serial::line("[ACPI] No valid RSDT/XSDT root.");
        return info;
    };

    let root_len = read_u32(root_addr, 4) as usize;
    if root_len < ACPI_HEADER_LEN {
        return info;
    }
    let entry_bytes = root_len - ACPI_HEADER_LEN;
    let entry_count = entry_bytes / entry_size;

    for i in 0..entry_count {
        let table_addr = if entry_size == 8 {
            read_u64(root_addr, ACPI_HEADER_LEN + i * 8) as usize
        } else {
            read_u32(root_addr, ACPI_HEADER_LEN + i * 4) as usize
        };
        if table_addr == 0 {
            continue;
        }
        let sig = signature_at(table_addr);
        match &sig {
            MADT_SIG => {
                info.has_madt = true;
                parse_madt(table_addr, &mut info);
            }
            FADT_SIG => {
                info.has_fadt = true;
                parse_fadt(table_addr, &mut info);
            }
            HPET_SIG => info.has_hpet = true,
            MCFG_SIG => {
                info.has_mcfg = true;
                parse_mcfg(table_addr, &mut info);
            }
            _ => {}
        }
    }

    info
}

/// Decoded ACPI information, published for later subsystems.
pub static ACPI: SpinLock<Option<AcpiInfo>> = SpinLock::new(None);

pub fn init() {
    serial::line("[ACPI] Parsing ACPI tables...");
    let info = parse_tables();
    if info.has_madt {
        serial::line(&alloc::format!(
            "[ACPI] MADT: {} CPU(s), local APIC at {:#x}",
            info.local_apic_ids.len(),
            info.local_apic_addr
        ));
    }
    if let Some(io) = info.io_apic_addr {
        serial::line(&alloc::format!("[ACPI] I/O APIC at {:#x}", io));
    }
    if let Some(pm) = info.pm_timer_addr {
        serial::line(&alloc::format!("[ACPI] PM timer at {:#x}", pm));
    }
    if let Some(ecam) = info.mcfg_ecam {
        serial::line(&alloc::format!("[ACPI] MCFG ECAM at {:#x}", ecam));
    }
    serial::line(&alloc::format!(
        "[ACPI] MADT={} FADT={} HPET={} MCFG={} CPUs={}",
        info.has_madt,
        info.has_fadt,
        info.has_hpet,
        info.has_mcfg,
        info.local_apic_ids.len()
    ));
    *ACPI.lock() = Some(info);
}
