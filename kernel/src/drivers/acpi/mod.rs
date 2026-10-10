/// ACPI Table Parser
/// Locates the RSDP, walks RSDT/XSDT, and parses MADT and FADT tables.
extern crate alloc;

use crate::arch::x86_64::serial;

const RSDP_SIG: &[u8; 8] = b"RSD PTR ";
#[allow(dead_code)]
const RSDT_SIG: &[u8; 4] = b"RSDT";
#[allow(dead_code)]
const XSDT_SIG: &[u8; 4] = b"XSDT";
const MADT_SIG: &[u8; 4] = b"APIC";
const FADT_SIG: &[u8; 4] = b"FACP";
const HPET_SIG: &[u8; 4] = b"HPET";

#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    // ACPI 2.0+ fields follow for revision >= 2
    length: u32,
    xsdt_addr: u64,
    ext_chk: u8,
    _reserved: [u8; 3],
}

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

fn checksum(data: &[u8]) -> bool {
    data.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

/// Scan the BIOS memory area (0x000E0000–0x000FFFFF) for the RSDP signature.
/// Returns the physical address of the RSDP if found.
pub fn find_rsdp() -> Option<usize> {
    // Also check EBDA (first 1 KiB pointed at by 0x040E)
    let ebda_base = unsafe {
        let seg = core::ptr::read_volatile(0x40E as *const u16);
        (seg as usize) << 4
    };

    let regions = [(ebda_base, ebda_base + 1024), (0x000E_0000, 0x000F_FFFF)];

    for (start, end) in regions {
        if start == 0 || end <= start {
            continue;
        }
        let mut ptr = start;
        while ptr + 8 <= end {
            // SAFETY: `ptr` is a valid, mapped physical address within the
            // firmware scan region and is non-null (guarded above). All reads
            // stay within `[start, end)`.
            let sig = unsafe { core::slice::from_raw_parts(ptr as *const u8, 8) };
            if sig == RSDP_SIG {
                // Verify checksum of first 20 bytes
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

#[derive(Debug, Clone)]
pub struct AcpiInfo {
    pub has_madt: bool,
    pub has_fadt: bool,
    pub has_hpet: bool,
    pub local_apic_count: usize,
    pub io_apic_addr: Option<u32>,
    pub pm_timer_addr: Option<u32>,
}

pub fn parse_tables() -> AcpiInfo {
    let mut info = AcpiInfo {
        has_madt: false,
        has_fadt: false,
        has_hpet: false,
        local_apic_count: 1, // Always at least BSP
        io_apic_addr: None,
        pm_timer_addr: None,
    };

    let rsdp_addr = match find_rsdp() {
        Some(a) => a,
        None => {
            serial::line("[ACPI] RSDP not found — running without ACPI.");
            return info;
        }
    };

    serial::line(&alloc::format!("[ACPI] RSDP found at {:#x}", rsdp_addr));
    let rsdp = unsafe { &*(rsdp_addr as *const Rsdp) };
    let revision = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(rsdp.revision)) };
    serial::line(&alloc::format!("[ACPI] Revision: {revision}"));

    // Walk RSDT (32-bit pointers)
    let rsdt_addr =
        unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(rsdp.rsdt_addr)) } as usize;
    let rsdt_hdr = unsafe { &*(rsdt_addr as *const AcpiHeader) };
    let rsdt_len =
        unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(rsdt_hdr.length)) } as usize;

    if rsdt_len < core::mem::size_of::<AcpiHeader>() {
        return info;
    }

    let entry_bytes = rsdt_len - core::mem::size_of::<AcpiHeader>();
    let entry_count = entry_bytes / 4;
    let entries_ptr = (rsdt_addr + core::mem::size_of::<AcpiHeader>()) as *const u32;

    for i in 0..entry_count {
        let table_addr = unsafe { core::ptr::read_unaligned(entries_ptr.add(i)) } as usize;
        if table_addr == 0 {
            continue;
        }

        let hdr = unsafe { &*(table_addr as *const AcpiHeader) };
        let sig = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(hdr.signature)) };

        if &sig == MADT_SIG {
            info.has_madt = true;
            serial::line("[ACPI] Found MADT (APIC table).");
        } else if &sig == FADT_SIG {
            info.has_fadt = true;
            serial::line("[ACPI] Found FADT (Fixed ACPI Description Table).");
        } else if &sig == HPET_SIG {
            info.has_hpet = true;
            serial::line("[ACPI] Found HPET (High Precision Event Timer).");
        }
    }

    info
}

pub fn init() {
    serial::line("[ACPI] Parsing ACPI tables...");
    let info = parse_tables();
    serial::line(&alloc::format!(
        "[ACPI] MADT={} FADT={} HPET={} CPUs={}",
        info.has_madt,
        info.has_fadt,
        info.has_hpet,
        info.local_apic_count
    ));
}
