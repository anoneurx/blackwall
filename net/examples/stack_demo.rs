//! Networking example: construct a unified `NetworkStack` bound to a MAC/IP,
//! open a TCP listener, bind a UDP port, and tighten the firewall.
//!
//! In the kernel the `transmit` callback is the NIC driver's send path; here
//! we use a host placeholder to show the stack's public API.
//!
//! Run with: `cargo run --example stack_demo -p blackwall-net`

use blackwall_net::ethernet::MacAddress;
use blackwall_net::firewall::{Firewall, FirewallAction};
use blackwall_net::ipv4::IpAddress;
use blackwall_net::stack::NetworkStack;

/// Stand-in for the NIC transmit callback (the kernel passes the driver fn).
fn transmit(_raw: &[u8]) {
    // In a real kernel this hands the frame to the NIC driver (e.g. VirtIO).
}

fn main() {
    let mac = MacAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    let ip = IpAddress([10, 0, 0, 2]);
    let gateway = IpAddress([10, 0, 0, 1]);

    let mut stack = NetworkStack::new(mac, ip, gateway, transmit);

    // Open a passive TCP listener (state Listen on port 80).
    stack.tcp_listen(80);

    // Bind a UDP port (DNS stub server), returns false if taken.
    assert!(stack.udp_bind(53), "port 53 should bind");

    // Tighten the default firewall policy to Deny for defense-in-depth.
    *stack.firewall_mut() = Firewall::new(FirewallAction::Deny);

    println!(
        "NetworkStack up: mac={:02x?} ip={:?}.{}.{}.{} tcp[80] udp[53] fw=deny",
        stack.mac.0, stack.ip.0[0], stack.ip.0[1], stack.ip.0[2], stack.ip.0[3],
    );

    // `handle_frame` would be called per received Ethernet frame.
    // stack.handle_frame(&raw_frame_from_nic);
}
