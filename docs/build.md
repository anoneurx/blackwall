# Building an ISO

ISO assembly is handled by `tools/iso-builder` (`bw-iso-builder`). It packs
the kernel and userland into a bootable ISO image.

## Prerequisites

- The kernel and bootloader built for `x86_64-unknown-uefi`
- The userland `init` built for `x86_64-unknown-none` (embedded into the kernel)
- A `grub` toolchain or the ISO build script provided in `build/scripts`

## Typical flow

1. Build the entire core (host + bare-metal):
   ```sh
   cargo build
   cargo build -p blackwall-kernel --target x86_64-unknown-uefi
   cargo build -p blackwall-bootloader --target x86_64-unknown-uefi
   cd userspace/init && cargo build --target x86_64-unknown-none
   ```
2. Assemble the ISO:
   ```sh
   cargo run -p bw-iso-builder -- --out /tmp/blackwall.iso
   ```
   or use the script: `build/scripts/build-iso.sh`

## Running under QEMU

With `qemu-system-x86_64` installed, use the runner:

```sh
cargo run -p blackwall-runner
```

Expected serial output: the boot phases (memory, VFS, drivers, network) and a
line announcing the userspace init hand-off.
