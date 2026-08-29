pub const PAGE_SIZE: u64 = 4096;
pub const USER_SPACE_END: u64 = 0x0000_7fff_ffff_ffff;
pub const KERNEL_SPACE_START: u64 = 0xffff_8000_0000_0000;
pub const KERNEL_VIRTUAL_BASE: u64 = 0xffff_ffff_8000_0000;

pub const PAGE_PRESENT: u64 = 1 << 0;
pub const PAGE_WRITABLE: u64 = 1 << 1;
pub const PAGE_USER_ACCESSIBLE: u64 = 1 << 2;
pub const PAGE_WRITE_THROUGH: u64 = 1 << 3;
pub const PAGE_CACHE_DISABLE: u64 = 1 << 4;
pub const PAGE_ACCESSED: u64 = 1 << 5;
pub const PAGE_DIRTY: u64 = 1 << 6;
pub const PAGE_HUGE: u64 = 1 << 7;
pub const PAGE_GLOBAL: u64 = 1 << 8;
pub const PAGE_NO_EXECUTE: u64 = 1 << 63;

pub fn align_down(address: u64) -> u64 {
    address & !(PAGE_SIZE - 1)
}

pub fn align_up(address: u64) -> u64 {
    (address + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub const fn new() -> Self {
        Self(0)
    }

    pub fn is_present(&self) -> bool {
        self.0 & PAGE_PRESENT != 0
    }

    pub fn address(&self) -> u64 {
        self.0 & 0x000f_ffff_ffff_f000
    }

    pub fn set(&mut self, addr: u64, flags: u64) {
        self.0 = (addr & 0x000f_ffff_ffff_f000) | flags;
    }

    pub fn flags(&self) -> u64 {
        self.0 & 0xfff0_0000_0000_0fff
    }
}

#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn new() -> Self {
        Self { entries: [PageTableEntry::new(); 512] }
    }

    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            *entry = PageTableEntry::new();
        }
    }
}
