use crate::ipv4::IpAddress;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    LastAck,
    TimeWait,
}

pub struct TcpSocket {
    pub local_ip: IpAddress,
    pub remote_ip: IpAddress,
    pub local_port: u16,
    pub remote_port: u16,
    pub state: SocketState,
    pub seq_num: u32,
    pub ack_num: u32,
    pub rx_buffer: Vec<u8>,
    pub tx_buffer: Vec<u8>,
}

impl TcpSocket {
    pub fn new(local_ip: IpAddress, local_port: u16) -> Self {
        Self {
            local_ip,
            remote_ip: IpAddress::UNSPECIFIED,
            local_port,
            remote_port: 0,
            state: SocketState::Closed,
            seq_num: 1000, // random start
            ack_num: 0,
            rx_buffer: Vec::new(),
            tx_buffer: Vec::new(),
        }
    }
}

pub struct TcpSegment<'a> {
    pub src_port: u16,
    pub dest_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub syn: bool,
    pub ack: bool,
    pub fin: bool,
    pub rst: bool,
    pub psh: bool,
    pub window: u16,
    pub payload: &'a [u8],
}

impl<'a> TcpSegment<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 20 {
            return None;
        }
        let src_port = ((data[0] as u16) << 8) | (data[1] as u16);
        let dest_port = ((data[2] as u16) << 8) | (data[3] as u16);
        let seq_num = ((data[4] as u32) << 24)
            | ((data[5] as u32) << 16)
            | ((data[6] as u32) << 8)
            | (data[7] as u32);
        let ack_num = ((data[8] as u32) << 24)
            | ((data[9] as u32) << 16)
            | ((data[10] as u32) << 8)
            | (data[11] as u32);

        let data_offset = (data[12] >> 4) & 0x0F;
        let flags = data[13];

        let fin = (flags & 0x01) != 0;
        let syn = (flags & 0x02) != 0;
        let rst = (flags & 0x04) != 0;
        let psh = (flags & 0x08) != 0;
        let ack = (flags & 0x10) != 0;

        let window = ((data[14] as u16) << 8) | (data[15] as u16);

        let header_len = (data_offset * 4) as usize;
        if data.len() < header_len {
            return None;
        }

        Some(Self {
            src_port,
            dest_port,
            seq_num,
            ack_num,
            syn,
            ack,
            fin,
            rst,
            psh,
            window,
            payload: &data[header_len..],
        })
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.push((self.src_port >> 8) as u8);
        buffer.push((self.src_port & 0xFF) as u8);
        buffer.push((self.dest_port >> 8) as u8);
        buffer.push((self.dest_port & 0xFF) as u8);

        buffer.push((self.seq_num >> 24) as u8);
        buffer.push((self.seq_num >> 16) as u8);
        buffer.push((self.seq_num >> 8) as u8);
        buffer.push((self.seq_num & 0xFF) as u8);

        buffer.push((self.ack_num >> 24) as u8);
        buffer.push((self.ack_num >> 16) as u8);
        buffer.push((self.ack_num >> 8) as u8);
        buffer.push((self.ack_num & 0xFF) as u8);

        // Data offset (5 words = 20 bytes)
        buffer.push(5 << 4);

        // Flags
        let mut flags = 0u8;
        if self.fin {
            flags |= 0x01;
        }
        if self.syn {
            flags |= 0x02;
        }
        if self.rst {
            flags |= 0x04;
        }
        if self.psh {
            flags |= 0x08;
        }
        if self.ack {
            flags |= 0x10;
        }
        buffer.push(flags);

        buffer.push((self.window >> 8) as u8);
        buffer.push((self.window & 0xFF) as u8);

        // Checksum (zero here; filled by `serialize_checksummed`).
        buffer.push(0);
        buffer.push(0);

        // Urgent Pointer
        buffer.push(0);
        buffer.push(0);

        buffer.extend_from_slice(self.payload);
    }

    /// Serialize the segment and fill in the TCP checksum computed over the
    /// IPv4 pseudo-header (RFC 793 §3.1): source/dest address, zero, protocol
    /// 6, and the TCP length.
    pub fn serialize_checksummed(
        &self,
        buffer: &mut Vec<u8>,
        source: [u8; 4],
        dest: [u8; 4],
    ) {
        let start = buffer.len();
        self.serialize(buffer);
        let checksum = compute_tcp_checksum(source, dest, &buffer[start..]);
        buffer[start + 16] = (checksum >> 8) as u8;
        buffer[start + 17] = (checksum & 0xFF) as u8;
    }
}

