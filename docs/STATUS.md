# Black Wall Core — Status

Last verified: 2026-10-24 (code-audit + QEMU boot; see `docs/ROADMAP.md` for the plan)

## Implemented
- UEFI boot (OVMF/QEMU), kernel init phases, serial console (stdin/out on COM1)
- Memory: PMM/VMM/heap, paging, user access (`access_ok`) checks
- Process: fork/execve/waitpid E2E, address-space reclamation (PAGE_OWNED)
- Scheduler (round-robin), interrupts, syscalls with allow-list
- VFS: RamFS + in-memory Ext2; sys_open/read/write/readdir/mkdir/unlink
- Drivers: AHCI, PCI, PS/2, timer, ACPI, VGA, VirtIO
- Network stack present (TCP/UDP/IPv4/IPv6/ARP/DNS/ICMP + firewall module)
- Logging: structured levels (error..trace) + `log=` boot-arg parsing (`kernel/src/logging.rs`)
- Panic handler: message + halt (`kernel/src/panic.rs`) — no backtrace yet
- Tracing: per-CPU counters (syscalls, context switches) (`kernel/src/tracing/`)
- SMP: APIC/LAPIC framework, INIT/SIPI trampoline skeleton
- Userspace init shell (bare metal): ~85 builtins wired; real: ls, cat, mkdir, rmdir,
  rm, touch, echo, cd, pwd, whoami, uname, hostname, run (fork/exec/wait), net, tree,
  head, tail, grep, find, stat, df, du, basename, dirname, ls* family, mount-family,
  power controls, watch — see `userspace/init/src/main.rs`
- Host crates: `anx` package-manager CLI, `bwsh`, coreutils, login, services
  (ssh/cron/snapshot/backup), tools (iso-builder/runner/firewall/package-builder/updater)
- CI (`.github/workflows/ci.yml`): fmt, clippy, host builds+tests, kernel/bootloader
  builds, QEMU boot test with `isa-debug-exit` and serial assertions
- 155 host-side `#[test]` functions across workspace crates

## Known Stubs (honest list)
- Shell builtins that print "not implemented": `cp`, `mv`, `ln`, `chmod`, `chown`,
  `passwd`, `resolve`; `anx` inside the boot shell prints a stub message
- `kernel/src/tests/mod.rs`: sanity-check placeholder only (real kernel tests pending)
- PAGE_COW: flag defined (`kernel/src/memory/paging.rs:22`), no COW fault handling yet
- mmap/munmap: syscall numbers/dispatch exist, implementation is a stub
- SMEP/SMAP/NX/W^X: groundwork only, no CR4 enforcement paths
- Panic handler: no backtrace/stack dump
- CI: kernel/bootloader clippy runs with `|| true` (non-blocking)
- `docs/benchmarks/README.md`: placeholder
- Shell polish (found in 2026-10-24 CLI battery, unfixed): `lspci` not wired to the
  boot-time PCI scan, `date` returns epoch placeholder, `uptime` stuck at 00:00,
  `exit` doesn't trigger `isa-debug-exit` for QEMU teardown

## Verified Fixed (2026-10-24)
- Relative-path CLI operations now resolve against the cwd (mkdir/touch/echo >/cat/rm)
- `rmdir` removes empty directories; non-empty dirs return `DirectoryNotEmpty` (ENOTEMPTY)
- First-command race: `ls` sent immediately after the first prompt executes correctly

## In Progress
- Phase 2: copy-on-write fault handling, real mmap/munmap, memory hardening
- Phase 3: SMEP/SMAP/NX/W^X enforcement, syscall audit
- Phase 5: COW-backed mmap, kernel test harness

## Planned
- See docs/ROADMAP.md for the full plan
