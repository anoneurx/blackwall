use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv6Address(pub [u8; 16]);

impl Ipv6Address {
    pub const UNSPECIFIED: Self = Ipv6Address([0; 16]);
    pub const LOOPBACK: Self = Ipv6Address([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
}

pub struct Ipv6Packet<'a> {
    pub version: u8,
    pub traffic_class: u8,
    pub flow_label: u32,
    pub payload_len: u16,
    pub next_header: u8,
    pub hop_limit: u8,
    pub src_ip: Ipv6Address,
    pub dest_ip: Ipv6Address,
    pub payload: &'a [u8],
}

impl<'a> Ipv6Packet<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 40 {
            return None;
        }

        let version = (data[0] >> 4) & 0x0F;
        if version != 6 {
            return None;
        }

        let traffic_class = ((data[0] & 0x0F) << 4) | ((data[1] >> 4) & 0x0F);
        let flow_label =
            (((data[1] & 0x0F) as u32) << 16) | ((data[2] as u32) << 8) | (data[3] as u32);

        let payload_len = ((data[4] as u16) << 8) | (data[5] as u16);
        let next_header = data[6];
        let hop_limit = data[7];

        let mut src_ip = [0u8; 16];
        src_ip.copy_from_slice(&data[8..24]);

        let mut dest_ip = [0u8; 16];
        dest_ip.copy_from_slice(&data[24..40]);

        if data.len() < (40 + payload_len as usize) {
            return None;
        }

        Some(Self {
            version,
            traffic_class,
            flow_label,
            payload_len,
            next_header,
            hop_limit,
            src_ip: Ipv6Address(src_ip),
            dest_ip: Ipv6Address(dest_ip),
            payload: &data[40..40 + payload_len as usize],
        })
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        // Version (6), Traffic Class (8 bits), Flow Label (20 bits)
        let v_tc_fl =
            (6u32 << 28) | ((self.traffic_class as u32) << 20) | (self.flow_label & 0xFFFFF);
        buffer.push((v_tc_fl >> 24) as u8);
        buffer.push((v_tc_fl >> 16) as u8);
        buffer.push((v_tc_fl >> 8) as u8);
        buffer.push((v_tc_fl & 0xFF) as u8);

        let payload_len = self.payload.len() as u16;
        buffer.push((payload_len >> 8) as u8);
        buffer.push((payload_len & 0xFF) as u8);

        buffer.push(self.next_header);
        buffer.push(self.hop_limit);

        buffer.extend_from_slice(&self.src_ip.0);
        buffer.extend_from_slice(&self.dest_ip.0);

        buffer.extend_from_slice(self.payload);
    }
}
