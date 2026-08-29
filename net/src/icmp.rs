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
    let mut icmp_buf: Vec<u8> = Vec::with_capacity(8 + request.payload.len());
    let reply = IcmpMessage {
        icmp_type: IcmpType::EchoReply,
        code: 0,
        checksum: 0,
        identifier: request.identifier,
        sequence: request.sequence,
        payload: request.payload,
    };
    reply.serialize(&mut icmp_buf);

    // Wrap in IPv4
    let ip_payload = icmp_buf;
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
    let mut frame_buf: Vec<u8> = Vec::with_capacity(14 + 20 + ip_payload.len());
    // Ethernet header
    frame_buf.extend_from_slice(dest_mac);
    frame_buf.extend_from_slice(src_mac);
    frame_buf.extend_from_slice(&[0x08, 0x00]); // IPv4
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
}