/// One's-complement checksum over `tcp_segment`, prefixed by the IPv4
/// pseudo-header formed from `source`, `dest` and the segment length.
pub fn compute_tcp_checksum(source: [u8; 4], dest: [u8; 4], tcp_segment: &[u8]) -> u16 {
    use crate::ipv4::{checksum_accumulate, checksum_finish};
    let mut sum = 0u32;
    sum = checksum_accumulate(sum, &source);
    sum = checksum_accumulate(sum, &dest);
    sum = checksum_accumulate(sum, &[0, 6]);
    sum = checksum_accumulate(sum, &(tcp_segment.len() as u16).to_be_bytes());
    sum = checksum_accumulate(sum, tcp_segment);
    checksum_finish(sum)
}

pub fn process_tcp_state(socket: &mut TcpSocket, seg: &TcpSegment) {
    match socket.state {
        SocketState::Listen => {
            if seg.syn {
                socket.state = SocketState::SynReceived;
                socket.remote_port = seg.src_port;
                socket.ack_num = seg.seq_num + 1;
            }
        }
        SocketState::SynSent => {
            if seg.syn && seg.ack {
                socket.state = SocketState::Established;
                socket.ack_num = seg.seq_num + 1;
                socket.seq_num = seg.ack_num;
            }
        }
        SocketState::SynReceived => {
            if seg.ack {
                socket.state = SocketState::Established;
                socket.seq_num = seg.ack_num;
            }
        }
        SocketState::Established => {
            if seg.fin {
                socket.state = SocketState::CloseWait;
                socket.ack_num = seg.seq_num + 1;
            } else if !seg.payload.is_empty() {
                socket.rx_buffer.extend_from_slice(seg.payload);
                socket.ack_num = seg.seq_num + seg.payload.len() as u32;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syn_segment() -> TcpSegment<'static> {
        TcpSegment {
            src_port: 1234,
            dest_port: 80,
            seq_num: 1,
            ack_num: 0,
            syn: true,
            ack: false,
            fin: false,
            rst: false,
            psh: false,
            window: 65535,
            payload: &[],
        }
    }

    #[test]
    fn checksummed_segment_validates_to_zero() {
        let seg = syn_segment();
        let mut buf = Vec::new();
        seg.serialize_checksummed(&mut buf, [192, 168, 0, 1], [192, 168, 0, 2]);
        // Recomputing over the serialized segment (checksum now present) must
        // fold to zero.
        assert_eq!(compute_tcp_checksum([192, 168, 0, 1], [192, 168, 0, 2], &buf), 0);
    }

    #[test]
    fn checksum_covers_pseudo_header() {
        let seg = syn_segment();
        let mut a = Vec::new();
        let mut b = Vec::new();
        seg.serialize_checksummed(&mut a, [10, 0, 0, 1], [10, 0, 0, 2]);
        seg.serialize_checksummed(&mut b, [10, 0, 0, 3], [10, 0, 0, 2]);
        assert_ne!(a[16..18], b[16..18]);
    }

    #[test]
    fn serialize_parse_roundtrip() {
        let seg = syn_segment();
        let mut buf = Vec::new();
        seg.serialize(&mut buf);
        let parsed = TcpSegment::parse(&buf).expect("parse");
        assert_eq!(parsed.src_port, 1234);
        assert_eq!(parsed.dest_port, 80);
        assert_eq!(parsed.seq_num, 1);
        assert!(parsed.syn);
        assert!(!parsed.ack);
    }
}
