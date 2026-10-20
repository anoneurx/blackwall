# Black Wall Core — Status

## Implemented
- UEFI boot, kernel init phases, serial console
- Memory (PMM/VMM/heap), paging, user access checks
- Process/fork/execve/waitpid, address space reclamation (PAGE_OWNED)
- Scheduler (round-robin), interrupts/syscalls (allow-list)
- VFS (RamFS + Ext2 in-memory), AHCI/PCI/PS2/timer/ACPI
- Network stack, SMP/security/IPC/containers stubs
- Userspace init shell (ls/cat/mkdir/rm/touch/echo/run/pwd/clear/net/anx)
- Package manager CLI `anx` (stub)

## In Progress
- Phase 1: logging levels, CI/xtask/Makefile, panic handler improvements
- Phase 2: PAGE_COW, mmap/munmap stubs, memory hardening

## Planned
- See docs/ROADMAP.md for full plan
