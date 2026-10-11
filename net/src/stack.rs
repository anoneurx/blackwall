extern crate alloc;
use alloc::vec::Vec;

use crate::arp::{build_arp_request, ArpOperation, ArpPacket, ArpTable};
use crate::ethernet::{ethertype, EthernetFrame, MacAddress};
use crate::firewall::{Firewall, FirewallAction};
use crate::icmp::{build_echo_reply, build_echo_request, IcmpMessage, IcmpType};
use crate::ipv4::{IpAddress, Ipv4Packet};
use crate::tcp::{process_tcp_state, SocketState, TcpSegment, TcpSocket};
use crate::udp::{UdpHeader, UdpSocketPool};

/// A callback type the kernel provides to transmit a raw frame via the NIC driver.
/// The stack calls this whenever it needs to send a packet.
pub type TransmitFn = fn(&[u8]);

/// Unified network stack — owns all protocol state for one NIC.
pub struct NetworkStack {
    pub mac: MacAddress,
    pub ip: IpAddress,
    pub gateway: IpAddress,

    arp_table: ArpTable,
    tcp_sockets: Vec<TcpSocket>,
    udp_pool: UdpSocketPool,
    firewall: Firewall,

    /// Identifier+sequence of the most recent outbound ping still awaiting a
    /// reply.  `None` = no outstanding echo request.
    pending_ping: Option<(u16, u16)>,

    /// Set when an inbound Echo Reply matches the outstanding ping.
    echo_reply_received: bool,

    transmit: TransmitFn,
}

impl NetworkStack {
    /// Construct a new stack bound to the given MAC and IP addresses.
    /// `transmit` is the NIC driver send callback.
    pub fn new(mac: MacAddress, ip: IpAddress, gateway: IpAddress, transmit: TransmitFn) -> Self {
        // Default policy: allow all (caller can tighten via `firewall_mut()`)
        let firewall = Firewall::new(FirewallAction::Allow);
        Self {
            mac,
            ip,
            gateway,
            arp_table: ArpTable::new(),
            tcp_sockets: Vec::new(),
            udp_pool: UdpSocketPool::new(),
            firewall,
            pending_ping: None,
            echo_reply_received: false,
            transmit,
        }
    }

    /// Expose the firewall for rule configuration.
    pub fn firewall_mut(&mut self) -> &mut Firewall {
        &mut self.firewall
    }

    /// Open a passive TCP listener on `port`.
    pub fn tcp_listen(&mut self, port: u16) {
        let mut sock = TcpSocket::new(self.ip, port);
        sock.state = SocketState::Listen;
        self.tcp_sockets.push(sock);
    }

    /// Bind a UDP port and return `true` on success.
    pub fn udp_bind(&mut self, port: u16) -> bool {
        self.udp_pool.bind(port)
    }

    // ── Outbound (active) API ────────────────────────────────────────────────

    /// Resolve `ip` from the ARP cache, if already learned.
    pub fn arp_lookup(&self, ip: IpAddress) -> Option<MacAddress> {
        self.arp_table.lookup(ip)
    }

    /// Broadcast an ARP *who-has* request for `target_ip`.
    pub fn send_arp_request(&self, target_ip: IpAddress) -> usize {
        let frame = build_arp_request(&self.mac.0, self.ip, target_ip);
        let len = frame.len();
        (self.transmit)(&frame);
        len
    }

    /// Send an ICMP Echo Request to `target_ip`.
    ///
    /// The destination MAC is taken from the ARP cache (the caller is expected
    /// to have issued an ARP request first).  Returns `Some(frame_len)` once
    /// the frame is handed to the NIC, or `None` if the neighbour is unknown.
    pub fn send_echo_request(
        &mut self,
        target_ip: IpAddress,
        identifier: u16,
        sequence: u16,
    ) -> Option<usize> {
        let dest_mac = self.arp_table.lookup(target_ip)?;
        let frame =
            build_echo_request(&self.mac.0, &dest_mac.0, self.ip, target_ip, identifier, sequence);
        self.pending_ping = Some((identifier, sequence));
        self.echo_reply_received = false;
        let len = frame.len();
        (self.transmit)(&frame);
        Some(len)
    }

    /// Send an ICMP Echo Request straight to a MAC (bypassing the ARP cache).
    /// Used when the destination is not yet resolvable.
    pub fn send_echo_request_to_mac(
        &mut self,
        dest_mac: MacAddress,
        target_ip: IpAddress,
        identifier: u16,
        sequence: u16,
    ) -> usize {
        let frame =
            build_echo_request(&self.mac.0, &dest_mac.0, self.ip, target_ip, identifier, sequence);
        self.pending_ping = Some((identifier, sequence));
        self.echo_reply_received = false;
        let len = frame.len();
        (self.transmit)(&frame);
        len
    }

    /// True once an Echo Reply matching the outstanding ping has arrived.
    pub fn echo_reply_received(&self) -> bool {
        self.echo_reply_received
    }

