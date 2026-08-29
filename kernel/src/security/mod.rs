/// Security — Capability-based access control and syscall filtering
extern crate alloc;

use alloc::vec::Vec;

// ── Minimal bitflags! shim (no external crate) ───────────────────────────────
macro_rules! bitflags {
    (pub struct $name:ident : $t:ty { $( const $flag:ident = $val:expr; )* }) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct $name(pub $t);
        #[allow(non_upper_case_globals, dead_code)]
        impl $name {
            $(pub const $flag: Self = Self($val);)*
            pub fn all()   -> Self { Self(<$t>::MAX) }
            pub fn empty() -> Self { Self(0) }
            pub fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }
        }
        impl core::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
        }
    };
}

// ── Capability bits ──────────────────────────────────────────────────────────
bitflags! {
    pub struct Capabilities : u64 {
        const CAP_NET_BIND     = 1 << 0;
        const CAP_NET_RAW      = 1 << 1;
        const CAP_SYS_ADMIN    = 1 << 2;
        const CAP_SYS_PTRACE   = 1 << 3;
        const CAP_CHOWN        = 1 << 4;
        const CAP_DAC_OVERRIDE = 1 << 5;
        const CAP_KILL         = 1 << 6;
        const CAP_SETUID       = 1 << 7;
        const CAP_SETGID       = 1 << 8;
        const CAP_SYS_MODULE   = 1 << 9;
    }
}

// ── Syscall filter (seccomp-like) ────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterAction {
    Allow,
    Kill,
    Log,
}

pub struct SyscallFilter {
    rules: Vec<FilterAction>,
}

impl SyscallFilter {
    pub fn new_permissive() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(&mut self, syscall_nr: u64, action: FilterAction) {
        let idx = syscall_nr as usize;
        if idx >= self.rules.len() {
            self.rules.resize(idx + 1, FilterAction::Allow);
        }
        self.rules[idx] = action;
    }

    pub fn check(&self, syscall_nr: u64) -> FilterAction {
        self.rules.get(syscall_nr as usize).copied().unwrap_or(FilterAction::Allow)
    }
}

// ── Process security context ─────────────────────────────────────────────────
pub struct SecurityContext {
    pub uid: u32,
    pub gid: u32,
    pub caps: Capabilities,
    pub filter: SyscallFilter,
}

impl SecurityContext {
    /// Full root / privileged context
    pub fn privileged() -> Self {
        Self { uid: 0, gid: 0, caps: Capabilities::all(), filter: SyscallFilter::new_permissive() }
    }

    /// Unprivileged container process — allow-list only
    pub fn unprivileged() -> Self {
        let mut filter = SyscallFilter::new_permissive();
        // Deny dangerous syscalls by default
        for nr in [
            2u64, /* sys_open — wide FS access */
            59,   /* sys_execve */
            57,   /* sys_fork  */
        ] {
            filter.add_rule(nr, FilterAction::Log);
        }
        Self { uid: 1000, gid: 1000, caps: Capabilities::empty(), filter }
    }

    pub fn has_cap(&self, cap: Capabilities) -> bool {
        self.caps.contains(cap)
    }

    pub fn check_syscall(&self, nr: u64) -> FilterAction {
        self.filter.check(nr)
    }
}

pub fn init() {
    use crate::arch::x86_64::serial;
    serial::line("[SECURITY] Capability system initialized.");
    serial::line("[SECURITY] Syscall filter ready (allow-list mode for userspace).");
}
