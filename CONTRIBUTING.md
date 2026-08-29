# Contributing

Thank you for helping build Black Wall Core.

## Getting started

1. Fork and clone the repository.
2. Install the Rust `stable` toolchain and the targets used by the project:
   ```sh
   rustup target add x86_64-unknown-uefi x86_64-unknown-none
   ```
3. Build the host crates and run the tests:
   ```sh
   cargo build
   cargo test --workspace
   ```

## Development workflow

- Keep the kernel and the crates in `fs/`, `net/`, `shared/` building for
  `x86_64-unknown-uefi` / `x86_64-unknown-none`. Use `cargo fmt` and `cargo
  clippy` before submitting.
- Cross-compile changes that touch the kernel:
  ```sh
  cargo build -p blackwall-kernel --target x86_64-unknown-uefi
  ```
  Use `tools/runner` (QEMU) to boot and verify bare-metal changes.
- Do not add dependencies on the excluded layers (cloud/API/UI). Keep all
  code self-hostable and offline-capable.

## Code style

- Follow existing conventions in the crate you are editing (see ARCHITECTURE.md).
- Do not add new secrets, private repository URLs, or credentials. Any default
  repository URL should be a neutral local default
  (e.g. `http://localhost:<port>`), never a hard-coded hosted one.

## Submitting changes

- Write a concise commit message that describes the change and the area
  (e.g. `kernel: fix scheduler race`, `fs: add readdir`).
- Include tests for library changes (`cargo test -p <crate>`).
- Open a pull request; the CI runs formatting, clippy, and the test suite.

## Reporting issues

Include the crate, the failing command or boot phase, and any serial output
if running under QEMU.

## Code of conduct

Be respectful and constructive. Harassment or discrimination will not be
tolerated.
