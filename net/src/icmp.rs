extern crate alloc;
use crate::ipv4::IpAddress;
use alloc::vec::Vec;

/// ICMP message types (RFC 792 §4–§11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IcmpType {
    EchoReply = 0,
    DestUnreachable = 3,
    EchoRequest = 8,
    TimeExceeded = 11,
    Unknown(u8),
}

impl IcmpType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::EchoReply,
            3 => Self::DestUnreachable,
            8 => Self::EchoRequest,
            11 => Self::TimeExceeded,
            x => Self::Unknown(x),
        }
    }
    pub fn to_u8(self) -> u8 {
        match self {
            Self::EchoReply => 0,
            Self::DestUnreachable => 3,
            Self::EchoRequest => 8,
            Self::TimeExceeded => 11,
            Self::Unknown(x) => x,
        }
    }
}

/// Parsed ICMP message (zero-copy view into the IPv4 payload).
pub struct IcmpMessage<'a> {
    pub icmp_type: IcmpType,
    pub code: u8,
    pub checksum: u16,
    /// Identifier field (meaningful for Echo Request/Reply).
    pub identifier: u16,
    /// Sequence number (meaningful for Echo Request/Reply).
    pub sequence: u16,
    pub payload: &'a [u8],
}

impl<'a> IcmpMessage<'a> {
    /// Parse an ICMP message from a raw IPv4 payload slice.
    /// Returns `None` if the slice is shorter than 8 bytes.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }
        let icmp_type = IcmpType::from_u8(data[0]);
        let code = data[1];
        let checksum = ((data[2] as u16) << 8) | data[3] as u16;
        let identifier = ((data[4] as u16) << 8) | data[5] as u16;
        let sequence = ((data[6] as u16) << 8) | data[7] as u16;

        Some(Self { icmp_type, code, checksum, identifier, sequence, payload: &data[8..] })
    }

    /// Serialize this ICMP message into `buffer` (checksum computed automatically).
    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        let start = buffer.len();
        buffer.push(self.icmp_type.to_u8());
        buffer.push(self.code);
        // checksum placeholder
        buffer.push(0);
        buffer.push(0);
        buffer.push((self.identifier >> 8) as u8);
        buffer.push((self.identifier & 0xFF) as u8);
        buffer.push((self.sequence >> 8) as u8);
        buffer.push((self.sequence & 0xFF) as u8);
        buffer.extend_from_slice(self.payload);

        // Fill checksum in-place
        let checksum = crate::ipv4::compute_checksum(&buffer[start..]);
        buffer[start + 2] = (checksum >> 8) as u8;
        buffer[start + 3] = (checksum & 0xFF) as u8;
    }
}

/// Build an ICMP Echo Reply from an inbound Echo Request.
pub fn build_echo_reply(
    request: &IcmpMessage,
    src_mac: &[u8; 6],
    dest_mac: &[u8; 6],
    src_ip: IpAddress,
    dest_ip: IpAddress,
) -> Vec<u8> {
    build_echo(
        IcmpType::EchoReply,
        src_mac,
        dest_mac,
        src_ip,
        dest_ip,
        request.identifier,
        request.sequence,
        request.payload,
    )
}

/// Build an outbound ICMP Echo Request (the *ping* side).
pub fn build_echo_request(
    src_mac: &[u8; 6],
    dest_mac: &[u8; 6],
    src_ip: IpAddress,
    dest_ip: IpAddress,
    identifier: u16,
    sequence: u16,
) -> Vec<u8> {
    build_echo(
        IcmpType::EchoRequest,
        src_mac,
        dest_mac,
        src_ip,
        dest_ip,
        identifier,
        sequence,
        &[],
    )
}

