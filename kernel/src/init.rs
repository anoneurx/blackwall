#[cfg(target_os = "uefi")]
use crate::arch::x86_64::{cpu, gdt, idt, serial, timer};
#[cfg(target_os = "uefi")]
use crate::memory;
#[cfg(target_os = "uefi")]
use blackwall_shared::messages;
#[cfg(target_os = "uefi")]
use uefi::prelude::*;
#[cfg(target_os = "uefi")]
use uefi::table::boot::MemoryType;

/// The init process ELF binary, embedded at compile time.
///
/// Build first with: `cargo build -p init --target x86_64-unknown-none`
/// The binary lives at `target/x86_64-unknown-none/debug/init`.
#[cfg(target_os = "uefi")]
static INIT_ELF: &[u8] =
    include_bytes!("../../userspace/init/target/x86_64-unknown-none/debug/init");

/// Dedicated kernel stack for the Ring 3 → Ring 0 (syscall / interrupt) transition.
/// 32 KiB — large enough for deep kernel call stacks from userspace.
#[cfg(target_os = "uefi")]
static mut KERNEL_ENTRY_STACK: [u8; 4096 * 8] = [0u8; 4096 * 8];

#[cfg(target_os = "uefi")]
pub fn start(system_table: SystemTable<Boot>) -> ! {
    serial::init();

    let _cpu_info = cpu::detect();
    let mut memory_state = memory::init(&system_table);

    // ── Exit UEFI Boot Services ───────────────────────────────────────────────
    // All boot-services calls (allocate_pages, memory_map) are done by now.
    // We must exit before touching ANY hardware I/O (PIC, PIT, serial ports)
    // because UEFI still owns those until we surrender them here.
    //
    // SAFETY: No boot-service pointers or references are live at this point.
    let (_runtime_table, _memory_map) =
        unsafe { system_table.exit_boot_services(MemoryType::LOADER_DATA) };

    // Re-init serial now that UEFI has released the UART.
    serial::init();

    gdt::init();
    idt::init();
    timer::init();
    crate::arch::x86_64::syscall::init();

    // ── Boot banner ──────────────────────────────────────────────────────────
    serial::line(messages::BOOT_BANNER);
    serial::line("");
    serial::line(messages::MEMORY_MANAGER_READY);
    serial::line(messages::SCHEDULER_READY);
    serial::line(messages::KERNEL_THREADS_ENABLED);
    serial::line(messages::MULTITASKING_ENABLED);
    serial::line("Syscall Interface Ready");
    serial::line("");

    // ── Kernel idle task (PID 0) ─────────────────────────────────────────────
    let mut sched = crate::scheduler::RoundRobinScheduler::new();
    sched.spawn_kernel_thread("idle", idle_task, 0);
    serial::line(messages::PID_0_IDLE);

    // Store globally so timer ISR can reach the scheduler.
    *crate::scheduler::SCHEDULER.lock() = Some(sched);

    // ── Initialize VFS and mount filesystems (Phase 6) ───────────────────────
    crate::fs::init(INIT_ELF);

    // Read the init ELF from VFS to verify VFS read works!
    let mut init_buf = alloc::vec![0u8; 128 * 1024]; // allocate 128 KiB buffer
    let bytes_read = {
        let vfs = crate::fs::vfs::VFS.lock();
        let vfs_mgr = vfs.as_ref().expect("VFS not initialized");
        let vnode = vfs_mgr.resolve_path("/bin/init").expect("Failed to resolve /bin/init");
        vnode.fs.read(vnode.inode, &mut init_buf, 0).expect("Failed to read /bin/init")
    };

    serial::line(&alloc::format!(
        "[DEBUG] Successfully read {} bytes of init ELF from VFS",
        bytes_read
    ));

    // Read test file from Ext2 partition!
    let mut ext2_buf = alloc::vec![0u8; 256];
    let ext2_read = {
        let vfs = crate::fs::vfs::VFS.lock();
        let vfs_mgr = vfs.as_ref().expect("VFS not initialized");
        if let Ok(vnode) = vfs_mgr.resolve_path("/mnt/hello.txt") {
            vnode.fs.read(vnode.inode, &mut ext2_buf, 0).ok()
        } else {
            None
        }
    };
    if let Some(len) = ext2_read {
        if let Ok(s) = core::str::from_utf8(&ext2_buf[..len]) {
            serial::line(&alloc::format!("[DEBUG] Read from Ext2 file (/mnt/hello.txt): {}", s));
        }
    }

    // ── Initialize Device Drivers (Phase 8) ──────────────────────────────────
    crate::drivers::init();

    // ── Initialize Network Stack (Phase 9) ───────────────────────────────────
    crate::net::init();

    // ── Phase 10: Remaining core subsystems ─────────────────────────────────────
    crate::smp::init();
    crate::security::init();
    crate::ipc::init();
    crate::containers::init();

    serial::line("Launching userspace init process...");
    serial::line("");

    // ── Kernel entry-stack top pointer ────────────────────────────────────────
    let kstack_top = {
        let base = core::ptr::addr_of!(KERNEL_ENTRY_STACK) as u64;
        base + (4096 * 8) as u64
    };

    // ── Hand off to Ring 3 via iretq ─────────────────────────────────────────
    // `spawn_user_process` parses the ELF, maps user pages, and executes
    // `iretq` — it does NOT return.
    unsafe {
        crate::process::loader::spawn_user_process(
            &init_buf[..bytes_read],
            &mut memory_state.virtual_memory,
            &mut memory_state.physical,
            kstack_top,
        )
    }
}

#[cfg(target_os = "uefi")]
pub fn idle_task() {
    loop {
        crate::net::poll();
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

#[cfg(not(target_os = "uefi"))]
pub fn start() -> ! {
    panic!("blackwall-kernel is only available for the UEFI target");
}
