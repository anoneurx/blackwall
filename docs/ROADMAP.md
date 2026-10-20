# Black Wall — Roadmap

Status: ACTIVE (Phase 0 complete). Living document. Updated for every change per CONTRIBUTING.md.

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

## Phase 1: Stability & Tooling (P0) — IN PROGRESS

### 1.1 Fix first shell command "command not found" race [TODO]
- Root cause: serial RX buffer/echo, PS/2 init race, newline/CRLF, or `read_line` consuming stray byte after banner/init
- Repro: boot → type `ls` immediately → may return "command not found", then works
- Fix: drain UART/PS2 on init, ignore leading CR/LF, flush buffers
- Tests: regression in userspace/init (QEMU serial capture)
- Acceptance: 10/10 cold boots → first command executes correctly

### 1.2 Kernel logging with levels + boot args [TODO]
- Levels: error|warn|info|debug|trace, compile-time default + runtime boot args (`log=debug`)
- Replace ad-hoc prints with structured logger; quiet default
- Acceptance: tunable verbosity, reduced default noise

### 1.3 GitHub Actions CI [TODO]
- Jobs: fmt, clippy (-D warnings), build (host+targets), `cargo test --workspace`, `cargo audit`, `cargo deny`, QEMU boot (serial output assertions) with isa-debug-exit, ISO artifact
- Caching, required checks on PRs
- Acceptance: green on main, blocks unbootable merges

### 1.4 Panic handler + backtraces + test harness [TODO]
- Panic: backtrace, serial dump, defined policy; no silent infinite loop
- In-kernel unit/integration + host tests; kernel test harness
- Acceptance: useful traces; harness runs in CI

### Phase 2 Progress
- 2.1: Added PAGE_COW flag; COW fork/mmap/guard pages in progress
