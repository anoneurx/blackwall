# Bootable example: adding a kernel boot phase

The kernel boots through ordered phases in `kernel/src/init.rs` (`start()`).
Here's the minimal set of steps to add your own initialization phase and boot
the OS under QEMU.

## 1. Add a phase in `kernel/src/init.rs`

Look for the phase markers in `start()`, e.g.:

```rust
// ── Initialize Network Stack (Phase 9) ───────────────────────────────────
crate::net::init();
```

Register your subsystem after the existing init calls, before the userspace
hand-off:

```rust
// ── My subsystem (Phase X) ────────────────────────────────────────────────
serial::line("[DEBUG] My subsystem initializing...");
```

Run it before the `spawn_user_process(...)` call at the end of `start()`.

## 2. Build for bare metal

The kernel and bootloader target `x86_64-unknown-uefi`; the userland `init`
(embedded into the kernel) targets `x86_64-unknown-none`:

```sh
cargo build -p blackwall-kernel --target x86_64-unknown-uefi
cargo build -p blackwall-bootloader --target x86_64-unknown-uefi
cd userspace/init && cargo build --target x86_64-unknown-none
```

## 3. Assemble an ISO

```sh
cargo run -p bw-iso-builder -- --out /tmp/blackwall.iso
# or: build/scripts/build-iso.sh
```

## 4. Boot under QEMU

With `qemu-system-x86_64` installed:

```sh
cargo run -p blackwall-runner
```

Watch the serial console for your `[DEBUG] My subsystem initializing...`
message (and any panic from your code, which prints through
`kernel/src/panic.rs`).

## Notes

- Phases run in a single core in Ring 0 before the hand-off to userland.
  Anything that must run later belongs in a userland service under
  `services/` instead.
- Validate standalone logic against the host libraries first using the
  `vfs_demo` and `stack_demo` examples before wiring it into the boot path.
