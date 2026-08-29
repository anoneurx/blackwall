use crate::logging;
use core::arch::x86_64::{__cpuid, __cpuid_count, CpuidResult};

#[derive(Clone, Copy)]
pub struct CpuInfo {
    pub vendor: [u8; 13],
    pub brand: [u8; 49],
    pub cores: u32,
    pub sse: bool,
    pub sse2: bool,
    pub avx: bool,
    pub avx2: bool,
}

impl CpuInfo {
    pub fn log(&self) {
        logging::print(format_args!("CPU Vendor: {}\n", self.vendor_str()));
        logging::print(format_args!("CPU Model: {}\n", self.brand_str()));
        logging::print(format_args!("CPU Cores: {}\n", self.cores));

        let mut feature_line = StringBuilder::new();
        feature_line.push("Features: ");
        if self.sse {
            feature_line.push("SSE ");
        }
        if self.sse2 {
            feature_line.push("SSE2 ");
        }
        if self.avx {
            feature_line.push("AVX ");
        }
        if self.avx2 {
            feature_line.push("AVX2 ");
        }
        logging::print(format_args!("{}\n", feature_line.as_str()));
    }

    fn vendor_str(&self) -> &str {
        trim_ascii(&self.vendor)
    }

    fn brand_str(&self) -> &str {
        trim_ascii(&self.brand)
    }
}

pub fn detect() -> CpuInfo {
    let vendor_leaf = __cpuid(0);
    let max_basic_leaf = vendor_leaf.eax;
    let vendor = vendor_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);

    let brand = if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
        brand_bytes()
    } else {
        *b"Unknown CPU                                     \0"
    };

    let feature_leaf = __cpuid(1);
    let feature_leaf7 = if max_basic_leaf >= 7 {
        __cpuid_count(7, 0)
    } else {
        CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 }
    };

    CpuInfo {
        vendor,
        brand,
        cores: logical_core_count(),
        sse: feature_leaf.edx & (1 << 25) != 0,
        sse2: feature_leaf.edx & (1 << 26) != 0,
        avx: feature_leaf.ecx & (1 << 28) != 0,
        avx2: feature_leaf7.ebx & (1 << 5) != 0,
    }
}

fn logical_core_count() -> u32 {
    let max_basic_leaf = __cpuid(0).eax;

    if max_basic_leaf < 0x0b {
        return 1;
    }

    let topology = __cpuid_count(0x0b, 1);

    let logical = topology.ebx & 0xffff;
    if logical == 0 {
        1
    } else {
        logical
    }
}

fn vendor_bytes(ebx: u32, edx: u32, ecx: u32) -> [u8; 13] {
    let mut vendor = [0u8; 13];
    vendor[0..4].copy_from_slice(&ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&ecx.to_le_bytes());
    vendor[12] = 0;
    vendor
}

fn brand_bytes() -> [u8; 49] {
    let mut brand = [0u8; 49];
    let mut offset = 0;
    while offset < 48 {
        let leaf = __cpuid(0x8000_0002 + (offset / 16) as u32);
        brand[offset..offset + 4].copy_from_slice(&leaf.eax.to_le_bytes());
        brand[offset + 4..offset + 8].copy_from_slice(&leaf.ebx.to_le_bytes());
        brand[offset + 8..offset + 12].copy_from_slice(&leaf.ecx.to_le_bytes());
        brand[offset + 12..offset + 16].copy_from_slice(&leaf.edx.to_le_bytes());
        offset += 16;
    }
    brand[48] = 0;
    brand
}

fn trim_ascii(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..end]).unwrap_or("Unknown")
}

struct StringBuilder {
    buf: [u8; 64],
    len: usize,
}

impl StringBuilder {
    const fn new() -> Self {
        Self { buf: [0; 64], len: 0 }
    }

    fn push(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let remaining = self.buf.len().saturating_sub(self.len);
        let count = remaining.min(bytes.len());
        self.buf[self.len..self.len + count].copy_from_slice(&bytes[..count]);
        self.len += count;
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}
