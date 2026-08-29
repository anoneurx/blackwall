//! Kernel networking adapter — thin bridge between the VirtIO NIC driver and
//! the portable `blackwall-net` library crate.
//!
//! # Responsibilities
//! - Defines the `virtio_transmit` callback that the network stack calls to
//!   push frames out to the NIC.
//! - Holds the global `NETWORK_STACK` wrapped in the kernel's `SpinLock`.
//! - `init()` configures the stack with the QEMU default MAC/IP.
//! - `poll()` receives one frame per tick and feeds it to the stack.
//!
//! All protocol logic (ARP, ICMP, TCP, UDP, DNS, Firewall) now lives in
//! `blackwall-net` — see `net/src/` in the workspace root.

extern crate alloc;

use blackwall_net::stack::NetworkStack;

use crate::arch::x86_64::serial;
use crate::sync::spin::SpinLock;

// Re-export the core address types so the rest of the kernel can do
// `use crate::net::{IpAddress, MacAddress}` without knowing about blackwall-net.
pub use blackwall_net::ethernet::MacAddress;
pub use blackwall_net::ipv4::IpAddress;

// ── Global state ──────────────────────────────────────────────────────────────

pub static NETWORK_STACK: SpinLock<Option<NetworkStack>> = SpinLock::new(None);

// ── VirtIO transmit callback ──────────────────────────────────────────────────

/// Called by `NetworkStack` whenever it needs to send a frame.
/// Routes to the VirtIO net device driver.
fn virtio_transmit(data: &[u8]) {
    if let Some(ref mut nic) = *crate::drivers::virtio::net::VIRTIO_NET_DEVICE.lock() {
        nic.transmit(data);
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Initialise the network stack with the QEMU user-mode networking defaults.
/// Call once during kernel boot (Phase 5).
pub fn init() {
    serial::line("[NET] Initializing networking stack (blackwall-net)...");

    let mac = MacAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]); // QEMU default
    let ip = IpAddress([10, 0, 2, 15]); // QEMU user-net
    let gateway = IpAddress([10, 0, 2, 2]);

    *NETWORK_STACK.lock() = Some(NetworkStack::new(mac, ip, gateway, virtio_transmit));

    serial::line(&alloc::format!(
        "[NET] Stack online — IP: {}.{}.{}.{} | MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        ip.0[0],
        ip.0[1],
        ip.0[2],
        ip.0[3],
        mac.0[0],
        mac.0[1],
        mac.0[2],
        mac.0[3],
        mac.0[4],
        mac.0[5]
    ));
}

/// Receive one frame from the VirtIO NIC and process it through the stack.
/// Called once per scheduler tick.
pub fn poll() {
    if let Some(ref mut nic) = *crate::drivers::virtio::net::VIRTIO_NET_DEVICE.lock() {
        if let Some(packet) = nic.receive() {
            if let Some(ref mut stack) = *NETWORK_STACK.lock() {
                stack.handle_frame(&packet);
            }
        }
    }
}
