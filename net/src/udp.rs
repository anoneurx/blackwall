use crate::ipv4::IpAddress;
use alloc::vec::Vec;

pub struct UdpHeader<'a> {
    pub src_port: u16,
    pub dest_port: u16,
    pub length: u16,
    pub checksum: u16,
    pub payload: &'a [u8],
}

impl<'a> UdpHeader<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }
        let src_port = ((data[0] as u16) << 8) | (data[1] as u16);
        let dest_port = ((data[2] as u16) << 8) | (data[3] as u16);
        let length = ((data[4] as u16) << 8) | (data[5] as u16);
        let checksum = ((data[6] as u16) << 8) | (data[7] as u16);

        if data.len() < length as usize {
            return None;
        }

        Some(Self { src_port, dest_port, length, checksum, payload: &data[8..length as usize] })
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.push((self.src_port >> 8) as u8);
        buffer.push((self.src_port & 0xFF) as u8);
        buffer.push((self.dest_port >> 8) as u8);
        buffer.push((self.dest_port & 0xFF) as u8);

        let length = 8 + self.payload.len() as u16;
        buffer.push((length >> 8) as u8);
        buffer.push((length & 0xFF) as u8);

        // Checksum (default to 0 / disabled in IPv4 unless calculated)
        buffer.push(0);
        buffer.push(0);

        buffer.extend_from_slice(self.payload);
    }
}

pub struct UdpSocket {
    pub local_port: u16,
    pub remote_ip: Option<IpAddress>,
    pub remote_port: Option<u16>,
    pub rx_buffer: Vec<Vec<u8>>,
}

impl UdpSocket {
    pub fn new(local_port: u16) -> Self {
        Self { local_port, remote_ip: None, remote_port: None, rx_buffer: Vec::new() }
    }

    pub fn connect(&mut self, remote_ip: IpAddress, remote_port: u16) {
        self.remote_ip = Some(remote_ip);
        self.remote_port = Some(remote_port);
    }
}

pub struct UdpSocketPool {
    pub sockets: Vec<UdpSocket>,
}

impl UdpSocketPool {
    pub fn new() -> Self {
        Self { sockets: Vec::new() }
    }

    pub fn bind(&mut self, port: u16) -> bool {
        if self.sockets.iter().any(|s| s.local_port == port) {
            false
        } else {
            self.sockets.push(UdpSocket::new(port));
            true
        }
    }

    pub fn deliver(
        &mut self,
        dest_port: u16,
        src_ip: IpAddress,
        src_port: u16,
        payload: &[u8],
    ) -> bool {
        if let Some(socket) = self.sockets.iter_mut().find(|s| s.local_port == dest_port) {
            // Check if socket is restricted to specific remote address
            if let Some(rip) = socket.remote_ip {
                if rip != src_ip {
                    return false;
                }
            }
            if let Some(rport) = socket.remote_port {
                if rport != src_port {
                    return false;
                }
            }
            socket.rx_buffer.push(payload.to_vec());
            true
        } else {
            false
        }
    }
}
