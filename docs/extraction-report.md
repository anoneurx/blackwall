# Extraction report: Black Wall Core OS foundation

This report records what was extracted, removed, and refactored when the
self-hostable OS foundation was carved out of the Black Wall Server product into
this repository. The original codebase was never modified; everything here lives
in a fresh git repo with no history.

- Source: `https://github.com/anoneurx/blackwallserver.git` (private)
- Target: this repository (Black Wall Core)
- Method: extraction, not rewrite — working core code is preserved verbatim.

## Extracted (kept in the open core)

Core OS layer, self-hostable and free of cloud/UI/API dependencies:

| Area | Path | Contents |
|------|------|----------|
| Kernel | `kernel/` | x86_64, UEFI; memory, process, scheduler, IPC, security, SMP, syscalls, containers, threads |
| Drivers | `kernel/src/drivers/` | ACPI, AHCI, NVMe, PS/2, PIT timer, RTC, VGA, VirtIO |
| Boot | `boot/bootloader`, `boot/init`, `boot/uefi` | UEFI bootloader, PID 1 init |
| Filesystem | `fs/` | VFS layer, RAM/Ext2/Ext4/`bwfs` support |
| Networking | `net/` | Ethernet/ARP/IPv4/IPv6/TCP/UDP/ICMP/DNS/firewall/stack |
| Shared | `shared/shared/` | Crate shared by kernel + userland |
| Userland | `userspace/` | shell (`bwsh`), coreutils, login, bare-metal `init` |
| Services | `services/` | SSH, cron, snapshot, backup |
| Tooling | `tools/` | QEMU runner, firewall, ISO builder, package builder, updater |
| Package manager | `pkg/anx`, `pkg/server` | `anx` CLI + `anx-repo-server` |
| Build | `build/scripts/`, Makefile, rust-toolchain/rustfmt | build automation |

## Removed (excluded from the open core)

| Removed | Reason |
|---------|--------|
| `api/` | Server API (WebSocket metrics/logs/services, `X-API-Token` auth) — cloud/management layer |
| `sdk/` | `bw-sdk` client SDK — depends on the API |
| `tools/cluster/`, `tools/bwctl` | Cluster + umbrella management — depend on the cloud layer |
| `installer/` | Slint GUI installer + dashboard UI — UI, excluded |
| `services/monitor/` | Telemetry monitor that pushed metrics to the Server API — cloud dependency |
| `package-manager/installer` (web) | Installer web UI |
| `package-manager/client`, `package-manager/repository` (web) | Repository/dashboard static UI with ANONEURX/cloud catalog & branding |
| `packages/` (recipes), `target/`, build artifacts | Product package recipes + build output — out of scope for the core |
| Empty placeholder dirs (`fs/vfs`, `tests/`, `userspace/{package-manager,services}`, `boot/uefi`, `tools/image-builder`) | No content; not committed |

## Refactored (behavior-preserving fixes required by extraction)

| Change | File | Why |
|--------|------|-----|
| Neutral default repo URL | `pkg/anx/src/repo.rs` | `https://repo.blackwallserver.io` → `http://localhost:8484`; removed `enterprise` channel example (private/hosted repo) |
| Portable linker-script path | `userspace/init/.cargo/config.toml` + new `build.rs` | Hard-coded `-T/home/kashie/blackwallserver/userspace/init/init.ld` → resolved from `CARGO_MANIFEST_DIR` at build time |
| Product rebrand (docs/comments) | all `.rs/.toml/.md/.sh/.html/.js` | "Black Wall Server" → "Black Wall Core" (cosmetic, non-code) |
| Metadata scrub | `pkg/anx/Cargo.toml` | Removed `repository = github.com/anoneurx/blackwallserver` (private) and old description |
| Neutralize branding | `build/scripts/enroll-mok.sh` | `O=Anoneurx` → `O=Black Wall Core` in Secure Boot MOK cert subject |
| Comment-only cleanups | `kernel/src/init.rs`, `tools/iso-builder`, `pkg/anx` | "Enterprise subsystems"/"enterprise" phrasing → neutral |

No functionality was rewritten. The only behavioral divergence from source is
the removal of cloud/API/UI calls (monitor, repo URL) and the relocation of the
linker-script path (equivalent behavior, portable).

## Dependency graph (as extracted)

Path dependencies remain internal to the repo (relative paths preserved):

- `boot/bootloader` → `blackwall-kernel`, `blackwall-shared`
- `boot/init`, `userland/init` → `blackwall-shared`
- `kernel` → `blackwall-shared`, `blackwall-net`, `blackwall-fs`
- No crate depends on `api`, `sdk`, `installer`, `tools/cluster`, or `tools/bwctl`

All external crates are standard (clap, serde, axum, tokio, reqwest, zstd,
zip, etc.). No ANONEURX/private/internal crates.

## Architecture

See [ARCHITECTURE.md](../ARCHITECTURE.md). Summary:

- Monolithic kernel crate with modular internals (arch, drivers, memory,
  process, scheduler, IPC, security, SMP, sync, syscall, thread, containers),
  built for `x86_64-unknown-uefi`.
- UEFI bootloader loads the kernel; `kernel/src/init.rs` runs boot phases then
  spawns the userland `init` (embedded via `include_bytes!`) in Ring 3.
- `userland/init` is an independent bare-metal binary (`x86_64-unknown-none`).
- Filesystem (`fs/`) and networking (`net/`) are separate portable crates
  reused by the kernel.
- Userland (`userspace/`), services (`services/`), package manager (`pkg/`),
  and tooling (`tools/`) run on host and on-device.

## Verified

| Check | Result |
|-------|--------|
| Host crates build (`cargo build --workspace`, non-bare-metal) | PASS |
| Kernel builds `x86_64-unknown-uefi` | PASS (26 dead-code warnings, expected) |
| Bootloader builds `x86_64-unknown-uefi` | PASS |
| Userland `init` builds `x86_64-unknown-none` (portable link) | PASS |
| Workspace tests (`cargo test --workspace`, non-bare-metal) | **129 passed, 0 failed** |
| Examples run (VFS demo + network stack demo) | PASS |
| `cargo fmt --check` on touched crates | PASS |
| No ANONEURX / blackwallserver.io / private-path / cloud refs remain | PASS |

Hardware boots (ISO under QEMU) were **not** executed here: `qemu-system-x86_64`
is not installed in this environment. See `examples/bootable/README.md` for the
exact steps to add a boot phase and boot under QEMU.

## Licensing

- Cargo metadata declares `license = "MIT"`; no LICENSE file or copyright
  headers existed in the source (verified). A standard MIT `LICENSE` with a
  neutral "Black Wall Core contributors" copyright was added; the declared
  license was not changed.

## Known remaining work

- Boot the assembled ISO under QEMU to verify hardware bring-up (needs QEMU).
- CI wiring (formatting + clippy + test matrix for host and cross targets).
- Optional: package/release `.anxpkg` builds for the shipped crates.
- Optional: split the monolithic kernel crate into discrete crates if desired —
  deliberately not done to avoid rewriting working code.