    /// Number of neighbours currently in the ARP cache.
    pub fn arp_entries(&self) -> usize {
        self.arp_table.len()
    }

    /// Process a single raw Ethernet frame received from the NIC.
    pub fn handle_frame(&mut self, raw: &[u8]) {
        let eth = match EthernetFrame::parse(raw) {
            Some(f) => f,
            None => return,
        };
        match eth.ethertype {
            ethertype::ARP => self.handle_arp(&eth),
            ethertype::IPV4 => self.handle_ipv4(&eth),
            ethertype::IPV6 => { /* IPv6 – future extension */ }
            _ => {}
        }
    }

    // ── ARP ──────────────────────────────────────────────────────────────────

    fn handle_arp(&mut self, eth: &EthernetFrame) {
        let pkt = match ArpPacket::parse(eth.payload) {
            Some(p) => p,
            None => return,
        };
        // Learn sender into ARP cache
        self.arp_table.insert(pkt.sender_ip, pkt.sender_mac);

        if pkt.operation == ArpOperation::Request && pkt.target_ip == self.ip {
            self.send_arp_reply(&pkt);
        }
    }

    fn send_arp_reply(&self, req: &ArpPacket) {
        let reply = ArpPacket {
            hw_type: req.hw_type,
            proto_type: req.proto_type,
            hw_size: req.hw_size,
            proto_size: req.proto_size,
            operation: ArpOperation::Reply,
            sender_mac: self.mac,
            sender_ip: self.ip,
            target_mac: req.sender_mac,
            target_ip: req.sender_ip,
        };
        let mut payload: Vec<u8> = Vec::with_capacity(28);
        reply.serialize(&mut payload);

        let frame = EthernetFrame {
            dest: req.sender_mac,
            src: self.mac,
            ethertype: ethertype::ARP,
            payload: &payload,
        };
        let mut buf: Vec<u8> = Vec::with_capacity(42);
        frame.serialize(&mut buf);
        (self.transmit)(&buf);
    }

    // ── IPv4 ─────────────────────────────────────────────────────────────────

    fn handle_ipv4(&mut self, eth: &EthernetFrame) {
        let ip = match Ipv4Packet::parse(eth.payload) {
            Some(p) => p,
            None => return,
        };
        if ip.dest_ip != self.ip {
            return; // not for us
        }

        // Firewall check (ports = 0 for ICMP)
        let (sp, dp) = if ip.protocol == 6 || ip.protocol == 17 {
            if ip.payload.len() >= 4 {
                let sp = ((ip.payload[0] as u16) << 8) | ip.payload[1] as u16;
                let dp = ((ip.payload[2] as u16) << 8) | ip.payload[3] as u16;
                (sp, dp)
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        if self.firewall.check_packet(ip.protocol, ip.src_ip, ip.dest_ip, sp, dp)
            == FirewallAction::Deny
        {
            return;
        }

        match ip.protocol {
            1 => self.handle_icmp(&ip, eth), // ICMP
            6 => self.handle_tcp(&ip),       // TCP
            17 => self.handle_udp(&ip),      // UDP
            _ => {}
        }
    }

    fn handle_icmp(&mut self, ip: &Ipv4Packet, eth: &EthernetFrame) {
        let msg = match IcmpMessage::parse(ip.payload) {
            Some(m) => m,
            None => return,
        };
        // Reject corrupt messages: the checksum must validate to zero over
        // the whole ICMP message (header + payload).
        if crate::ipv4::compute_checksum(ip.payload) != 0 {
            return;
        }
        match msg.icmp_type {
            IcmpType::EchoRequest => {
                let frame = build_echo_reply(&msg, &self.mac.0, &eth.src.0, self.ip, ip.src_ip);
                (self.transmit)(&frame);
            }
            IcmpType::EchoReply => {
                if matches!(self.pending_ping, Some((id, _)) if id == msg.identifier) {
                    self.echo_reply_received = true;
                }
            }
            _ => {}
        }
    }

    fn handle_tcp(&mut self, ip: &Ipv4Packet) {
        let seg = match TcpSegment::parse(ip.payload) {
            Some(s) => s,
            None => return,
        };
        // Validate the TCP checksum over the pseudo-header + segment.
        if crate::tcp::compute_tcp_checksum(ip.src_ip.0, ip.dest_ip.0, ip.payload) != 0 {
            return;
        }
        for sock in &mut self.tcp_sockets {
            if sock.local_port == seg.dest_port {
                process_tcp_state(sock, &seg);
                break;
            }
        }
    }

    fn handle_udp(&mut self, ip: &Ipv4Packet) {
        let hdr = match UdpHeader::parse(ip.payload) {
            Some(h) => h,
            None => return,
        };
        // Validate the UDP checksum (IPv4 pseudo-header + datagram).
        let datagram = &ip.payload[..hdr.length as usize];
        if !UdpHeader::checksum_valid(ip.src_ip.0, self.ip.0, datagram) {
            return;
        }
        self.udp_pool.deliver(hdr.dest_port, ip.src_ip, hdr.src_port, hdr.payload);
    }
}
