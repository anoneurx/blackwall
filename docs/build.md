# Building an ISO

ISO assembly is handled by `tools/iso-builder` (`bw-iso-builder`). It packs
the kernel and userland into a bootable UEFI ISO image.  The builder is pure
Rust (uses the `isobemak` crate), so **no `grub`, `xorriso` or `mkisofs` is
required**.

## Prerequisites

- Rust targets `x86_64-unknown-uefi` and `x86_64-unknown-none`
  (`rustup target add x86_64-unknown-uefi x86_64-unknown-none`)
- The userland `init` built for `x86_64-unknown-none` — it is embedded into
  the kernel with `include_bytes!`, so it must exist before the kernel builds

## Typical flow

1. Build the userland init (embedded into the kernel):
   ```sh
   cd userspace/init && cargo build --target x86_64-unknown-none
   ```
2. Build the UEFI artifacts:
   ```sh
   cargo build -p blackwall-bootloader --target x86_64-unknown-uefi
   cargo build -p blackwall-kernel --target x86_64-unknown-uefi
   ```
   **Important:** the bootloader statically embeds the kernel library
   (`blackwall_kernel::init::start`), so after *any* kernel change you must
   rebuild **both** `-p blackwall-bootloader` and `-p blackwall-kernel` before
   re-running the ISO builder — otherwise the ISO boots the stale kernel.
3. Assemble the ISO:
   ```sh
   cargo run -p bw-iso-builder -- --output dist/blackwall-server-v0.1.iso
   ```
   The flag is `--output` (or `-o`); the default output is
   `dist/blackwall-server-v0.1.iso`.  The same flow is scripted in
   `build/scripts/build-iso.sh` and `build/Makefile`.

## Running under QEMU

`cargo run -p blackwall-runner` automates build + ISO + QEMU, but its ISO
step shells out to `xorriso`; where `xorriso` is unavailable use
`bw-iso-builder` plus a direct QEMU invocation with OVMF:

```sh
cp /usr/share/OVMF/OVMF_VARS_4M.fd /tmp/vars.fd   # writable variable store
qemu-system-x86_64 -enable-kvm -machine q35 -m 256M \
  -display none -serial stdio -nic none -sandbox on \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file=/tmp/vars.fd \
  -cdrom dist/blackwall-server-v0.1.iso
```

Do not use `-bios` with the 4 MiB OVMF images; they must be attached as
pflash units (code + writable vars copy).

Expected serial output: the boot phases (memory, VFS, drivers, network) and a
line announcing the userspace init hand-off, followed by the `blackwall>`
shell prompt.
