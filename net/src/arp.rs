use crate::ethernet::MacAddress;
use crate::ipv4::IpAddress;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpOperation {
    Request = 1,
    Reply = 2,
    Unknown,
}

pub struct ArpPacket {
    pub hw_type: u16,
    pub proto_type: u16,
    pub hw_size: u8,
    pub proto_size: u8,
    pub operation: ArpOperation,
    pub sender_mac: MacAddress,
    pub sender_ip: IpAddress,
    pub target_mac: MacAddress,
    pub target_ip: IpAddress,
}

impl ArpPacket {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 28 {
            return None;
        }
        let hw_type = ((data[0] as u16) << 8) | (data[1] as u16);
        let proto_type = ((data[2] as u16) << 8) | (data[3] as u16);
        let hw_size = data[4];
        let proto_size = data[5];

        let op_code = ((data[6] as u16) << 8) | (data[7] as u16);
        let operation = match op_code {
            1 => ArpOperation::Request,
            2 => ArpOperation::Reply,
            _ => ArpOperation::Unknown,
        };

        let mut sender_mac = [0u8; 6];
        sender_mac.copy_from_slice(&data[8..14]);

        let mut sender_ip = [0u8; 4];
        sender_ip.copy_from_slice(&data[14..18]);

        let mut target_mac = [0u8; 6];
        target_mac.copy_from_slice(&data[18..24]);

        let mut target_ip = [0u8; 4];
        target_ip.copy_from_slice(&data[24..28]);

        Some(Self {
            hw_type,
            proto_type,
            hw_size,
            proto_size,
            operation,
            sender_mac: MacAddress(sender_mac),
            sender_ip: IpAddress(sender_ip),
            target_mac: MacAddress(target_mac),
            target_ip: IpAddress(target_ip),
        })
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.push((self.hw_type >> 8) as u8);
        buffer.push((self.hw_type & 0xFF) as u8);
        buffer.push((self.proto_type >> 8) as u8);
        buffer.push((self.proto_type & 0xFF) as u8);
        buffer.push(self.hw_size);
        buffer.push(self.proto_size);

        let op_code = self.operation as u16;
        buffer.push((op_code >> 8) as u8);
        buffer.push((op_code & 0xFF) as u8);

        buffer.extend_from_slice(&self.sender_mac.0);
        buffer.extend_from_slice(&self.sender_ip.0);
        buffer.extend_from_slice(&self.target_mac.0);
        buffer.extend_from_slice(&self.target_ip.0);
    }
}

pub struct ArpEntry {
    pub ip: IpAddress,
    pub mac: MacAddress,
}

pub struct ArpTable {
    entries: Vec<ArpEntry>,
}

impl ArpTable {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn insert(&mut self, ip: IpAddress, mac: MacAddress) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.ip == ip) {
            entry.mac = mac;
        } else {
            self.entries.push(ArpEntry { ip, mac });
        }
    }

    pub fn lookup(&self, ip: IpAddress) -> Option<MacAddress> {
        self.entries.iter().find(|e| e.ip == ip).map(|e| e.mac)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mac(n: u8) -> MacAddress {
        MacAddress([n, n, n, n, n, n])
    }
    fn test_ip(a: u8, b: u8, c: u8, d: u8) -> IpAddress {
        IpAddress([a, b, c, d])
    }

    #[test]
    fn arp_table_insert_and_lookup() {
        let mut table = ArpTable::new();
        let ip = test_ip(192, 168, 1, 1);
        let mac = test_mac(0xAA);
        table.insert(ip, mac);
        assert_eq!(table.lookup(ip), Some(mac));
    }

    #[test]
    fn arp_table_lookup_miss() {
        let table = ArpTable::new();
        assert_eq!(table.lookup(test_ip(10, 0, 0, 1)), None);
    }

    #[test]
    fn arp_table_update_existing() {
        let mut table = ArpTable::new();
        let ip = test_ip(10, 0, 0, 1);
        table.insert(ip, test_mac(0x01));
        table.insert(ip, test_mac(0x02));
        assert_eq!(table.lookup(ip), Some(test_mac(0x02)));
    }

    #[test]
    fn arp_table_multiple_entries() {
        let mut table = ArpTable::new();
        let ip1 = test_ip(10, 0, 0, 1);
        let ip2 = test_ip(10, 0, 0, 2);
        table.insert(ip1, test_mac(0x01));
        table.insert(ip2, test_mac(0x02));
        assert_eq!(table.lookup(ip1), Some(test_mac(0x01)));
        assert_eq!(table.lookup(ip2), Some(test_mac(0x02)));
    }

    #[test]
    fn arp_packet_parse_too_short() {
        assert!(ArpPacket::parse(&[0u8; 27]).is_none());
    }

    #[test]
    fn arp_packet_serialize_roundtrip() {
        let pkt = ArpPacket {
            hw_type: 1,
            proto_type: 0x0800,
            hw_size: 6,
            proto_size: 4,
            operation: ArpOperation::Request,
            sender_mac: test_mac(0x01),
            sender_ip: test_ip(192, 168, 1, 1),
            target_mac: MacAddress::ZERO,
            target_ip: test_ip(192, 168, 1, 254),
        };
        let mut buf = Vec::new();
        pkt.serialize(&mut buf);
        assert_eq!(buf.len(), 28);
        let parsed = ArpPacket::parse(&buf).unwrap();
        assert_eq!(parsed.hw_type, 1);
        assert_eq!(parsed.proto_type, 0x0800);
        assert_eq!(parsed.operation, ArpOperation::Request);
        assert_eq!(parsed.sender_mac, test_mac(0x01));
        assert_eq!(parsed.sender_ip, test_ip(192, 168, 1, 1));
        assert_eq!(parsed.target_ip, test_ip(192, 168, 1, 254));
    }

    #[test]
    fn arp_operation_from_op_code() {
        let mut data = [0u8; 28];
        data[6] = 0;
        data[7] = 1; // Request
        let pkt = ArpPacket::parse(&data).unwrap();
        assert_eq!(pkt.operation, ArpOperation::Request);

        data[7] = 2; // Reply
        let pkt = ArpPacket::parse(&data).unwrap();
        assert_eq!(pkt.operation, ArpOperation::Reply);

        data[7] = 99; // Unknown
        let pkt = ArpPacket::parse(&data).unwrap();
        assert_eq!(pkt.operation, ArpOperation::Unknown);
    }
}
