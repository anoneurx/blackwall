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
    // Scope the NIC lock so it is released BEFORE handle_frame: the stack's
    // transmit path re-acrupts VIRTIO_NET_DEVICE (self-deadlock otherwise).
    let packet = {
        if let Some(ref mut nic) = *crate::drivers::virtio::net::VIRTIO_NET_DEVICE.lock() {
            nic.receive()
        } else {
            None
        }
    };
    if let Some(packet) = packet {
        if let Some(ref mut stack) = *NETWORK_STACK.lock() {
            stack.handle_frame(&packet);
        }
    }
}

// ── SYS_NET control surface ──────────────────────────────────────────────────

/// Handle a `SYS_NET` syscall.  `op` (formerly `r0`) selects the operation;
/// the return value is a bitmask of `NET_ST_*` flags plus the ARP entry count.
pub fn handle_syscall(op: u64) -> u64 {
    use blackwall_shared::syscall::*;

    let nic_present = crate::drivers::virtio::net::VIRTIO_NET_DEVICE.lock().is_some();

    let mut guard = NETWORK_STACK.lock();
    let Some(stack) = guard.as_mut() else {
        return 0;
    };

    let mut status = if nic_present { NET_ST_NIC } else { 0 };

    match op {
        NET_OP_STATE => {
            serial::line(&alloc::format!(
                "[NET] state: ip={}.{}.{}.{} gw={}.{}.{}.{} arp_entries={}",
                stack.ip.0[0], stack.ip.0[1], stack.ip.0[2], stack.ip.0[3],
                stack.gateway.0[0], stack.gateway.0[1], stack.gateway.0[2], stack.gateway.0[3],
                stack.arp_entries(),
            ));
        }
        NET_OP_ARP => {
            let gw = stack.gateway;
            let len = stack.send_arp_request(gw);
            status |= NET_ST_TX_OK;
            serial::line(&alloc::format!(
                "[NET] ARP who-has {}.{}.{}.{} — TX {} bytes",
                gw.0[0], gw.0[1], gw.0[2], gw.0[3], len,
            ));
        }
        NET_OP_PING => {
            let gw = stack.gateway;
            // A fresh sequence each time keeps the reply match unambiguous.
            let seq = (stack.arp_entries() as u16).wrapping_add(1);
            match stack.send_echo_request(gw, 0xB1A4, seq) {
                Some(len) => {
                    status |= NET_ST_TX_OK;
                    serial::line(&alloc::format!(
                        "[NET] ICMP echo -> {}.{}.{}.{} — TX {} bytes (id=0x{:x} seq={})",
                        gw.0[0], gw.0[1], gw.0[2], gw.0[3], len, 0xB1A4, seq,
                    ));
                }
                None => {
                    serial::line("[NET] ICMP echo deferred: gateway MAC unresolved (run 'net arp')");
                }
            }
        }
        _ => return ENOSYS as u64,
    }

    if stack.arp_lookup(stack.gateway).is_some() {
        status |= NET_ST_ARP_GATEWAY;
    }
    if stack.echo_reply_received() {
        status |= NET_ST_ECHO_REPLY;
    }
    status |= ((stack.arp_entries() as u64) & 0xff) << NET_ST_ARP_ENTRIES_SHIFT;
    status
}

