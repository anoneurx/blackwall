/// Minimal ELF64 parser for loading userspace binaries.
///
/// Only PT_LOAD segments are required. All other segment types are
/// intentionally ignored. This keeps the loader small and auditable.

// ─── ELF constants ──────────────────────────────────────────────────────────

pub const ELFMAG: [u8; 4] = [0x7f, b'E', b'L', b'F'];
pub const EI_CLASS_64: u8 = 2;
pub const EI_DATA_LE: u8 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3; // PIE
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;

pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

// ─── ELF64 structures ───────────────────────────────────────────────────────

#[repr(C)]
pub struct Elf64Header {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

// ─── Parser ─────────────────────────────────────────────────────────────────

pub struct ElfLoader<'a> {
    data: &'a [u8],
    pub header: &'a Elf64Header,
}

impl<'a> ElfLoader<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, &'static str> {
        if data.len() < core::mem::size_of::<Elf64Header>() {
            return Err("ELF too small");
        }

        let header = unsafe { &*(data.as_ptr() as *const Elf64Header) };

        if header.e_ident[..4] != ELFMAG {
            return Err("Bad ELF magic");
        }
        if header.e_ident[4] != EI_CLASS_64 {
            return Err("Not ELF64");
        }
        if header.e_ident[5] != EI_DATA_LE {
            return Err("Not little-endian");
        }
        if header.e_machine != EM_X86_64 {
            return Err("Not x86_64");
        }
        if header.e_type != ET_EXEC && header.e_type != ET_DYN {
            return Err("Not an executable");
        }

        Ok(Self { data, header })
    }

    pub fn is_pie(&self) -> bool {
        self.header.e_type == ET_DYN
    }

    pub fn entry(&self, base: u64) -> u64 {
        self.header.e_entry + if self.is_pie() { base } else { 0 }
    }

    pub fn program_headers(&self) -> impl Iterator<Item = &'a Elf64Phdr> {
        let phoff = self.header.e_phoff as usize;
        let phentsize = self.header.e_phentsize as usize;
        let phnum = self.header.e_phnum as usize;
        let data = self.data;

        (0..phnum).filter_map(move |i| {
            let offset = phoff + i * phentsize;
            if offset + core::mem::size_of::<Elf64Phdr>() > data.len() {
                None
            } else {
                Some(unsafe { &*(data.as_ptr().add(offset) as *const Elf64Phdr) })
            }
        })
    }

    pub fn load_segments(&self) -> impl Iterator<Item = &'a Elf64Phdr> {
        self.program_headers().filter(|ph| ph.p_type == PT_LOAD)
    }
}
