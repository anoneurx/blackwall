# ARM64 (aarch64) Port Plan

## Goals
Port Black Wall Core to aarch64 (Raspberry Pi 4/5, general embedded) with device tree support.

## Phases
1. Toolchain/setup (aarch64-unknown-none-softfloat, device tree)
2. Boot (U-Boot/UEFI aarch64 or direct kernel)
3. Arch layer (exceptions, MMU, timers, UART)
4. Interrupts/GIC
5. Memory/PCI/virtio
6. Drivers (GPIO, storage, network)
7. SMP
8. Testing on QEMU virt

## Dependencies
- DTB parsing
- GICv2/GICv3
- PL011/8250 UART
