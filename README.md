# Black Wall Core

A compact, self-hostable operating system foundation for building server
appliances and embedded/gateway systems. Black Wall Core is the adaptable,
open-core OS layer extracted from the Black Wall Core product: it provides
the kernel, boot, filesystem, networking, drivers, and a small userland, free
from the proprietary cloud, API, and GUI layers.

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

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
