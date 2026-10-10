use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpAddress(pub [u8; 4]);

impl IpAddress {
    pub const BROADCAST: Self = IpAddress([255, 255, 255, 255]);
    pub const UNSPECIFIED: Self = IpAddress([0, 0, 0, 0]);
    pub const LOOPBACK: Self = IpAddress([127, 0, 0, 1]);
}

pub struct Ipv4Packet<'a> {
    pub version: u8,
    pub ihl: u8,
    pub tos: u8,
    pub id: u16,
    pub flags: u8,
    pub fragment_offset: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub src_ip: IpAddress,
    pub dest_ip: IpAddress,
    pub payload: &'a [u8],
}

impl<'a> Ipv4Packet<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 20 {
            return None;
        }
        let version = (data[0] >> 4) & 0x0F;
        let ihl = data[0] & 0x0F;
        let tos = data[1];
        let total_len = ((data[2] as u16) << 8) | (data[3] as u16);
        let id = ((data[4] as u16) << 8) | (data[5] as u16);

        let flags = data[6] >> 5;
        let fragment_offset = (((data[6] & 0x1F) as u16) << 8) | (data[7] as u16);

        let ttl = data[8];
        let protocol = data[9];

        let mut src_ip = [0u8; 4];
        src_ip.copy_from_slice(&data[12..16]);

        let mut dest_ip = [0u8; 4];
        dest_ip.copy_from_slice(&data[16..20]);

        let header_len = (ihl * 4) as usize;
        if data.len() < header_len || data.len() < total_len as usize {
            return None;
        }

        Some(Self {
            version,
            ihl,
            tos,
            id,
            flags,
            fragment_offset,
            ttl,
            protocol,
            src_ip: IpAddress(src_ip),
            dest_ip: IpAddress(dest_ip),
            payload: &data[header_len..total_len as usize],
        })
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        let header_start = buffer.len();

        // Version (4) & IHL (5 words = 20 bytes)
        let version_ihl = (4 << 4) | 5;
        buffer.push(version_ihl);
        buffer.push(self.tos);

        let total_len = 20 + self.payload.len();
        buffer.push((total_len >> 8) as u8);
        buffer.push((total_len & 0xFF) as u8);

        buffer.push((self.id >> 8) as u8);
        buffer.push((self.id & 0xFF) as u8);

        let flags_frag = ((self.flags & 0x07) << 5) as u16 | (self.fragment_offset & 0x1FFF);
        buffer.push((flags_frag >> 8) as u8);
        buffer.push((flags_frag & 0xFF) as u8);

        buffer.push(self.ttl);
        buffer.push(self.protocol);

        // Checksum placeholder
        buffer.push(0);
        buffer.push(0);

        buffer.extend_from_slice(&self.src_ip.0);
        buffer.extend_from_slice(&self.dest_ip.0);

        // Calculate internet checksum over header
        let checksum = compute_checksum(&buffer[header_start..header_start + 20]);
        buffer[header_start + 10] = (checksum >> 8) as u8;
        buffer[header_start + 11] = (checksum & 0xFF) as u8;

        // Payload
        buffer.extend_from_slice(self.payload);
    }
}

/// Accumulate the one's-complement big-endian 16-bit sum of `data` into a
/// running 32-bit `sum`.  Exposed so callers (e.g. TCP) can seed the sum with
/// a pseudo-header before folding.
pub fn checksum_accumulate(mut sum: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | (data[i + 1] as u32);
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    sum
}

/// Fold a 32-bit one's-complement accumulator into the final 16-bit checksum.
pub fn checksum_finish(mut sum: u32) -> u16 {
    while (sum >> 16) > 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn compute_checksum(data: &[u8]) -> u16 {
    checksum_finish(checksum_accumulate(0, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_address_constants() {
        assert_eq!(IpAddress::BROADCAST.0, [255, 255, 255, 255]);
        assert_eq!(IpAddress::UNSPECIFIED.0, [0, 0, 0, 0]);
        assert_eq!(IpAddress::LOOPBACK.0, [127, 0, 0, 1]);
    }

    #[test]
    fn ip_address_equality() {
        let a = IpAddress([192, 168, 1, 1]);
        let b = IpAddress([192, 168, 1, 1]);
        let c = IpAddress([10, 0, 0, 1]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn compute_checksum_single_byte() {
        // Single byte 0x00 → padded to 0x0000, sum = 0, checksum = 0xFFFF
        assert_eq!(compute_checksum(&[0x00]), 0xFFFF);
    }

    #[test]
    fn compute_checksum_two_bytes() {
        // 0x01 0x02 → word = 0x0102, sum = 0x0102, !0x0102 = 0xFDFD
        assert_eq!(compute_checksum(&[0x01, 0x02]), 0xFEFD);
    }

    #[test]
    fn compute_checksum_known_value() {
        // RFC 793 example: header bytes 0x00 0x01 → sum = 0x0001, checksum = 0xFFFE
        assert_eq!(compute_checksum(&[0x00, 0x01]), 0xFFFE);
    }

    #[test]
    fn parse_minimal_ipv4_packet() {
        // 20-byte header: version=4, ihl=5, total_len=20, ttl=64, protocol=1 (ICMP), src=10.0.0.1, dst=10.0.0.2
        let mut data = [0u8; 20];
        data[0] = 0x45; // version=4, ihl=5
        data[2] = 0;
        data[3] = 20; // total_len = 20
        data[8] = 64; // ttl
        data[9] = 1; // protocol (ICMP)
        data[12..16].copy_from_slice(&[10, 0, 0, 1]); // src
        data[16..20].copy_from_slice(&[10, 0, 0, 2]); // dst

        let pkt = Ipv4Packet::parse(&data).unwrap();
        assert_eq!(pkt.version, 4);
        assert_eq!(pkt.ihl, 5);
        assert_eq!(pkt.ttl, 64);
        assert_eq!(pkt.protocol, 1);
        assert_eq!(pkt.src_ip, IpAddress([10, 0, 0, 1]));
        assert_eq!(pkt.dest_ip, IpAddress([10, 0, 0, 2]));
        assert!(pkt.payload.is_empty());
    }

    #[test]
    fn parse_too_short_returns_none() {
        assert!(Ipv4Packet::parse(&[0u8; 19]).is_none());
    }

    #[test]
    fn serialize_roundtrip() {
        let pkt = Ipv4Packet {
            version: 4,
            ihl: 5,
            tos: 0,
            id: 0x1234,
            flags: 0,
            fragment_offset: 0,
            ttl: 128,
            protocol: 6,
            src_ip: IpAddress([172, 16, 0, 1]),
            dest_ip: IpAddress([172, 16, 0, 2]),
            payload: &[0xDE, 0xAD],
        };
        let mut buf = Vec::new();
        pkt.serialize(&mut buf);
        assert_eq!(buf.len(), 22); // 20 header + 2 payload
        let parsed = Ipv4Packet::parse(&buf).unwrap();
        assert_eq!(parsed.version, 4);
        assert_eq!(parsed.ttl, 128);
        assert_eq!(parsed.protocol, 6);
        assert_eq!(parsed.src_ip, IpAddress([172, 16, 0, 1]));
        assert_eq!(parsed.dest_ip, IpAddress([172, 16, 0, 2]));
        assert_eq!(parsed.payload, &[0xDE, 0xAD]);
    }
}
