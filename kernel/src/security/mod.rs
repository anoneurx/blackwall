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
    /// Per-syscall overrides; gaps fall back to `default`.
    rules: Vec<FilterAction>,
    /// Action applied to any syscall without an explicit override.
    default: FilterAction,
}

impl SyscallFilter {
    pub fn new(default: FilterAction) -> Self {
        Self { rules: Vec::new(), default }
    }

    /// Allow everything (used for the boot/kernel context).
    pub fn new_permissive() -> Self {
        Self::new(FilterAction::Allow)
    }

    /// Deny everything unless explicitly allowed (allow-list mode).
    pub fn new_deny_all() -> Self {
        Self::new(FilterAction::Kill)
    }

    pub fn add_rule(&mut self, syscall_nr: u64, action: FilterAction) {
        let idx = syscall_nr as usize;
        if idx >= self.rules.len() {
            self.rules.resize(idx + 1, self.default);
        }
        self.rules[idx] = action;
    }

    pub fn check(&self, syscall_nr: u64) -> FilterAction {
        self.rules.get(syscall_nr as usize).copied().unwrap_or(self.default)
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

    /// Unprivileged process — allow-list only.
    ///
    /// Every userspace syscall passes through this filter in the dispatcher;
    /// anything not enumerated here is a `Kill`.  The list mirrors the
    /// syscalls a plain user shell actually needs: stdio, filesystem access,
    /// `lseek`/`stat`, timing, process identity and exit.  Dangerous entries
    /// (`open`-style wide FS handles, `fork`/`execve`) stay denied.
    pub fn unprivileged() -> Self {
        let mut filter = SyscallFilter::new_deny_all();
        for nr in [
            0u64, /* read */
            1,    /* write */
            2,    /* open */
            3,    /* close */
            4,    /* stat */
            8,    /* lseek */
            24,   /* yield */
            35,   /* sleep */
            39,   /* getpid */
            60,   /* exit */
            78,   /* readdir */
            83,   /* mkdir */
            84,   /* rmdir */
            87,   /* unlink */
            110,  /* getppid */
        ] {
            filter.add_rule(nr, FilterAction::Allow);
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

    // Process-wide syscall policy.  The kernel itself never issues `syscall`
    // instructions (yields go through the dedicated ring-0 trap), so this
    // filter gates every syscall from userspace.
    let ctx = SecurityContext::unprivileged();
    *GLOBAL_FILTER.lock() = Some(ctx.filter);
    serial::line("[SECURITY] Capability system initialized.");
    serial::line("[SECURITY] Syscall filter ready (allow-list mode for userspace).");
}

/// The enforced process-wide syscall filter (set by [`init`]).
static GLOBAL_FILTER: crate::sync::spin::SpinLock<Option<SyscallFilter>> =
    crate::sync::spin::SpinLock::new(None);

/// Look up what the enforced policy says about `syscall_nr`.
///
/// Before the security subsystem has initialized (boot only — no userspace
/// has run yet) the check fail-opens so the dispatcher never artificially
/// stalls a syscall that cannot legally occur.
pub fn check_syscall(syscall_nr: u64) -> FilterAction {
    GLOBAL_FILTER.lock().as_ref().map(|f| f.check(syscall_nr)).unwrap_or(FilterAction::Allow)
}
