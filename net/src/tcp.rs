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

        // Checksum placeholder
        buffer.push(0);
        buffer.push(0);

        // Urgent Pointer
        buffer.push(0);
        buffer.push(0);

        buffer.extend_from_slice(self.payload);
    }
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
