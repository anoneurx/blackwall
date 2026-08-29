# Examples

Small programs demonstrating how to build on Black Wall Core. Each is
host-runnable (they exercise the portable `blackwall-fs` / `blackwall-net`
libraries).

| Example | What it shows | Run with |
|---------|---------------|----------|
| `../fs/examples/vfs_demo.rs` | Create an in-memory RAM filesystem, populate a file, mount it into the VFS, and read it back through path resolution | `cargo run --example vfs_demo -p blackwall-fs` |
| `../net/examples/stack_demo.rs` | Build a `NetworkStack` (MAC/IP/gateway), open a TCP listener, bind a UDP port, tighten the firewall | `cargo run --example stack_demo -p blackwall-net` |
| `bootable/` | Add a new kernel boot phase and boot the OS under QEMU (bare-metal reference) | see `bootable/README.md` |

The kernel's own boot-time usage of these libraries lives in `kernel/src/init.rs`
(mounting the root RAM filesystem, initializing the network stack) — the
examples mirror that initialization outside the kernel so you can iterate
quickly on host before touching bare-metal code.
