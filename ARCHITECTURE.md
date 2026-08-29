# Architecture

Black Wall Core is a small, monolithic-style kernel with a modular internal
layout, a UEFI boot chain, and a compact userland. It is designed to be
self-hostable and free of external cloud or API dependencies.

## Boot chain

1. **`boot/bootloader`** (blackwall-bootloader) — UEFI bootloader, built for
   `x86_64-unknown-uefi`. It loads the kernel into memory and hands off to
   the kernel entry point.
2. **`kernel/`** (blackwall-kernel) — built for `x86_64-unknown-uefi`.
   `kernel/src/init.rs` runs the boot phases (memory, VFS, drivers, network,
   SMP, security, IPC, containers), then spawns the userland init process in
   Ring 3.
3. **`userspace/init`** — a strictly separate bare-metal binary built for
   `x86_64-unknown-none` and **embedded into the kernel** via `include_bytes!`
   (see `kernel/src/init.rs`). It becomes PID 1.

Because the userland init ELF is embedded at compile time, change the
`include_bytes!` path if you relocate the `userspace/init` build.

## Kernel internals

The kernel is a single crate whose subsystems are modules — this keeps the
boot-time hand-off simple and matches an OS where everything runs in the same
address space:

| Module | Role |
|--------|------|
| `arch/x86_64` | low-level CPU, paging, interrupts |
| `memory` | physical + virtual memory managers |
| `process` / `scheduler` / `thread` | process and thread lifecycle, scheduling |
| `ipc` / `sync` / `syscall` | inter-process communication, primitives, syscall table |
| `security` / `smp` / `containers` | access control, SMP bootstrap, isolation |
| `drivers` | ACPI, AHCI, NVMe, PS/2, PIT timer, RTC, VGA, VirtIO |
| `fs` / `net` | in-kernel filesystem and network hooks (thin; see below) |
| `init` | boot-phase orchestration + userspace hand-off |

## Filesystem and networking

Higher-level filesystem and networking logic live in dedicated crates so they
can be reused outside the bare-metal kernel:

- **`fs/`** (blackwall-fs) — VFS layer and filesystem helpers.
- **`net/`** (blackwall-net) — network stack.
- **`shared/shared`** (blackwall-shared) — types and utilities shared by the
  kernel and userland.

The kernel's `kernel/src/fs` and `kernel/src/net` are the in-kernel entry
points that coordinate with these crates.

## Userland

`userspace/` contains the user-mode programs:

- `shell/` — `bwsh`, an interactive shell
- `coreutils/` — ~30 classic utilities (`ls`, `cat`, `cp`, `mv`, `rm`, ...)
- `login/` — `bwlogin`, the login process
- `init/` — bare-metal PID 1, embedded into the kernel

## Services

`services/` ships the daemons installed by default:

| Service | Binary | Purpose |
|---------|--------|---------|
| `services/ssh` | `bwssh` | SSH service wrapper |
| `services/cron` | `bwcron` | cron daemon |
| `services/snapshot` | `bwsnap` | snapshot manager |
| `services/backup` | `bwbackup` | backup manager |

Telemetry/monitoring that pushed metrics to an external API was removed during
extraction (see the extraction report).

## Package management

`pkg/` provides the self-hosted package system:

- `pkg/anx` — `anx` CLI (package install/search/build)
- `pkg/server` — `anx-repo-server`, a local repository server
- `tools/package-builder` — `bw-pkg`, builds `.anxpkg` packages

The default repository URL is a **neutral local default**
(`http://localhost:8484`); point it at your own repository.
`tools/updater` (`anxd`) is the auto-update daemon.

## Tools

`tools/` is split between system and development tools:

- Dev/CI: `runner` (QEMU), `iso-builder`, `package-builder`
- System: `firewall` (`bwfw`), `updater` (`anxd`)

Cluster management (`tools/cluster`) and the umbrella management CLI
(`tools/bwctl`) were excluded from the open core — they depend on the cloud
layer.

## Build system

The root `Cargo.toml` is a workspace. `profile.dev` and `profile.release` set
`panic = "abort"` (required for the bare-metal kernel). Build scripts live in
`build/scripts/` and `tools/iso-builder`, `tools/runner`.