/// Shared Ethernet+IPv4+ICMP framing for outgoing echo traffic.
fn build_echo(
    icmp_type: IcmpType,
    src_mac: &[u8; 6],
    dest_mac: &[u8; 6],
    src_ip: IpAddress,
    dest_ip: IpAddress,
    identifier: u16,
    sequence: u16,
    payload: &[u8],
) -> Vec<u8> {
    let icmp = IcmpMessage {
        icmp_type,
        code: 0,
        checksum: 0,
        identifier,
        sequence,
        payload,
    };
    let mut icmp_buf: Vec<u8> = Vec::with_capacity(8 + payload.len());
    icmp.serialize(&mut icmp_buf);

    // Wrap in IPv4.
    let ip_payload = icmp_buf;
    let mut frame_buf: Vec<u8> = Vec::with_capacity(14 + 20 + ip_payload.len());
    // Ethernet header.
    frame_buf.extend_from_slice(dest_mac);
    frame_buf.extend_from_slice(src_mac);
    frame_buf.extend_from_slice(&[0x08, 0x00]); // IPv4
    let ip_pkt = crate::ipv4::Ipv4Packet {
        version: 4,
        ihl: 5,
        tos: 0,
        id: 0x1234,
        flags: 0,
        fragment_offset: 0,
        ttl: 64,
        protocol: 1, // ICMP
        src_ip,
        dest_ip,
        payload: &ip_payload,
    };
    ip_pkt.serialize(&mut frame_buf);
    frame_buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icmp_type_from_u8() {
        assert_eq!(IcmpType::from_u8(0), IcmpType::EchoReply);
        assert_eq!(IcmpType::from_u8(3), IcmpType::DestUnreachable);
        assert_eq!(IcmpType::from_u8(8), IcmpType::EchoRequest);
        assert_eq!(IcmpType::from_u8(11), IcmpType::TimeExceeded);
        assert_eq!(IcmpType::from_u8(42), IcmpType::Unknown(42));
    }

    #[test]
    fn icmp_type_to_u8() {
        assert_eq!(IcmpType::EchoReply.to_u8(), 0);
        assert_eq!(IcmpType::DestUnreachable.to_u8(), 3);
        assert_eq!(IcmpType::EchoRequest.to_u8(), 8);
        assert_eq!(IcmpType::TimeExceeded.to_u8(), 11);
        assert_eq!(IcmpType::Unknown(99).to_u8(), 99);
    }

    #[test]
    fn icmp_parse_too_short() {
        assert!(IcmpMessage::parse(&[0u8; 7]).is_none());
        assert!(IcmpMessage::parse(&[]).is_none());
    }

    #[test]
    fn icmp_parse_echo_request() {
        // 8-byte header + 6-byte payload = 14 bytes
        let mut data = [0u8; 14];
        data[0] = 8; // EchoRequest
        data[1] = 0; // code
        data[4] = 0x12;
        data[5] = 0x34; // identifier
        data[6] = 0x00;
        data[7] = 0x01; // sequence
        data[8..14].copy_from_slice(b"hello!");

        let msg = IcmpMessage::parse(&data).unwrap();
        assert_eq!(msg.icmp_type, IcmpType::EchoRequest);
        assert_eq!(msg.code, 0);
        assert_eq!(msg.identifier, 0x1234);
        assert_eq!(msg.sequence, 1);
        assert_eq!(msg.payload, b"hello!");
    }

    #[test]
    fn icmp_serialize_roundtrip() {
        let msg = IcmpMessage {
            icmp_type: IcmpType::EchoReply,
            code: 0,
            checksum: 0,
            identifier: 0xABCD,
            sequence: 42,
            payload: &[1, 2, 3, 4],
        };
        let mut buf = Vec::new();
        msg.serialize(&mut buf);
        assert_eq!(buf.len(), 12); // 8 header + 4 payload
        assert_eq!(buf[0], 0); // EchoReply
        assert_eq!(buf[4], 0xAB);
        buf[4];
        assert_eq!(buf[5], 0xCD);

        let parsed = IcmpMessage::parse(&buf).unwrap();
        assert_eq!(parsed.icmp_type, IcmpType::EchoReply);
        assert_eq!(parsed.identifier, 0xABCD);
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn icmp_checksum_is_nonzero() {
        let msg = IcmpMessage {
            icmp_type: IcmpType::EchoRequest,
            code: 0,
            checksum: 0,
            identifier: 1,
            sequence: 1,
            payload: &[],
        };
        let mut buf = Vec::new();
        msg.serialize(&mut buf);
        let cksum = ((buf[2] as u16) << 8) | buf[3] as u16;
        assert_ne!(cksum, 0);
    }

    #[test]
    fn build_echo_request_frames_icmp() {
        let frame = build_echo_request(
            &[0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            &[0x52, 0x54, 0x00, 0x12, 0x34, 0x02],
            IpAddress([10, 0, 2, 15]),
            IpAddress([10, 0, 2, 2]),
            0x1234,
            1,
        );
        assert_eq!(frame.len(), 14 + 20 + 8);
        // Ethernet: IPv4 ethertype, dest/src MACs.
        assert_eq!(&frame[0..6], &[0x52, 0x54, 0x00, 0x12, 0x34, 0x02]);
        assert_eq!(&frame[6..12], &[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        assert_eq!(frame[12], 0x08);
        assert_eq!(frame[13], 0x00);
        // ICMP starts at offset 14+20; type = 8 (EchoRequest), id/seq back.
        let icmp = &frame[14 + 20..];
        assert_eq!(icmp[0], 8);
        assert_eq!(icmp[4], 0x12);
        assert_eq!(icmp[5], 0x34);
        assert_eq!(icmp[6], 0x00);
        assert_eq!(icmp[7], 0x01);
        // IPv4 header: version/IHL, protocol=1 (ICMP), src/dest IP.
        assert_eq!(frame[14], 0x45);
        assert_eq!(frame[14 + 9], 1);
        assert_eq!(&frame[14 + 12..14 + 16], &[10, 0, 2, 15]);
        assert_eq!(&frame[14 + 16..14 + 20], &[10, 0, 2, 2]);
    }
}
