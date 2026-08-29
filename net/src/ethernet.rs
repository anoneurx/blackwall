extern crate alloc;
use alloc::vec::Vec;

/// 6-byte hardware MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// All-ones broadcast address (`ff:ff:ff:ff:ff:ff`).
    pub const BROADCAST: Self = MacAddress([0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    /// Zero / unspecified address.
    pub const ZERO: Self = MacAddress([0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
}

/// IEEE 802.3 Ethernet frame (zero-copy view into a raw packet buffer).
pub struct EthernetFrame<'a> {
    pub dest: MacAddress,
    pub src: MacAddress,
    /// EtherType: `0x0800` = IPv4, `0x0806` = ARP, `0x86DD` = IPv6.
    pub ethertype: u16,
    pub payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    /// Parse a raw byte slice into an [`EthernetFrame`].
    /// Returns `None` if the slice is shorter than 14 bytes (minimum header).
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 14 {
            return None;
        }
        let mut dest = [0u8; 6];
        dest.copy_from_slice(&data[0..6]);

        let mut src = [0u8; 6];
        src.copy_from_slice(&data[6..12]);

        let ethertype = ((data[12] as u16) << 8) | data[13] as u16;

        Some(Self { dest: MacAddress(dest), src: MacAddress(src), ethertype, payload: &data[14..] })
    }

    /// Serialize this frame into `buffer`.
    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.dest.0);
        buffer.extend_from_slice(&self.src.0);
        buffer.push((self.ethertype >> 8) as u8);
        buffer.push((self.ethertype & 0xFF) as u8);
        buffer.extend_from_slice(self.payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_frame() {
        let data = [
            0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, // dest
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, // src
            0x08, 0x00, // IPv4
            0xDE, 0xAD, // payload
        ];
        let frame = EthernetFrame::parse(&data).unwrap();
        assert_eq!(frame.dest, MacAddress([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]));
        assert_eq!(frame.src, MacAddress([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]));
        assert_eq!(frame.ethertype, ethertype::IPV4);
        assert_eq!(frame.payload, &[0xDE, 0xAD]);
    }

    #[test]
    fn parse_too_short_returns_none() {
        assert!(EthernetFrame::parse(&[0u8; 13]).is_none());
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(EthernetFrame::parse(&[]).is_none());
    }

    #[test]
    fn serialize_roundtrip() {
        let frame = EthernetFrame {
            dest: MacAddress::BROADCAST,
            src: MacAddress([1, 2, 3, 4, 5, 6]),
            ethertype: ethertype::ARP,
            payload: &[0x08, 0x06],
        };
        let mut buf = Vec::new();
        frame.serialize(&mut buf);
        let parsed = EthernetFrame::parse(&buf).unwrap();
        assert_eq!(parsed.dest, MacAddress::BROADCAST);
        assert_eq!(parsed.src, MacAddress([1, 2, 3, 4, 5, 6]));
        assert_eq!(parsed.ethertype, ethertype::ARP);
        assert_eq!(parsed.payload, &[0x08, 0x06]);
    }

    #[test]
    fn mac_address_constants() {
        assert_eq!(MacAddress::BROADCAST.0, [0xff; 6]);
        assert_eq!(MacAddress::ZERO.0, [0u8; 6]);
    }
}

/// Known EtherType constants.
pub mod ethertype {
    pub const IPV4: u16 = 0x0800;
    pub const ARP: u16 = 0x0806;
    pub const IPV6: u16 = 0x86DD;
}
