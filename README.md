# Black Wall

A compact, self-hostable operating system foundation for building server
appliances and embedded/gateway systems. Black Wall Core is the adaptable,
open-core OS layer extracted from the Black Wall Core product: it provides
the kernel, boot, filesystem, networking, drivers, and a small userland, free
from the proprietary cloud, API, and GUI layers.

## Keywords

`operating system`, `os development`, `rust osdev`, `kernel development`,
`x86_64 kernel`, `monolithic kernel`, `UEFI bootloader`, `bare metal rust`,
`systems programming`, `self-hosted operating system`, `privacy-first os`,
`embedded operating system`, `homebrew OS`, `filesystem`, `network stack`,
`QEMU`, `package manager`, `secure enclaves`

## Why

Black Wall Core is a from-scratch general-purpose operating system written in
Rust, built for people who want to understand or modify every layer of the
stack instead of configuring someone else's. It targets `x86_64` with a UEFI
boot chain and is developed against QEMU, so you can build an ISO and boot it
without physical hardware.

Use it as a base for:

- **Server appliances and gateways** — the userland already ships an SSH
  service, cron, a snapshot manager, a backup manager, and a firewall.
- **Self-hosted infrastructure** — no cloud, no external API, no telemetry.
  Audit the whole system in one repository.
- **Privacy-first deployments** — network stack and access control are part of
  the kernel, not bolted on afterwards.
- **Learning and research** — a readable reference for kernel internals:
  memory management, scheduling, IPC, SMP, syscalls, containers, and drivers.

## What's included

| Area | Location | Contents |
|------|----------|----------|
| Kernel | `kernel/` | x86_64, UEFI; memory, process, scheduler, IPC, security, SMP, syscalls, threads |
| Drivers | `kernel/src/drivers/` | ACPI, AHCI, NVMe, PS/2, PIT timer, RTC, VGA, VirtIO |
| Boot | `boot/` | UEFI bootloader + init (PID 1) |
| Filesystem | `fs/` | VFS layer and filesystem support |
| Networking | `net/` | Network stack |
| Shared | `shared/shared` | Common crate shared across the kernel and userland |
| Userland | `userspace/` | `init` (bare-metal), shell (`bwsh`), coreutils, login |
| Services | `services/` | SSH, cron, snapshot, backup |
| Tooling | `tools/` | QEMU runner, firewall, ISO builder, package builder, updater |
| Package manager | `pkg/` | `anx` CLI + `anx-repo-server` |

## Repository layout

- `kernel/`, `boot/`, `fs/`, `net/`, `shared/shared/` — the core OS
- `userspace/` — user-mode programs (shell, coreutils, login, init)
- `services/` — daemons that ship with the OS
- `tools/` — development and system tooling
- `pkg/` — the package manager and repository server
- `examples/` — small programs showing how to use the crates
- `docs/`, `build/` — documentation and build scripts

See [ARCHITECTURE.md](ARCHITECTURE.md) for a deeper layout and
[CONTRIBUTING.md](CONTRIBUTING.md) for how to contribute.

## Prerequisites

- Rust `stable` toolchain
- Target installed: `x86_64-unknown-uefi`, `x86_64-unknown-none`
  ```sh
  rustup target add x86_64-unknown-uefi x86_64-unknown-none
  ```
- (Optional) `qemu-system-x86_64` for booting the ISO

## Building

The workspace defaults to the core OS members:

```sh
cargo build            # host crates (net, fs, shared, tools, services, pkg)
cargo build -p blackwall-kernel    --target x86_64-unknown-uefi
cargo build -p blackwall-bootloader --target x86_64-unknown-uefi
```

The userland `init` binary is built for bare metal and embedded into the
kernel:

```sh
cd userspace/init && cargo build --target x86_64-unknown-none
```

### Building an ISO

See `docs/build.md` and `tools/iso-builder` for assembling a bootable ISO,
and `tools/runner` (QEMU) for running it.

## Testing

```sh
cargo test --workspace
```

The kernel and bootloader are bare-metal and must be cross-compiled (host
tests cover the library, networking, filesystem, tools and services crates).

## FAQ

**What is Black Wall Core?**
A compact, self-hostable operating system foundation in Rust, providing the
kernel, bootloader, filesystem, network stack, drivers, userland, and services
for building server appliances and embedded gateways.

**Which architecture does it target?**
`x86_64`, booted through UEFI. The bootloader and kernel cross-compile to
`x86_64-unknown-uefi`, and the bare-metal userland `init` to
`x86_64-unknown-none`.

**Is it a microkernel or a monolithic kernel?**
Monolithic. The kernel is a single crate whose subsystems are internal
modules — memory, process, scheduler, IPC, sync, syscall, security, SMP,
containers, drivers, fs, and net — all running in the same address space.

**What language is it written in?**
Rust, throughout the kernel, bootloader, userland, services, tools, and
package manager.

**Does it need external services or an API?**
No. It is designed to run fully self-hosted with no cloud dependency, and
external-API telemetry was removed when the open-core layer was extracted.

**Can I run it without dedicated hardware?**
Yes. `tools/iso-builder` assembles a bootable ISO and `tools/runner` boots it
in `qemu-system-x86_64`.

**What userland programs are included?**
`bwsh` (shell), `bwlogin` (login), bare-metal `init` (PID 1), and around thirty
coreutils including `ls`, `cat`, `cp`, `mv`, and `rm`.

**Does it have a package manager?**
Yes — `anx`, with `anx-repo-server` for hosting a local repository and
`tools/package-builder` to produce `.anxpkg` packages.

**What license is it under?**
MIT.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
