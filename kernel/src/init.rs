#[cfg(target_os = "uefi")]
use crate::arch::x86_64::{cpu, gdt, idt, interrupts, serial, timer, trap};
#[cfg(target_os = "uefi")]
use crate::memory;
#[cfg(target_os = "uefi")]
use crate::scheduler::RoundRobinScheduler;
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
    // Capture the ACPI RSDP from the EFI configuration table while boot
    // services (which own that table) are still alive.
    crate::drivers::acpi::capture_rsdp(&system_table);

    // SAFETY: No boot-service pointers or references are live at this point.
    let (_runtime_table, _memory_map) =
        unsafe { system_table.exit_boot_services(MemoryType::LOADER_DATA) };

    // Re-init serial now that UEFI has released the UART.
    serial::init();

    gdt::init();
    idt::init();
    timer::init();
    crate::arch::x86_64::syscall::init();

    // The page table the firmware left us in.  Kernel threads must switch
    // back to it whenever they leave a user task.
    let kernel_cr3 = trap::current_cr3();

    // ── Boot banner ──────────────────────────────────────────────────────────
    serial::line(messages::BOOT_BANNER);
    serial::line("");
    serial::line(messages::MEMORY_MANAGER_READY);
    serial::line(messages::SCHEDULER_READY);
    serial::line(messages::KERNEL_THREADS_ENABLED);
    serial::line(messages::MULTITASKING_ENABLED);
    serial::line("Syscall Interface Ready");
    serial::line("");

    // ── Scheduler + boot/idle context (PID 0) ────────────────────────────────
    // The context we are executing in right now is registered as an ordinary
    // task so the scheduler can save and restore it like any other.  It never
    // returns to ring 3 and never issues syscalls, so it needs no private
    // kernel stack.
    let mut sched = RoundRobinScheduler::new();
    let boot_pid = sched.manager_mut().register_running("idle", kernel_cr3);
    serial::line(messages::PID_0_IDLE);

    // Publish the (not yet enabled) scheduler so later subsystems can reach it.
    *crate::scheduler::SCHEDULER.lock() = Some(sched);

    // ── Initialize VFS and mount filesystems (Phase 6) ───────────────────────
    crate::fs::init(INIT_ELF);

    // Read the whole init ELF from the VFS.  The size comes from the inode so
    // the buffer is always large enough — the ELF's section headers (and thus
    // its PIE relocations) live at the very end of the file and must be intact.
    let init_buf = {
        let vfs = crate::fs::vfs::VFS.lock();
        let vfs_mgr = vfs.as_ref().expect("VFS not initialized");
        vfs_mgr.read_all("/bin/init").expect("Failed to read /bin/init")
    };
    let bytes_read = init_buf.len();

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
    // Drivers run with IRQs masked; allow the PS/2 keyboard through now.
    timer::unmask_keyboard();

    // ── Mount a persistent Ext2 from the AHCI data disk (if present) ─────────
    crate::fs::mount_disk();
    {
        let mut buf = alloc::vec![0u8; 256];
        let got = {
            let vfs = crate::fs::vfs::VFS.lock();
            let mgr = vfs.as_ref().expect("VFS not initialized");
            match mgr.resolve_path("/mnt/hello.txt") {
                Ok(vnode) => vnode.fs.read(vnode.inode, &mut buf, 0).ok(),
                Err(_) => None,
            }
        };
        if let Some(len) = got {
            if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                serial::line(&alloc::format!("[DEBUG] /mnt/hello.txt: {}", s.trim_end()));
            }
        }
    }

    // ── Initialize Network Stack (Phase 9) ───────────────────────────────────
    crate::net::init();

    // ── Phase 10: Remaining core subsystems ─────────────────────────────────────
    crate::smp::init();
    crate::security::init();
    crate::ipc::init();
    crate::containers::init();

    serial::line("Launching userspace init process...");
    serial::line("");

    // ── Load the init ELF into its own address space ─────────────────────────
    let image = unsafe {
        crate::process::loader::build_user_image(
            &init_buf[..bytes_read],
            &mut memory_state.virtual_memory,
            &mut memory_state.physical,
        )
    };

    // ── Register init as a user task and enable scheduling ───────────────────
    let init_pid = {
        let mut guard = crate::scheduler::SCHEDULER.lock();
        let s = guard.as_mut().expect("scheduler missing");
        let pid = s.spawn_user_task("init", image.cr3, image.entry, image.user_rsp, 0, 32 * 1024);
        s.start(boot_pid);
        pid
    };

    serial::line(&alloc::format!("PID {} init", init_pid));

    // ── Turn on interrupts and become the idle task ──────────────────────────
    // The first timer tick (or the explicit yield below) switches the CPU to
    // the init task; from here on the boot context only runs when nothing else
    // is runnable.
    serial::line("interrupts on; entering idle");
    interrupts::enable();
    trap::yield_now();

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
