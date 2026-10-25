# Black Wall — Roadmap

Status: ACTIVE (Phase 0 complete, Phase 1 mostly complete). Living document. Updated for every change per CONTRIBUTING.md. Last verified by code-audit: 2026-10-24.

## Guiding Principles
- Server-first, robotics/embedded later (ARM64/RISC-V)
- Be honest: never claim unimplemented features
- Small, reviewable commits; never leave main unbootable
- `unsafe` minimal, isolated, each block with `// SAFETY:`
- Tests for every new feature; cargo fmt/clippy -D warnings clean; QEMU boot validated
- Ask before making breaking changes to the syscall ABI or on-disk formats

## Current Baseline (Phase 0)
- Boots UEFI+QEMU+OVMF, reaches `blackwall>` shell
- fork/execve/waitpid E2E; execve/exit address-space leak closed (PAGE_OWNED)
- VFS (RamFS+Ext2), AHCI/PCI/PS2/timer/ACPI, network present
- Syscall allow-list + user_access checks
- `anx` stub added (Linux commands kept)

## Phase 1: Stability & Tooling (P0) — MOSTLY COMPLETE (verified 2026-10-24)

### 1.1 Fix first shell command "command not found" race [FIX APPLIED]
- Fix applied: init shell drains leading CR/LF after banner (first-byte check in `read_line`)
- Regression test (10/10 cold-boot QEMU serial capture): NOT yet automated — pending
- Acceptance: 10/10 cold boots → first command executes correctly (partially verified 2026-10-24, see QEMU CLI test notes in git history)

### 1.2 Kernel logging with levels + boot args [DONE]
- `kernel/src/logging.rs`: levels error|warn|info|debug|trace, boot-arg parsing (`log=debug`)
- Structured replaces ad-hoc prints in kernel init paths

### 1.3 GitHub Actions CI [DONE, with caveat]
- `.github/workflows/ci.yml`: fmt, clippy (-D warnings, host), builds (host + kernel/bootloader targets), `cargo test --workspace`, QEMU boot with serial assertion + isa-debug-exit, cargo audit/deny
- Caveat: kernel/bootloader clippy and audit/deny run with `|| true` (non-blocking)

### 1.4 Panic handler + backtraces + test harness [PARTIAL]
- Panic: message + location + halt (`kernel/src/panic.rs`) — no backtrace/stack dump yet
- In-kernel test harness: placeholder (`kernel/src/tests/mod.rs` = sanity_check); host tests: 155 test fns across workspace crates


### Phase 2: Memory — GROUNDWORK (verified 2026-10-24)
- 2.1 COW: `PAGE_COW` flag defined (`kernel/src/memory/paging.rs:22`); fault-path COW handling NOT implemented
- 2.2 mmap/munmap: syscall numbers + dispatch entries exist (`kernel/src/syscall/mod.rs`); implementation is a stub on ENOSYS
- 2.3 Memory hardening: pending
- Remaining: COW fault handling in page-fault path, real mmap/munmap with VMA tracking, guard pages, heap hardening

## Phase 3: Security — GROUNDWORK (verified 2026-10-24)
- Threat model draft + SECURITY.md policy: done
- SMEP/SMAP/NX/W^X: groundwork notes only; no CR4 enforcement paths in paging/interrupt entry

## Phase 4: Performance & Tracing — PARTIAL
- Per-CPU counters for syscalls/context switches wired; benchmarks doc is a placeholder

## Phase 5: Userspace — PARTIAL
- Boot shell: ~85 builtins wired; 7 print "not implemented" (cp/mv/ln/chmod/chown/passwd/resolve), `anx` in-shell is a stub
- Host-side `bwsh`, coreutils (25), login, services: present in repo, not exercised by the QEMU boot test

## Phase 6: OSS — ~90%
- SECURITY.md, threat model, syscall reference, ARM64 port plan, PR/issue templates, CoC, CONTRIBUTING: done
- Pending: `docs/v2_release_plan.md` was missing from the repo (created 2026-10-24); benchmarks data

## Verification Log
- 2026-10-24: full workspace + kernel/bootloader + init built clean (2 dead-code warnings in init, 1 in bw-pkg); ISO assembled (`bw-iso-builder`); QEMU boot + CLI driving via serial → see CHANGELOG "Verified" entry and commit history for results
- 2026-10-24 QEMU CLI battery (TCG + `-serial stdio`, commands typed over serial):
  - PASS: prompt ~3 s; first `ls` sent immediately after prompt executes correctly (race fix verified)
  - PASS: pwd, echo (incl. `>` redirect), cat, mkdir/touch/rm (absolute), ls (root + subdir), cat of Ext2 `/mnt/hello.txt`, uname, whoami, hostname, help, `net state` (live stack info), unknown-command rejection, `exit` (task exits, status 0)
  - PASS: honest stubs behave as documented (cp/mv → "not implemented in this shell build")
  - BUG FOUND + FIXED: all relative paths (`mkdir testdir`, `touch testdir/f`, `echo > x`) failed — init's fs helpers bypassed `resolve_path()`; fixed in `userspace/init/src/main.rs` (`path_buf` now resolves)
  - BUG FOUND + FIXED: `rmdir` of an empty directory returned `IsADirectory` — `VfsManager::rmdir` delegated to `unlink`, and RamFs refused dirs; added empty-dir removal to RamFs (`fs/src/ramfs.rs`), new `VfsError::DirectoryNotEmpty` + `ENOTEMPTY` mapping; 4 regression tests in `fs/tests/ramfs_rmdir_test.rs`
  - Verified fixed (2026-10-24 QEMU CLI battery, `-serial stdio`):
  - `lspci` now shows the 6 PCI devices enumerated at boot.
  - `date` reports the real wall-clock time from the RTC (e.g. `2026-10-24 21:00:03 UTC`).
  - `uptime` reports actual uptime (e.g. `0 days, 00:02` after ~2 min boot).
  - `exit` triggers `isa-debug-exit`, causing QEMU to terminate cleanly (no signal 15).
- 2026-10-25 QEMU boot diagnostics (why VMs showed "no boot found"):
  - Root cause 1: the ISO is UEFI-only (El Torito entries point at an EFI boot image `BOOT/EFIBOOT.IMG`; no BIOS/ISOLINUX entry). Legacy-BIOS (SeaBIOS) VMs report "no bootable device". virt-manager defaults to BIOS firmware for new x86 VMs → must switch Overview → Firmware to UEFI/OVMF.
  - Root cause 2 (fixed): `tools/runner` hand-rolled xorriso call produced a non-bootable catalog (`-eltorito-alt-boot -e` with default 4-sector load size, no FAT ESP image) — `BdsDxe: No bootable option or device was found`. Fixed by delegating ISO build to `bw-iso-builder`; OVMF boots it via the standard `\EFI\BOOT\BOOTX64.EFI` fallback path.
  - Root cause 3 (fixed): every runner run copied a fresh OVMF var store, halting at the `Please select boot device` menu; store is now persistent (`target/blackwall-ovmf-vars.fd`, copied once).
  - measured: shell `exit` → qemu exits with status 1 (isa-debug-exit writes 0 → code value+1); runner now treats code 1 as clean powerdown.
  - PASS (end-to-end): `target/debug/blackwall-runner run-headless` with piped `help`,`exit` → prompt appears, help output correct, clean powerdown (runner exit 0).
