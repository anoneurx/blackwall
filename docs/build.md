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

```sh
cargo run -p blackwall-runner -- run
```

This builds everything, assembles the ISO, and opens a **graphical QEMU
window**.  The interactive shell (`blackwall>`) lives on the serial
console: switch to it from the QEMU window menu (**View → serial0**, or
`Ctrl+Alt+2`).  `exit` / `poweroff` in the shell power the VM down.

For scripted or CI use:

```sh
cargo run -p blackwall-runner -- run-headless   # serial on stdio
cargo run -p blackwall-runner -- run --kvm      # opt-in acceleration
```

KVM is opt-in because the kernel's serial output is unreliable when the
machine runs under KVM.

Notes on firmware:

* The ISO is **UEFI-only**.  `blackwall-runner` attaches OVMF (a
  `pflash` code+vars pair when available, otherwise the combined
  `OVMF.fd` via `-bios`).
* The OVMF variable store is kept across runs in
  `target/blackwall-ovmf-vars.fd` (like a real VM's NVRAM).  The **first**
  boot with a fresh store may stop at the `Please select boot device`
  firmware menu — pick the DVD entry (or just re-run once; the choice
  persists).
* Legacy BIOS (SeaBIOS) VMs will report "no bootable device"; select the
  UEFI/OVMF firmware option in virt-manager (Overview → Firmware) or
  VirtualBox (System → Enable EFI).

Direct invocation (equivalent to what the runner does with a `pflash`
pair):

```sh
cp /usr/share/OVMF/OVMF_VARS_4M.fd target/vars.fd   # writable variable store
qemu-system-x86_64 -enable-kvm -machine q35 -m 512M \
  -display none -serial stdio -nic none -sandbox on \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file=target/vars.fd \
  -cdrom dist/blackwall-server-v0.1.iso
```

Do not use `-bios` with the 4 MiB OVMF images; they must be attached as
pflash units (code + writable vars copy).  The combined `OVMF.fd` image
(`/usr/share/ovmf/OVMF.fd`) works fine with `-bios`.

Expected serial output: the boot phases (memory, VFS, drivers, network) and a
line announcing the userspace init hand-off, followed by the `blackwall>`
shell prompt.
