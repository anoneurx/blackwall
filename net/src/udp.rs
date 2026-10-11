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

        if length < 8 || data.len() < length as usize {
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

        // Checksum placeholder; `serialize_checksummed` fills it in.
        buffer.push(0);
        buffer.push(0);

        buffer.extend_from_slice(self.payload);
    }

    /// Serialize and fill in the UDP checksum computed over the IPv4
    /// pseudo-header + the full UDP datagram.
    pub fn serialize_checksummed(&self, source: [u8; 4], dest: [u8; 4], buffer: &mut Vec<u8>) {
        let start = buffer.len();
        self.serialize(buffer);
        let checksum = compute_udp_checksum(source, dest, &buffer[start..]);
        // A computed checksum of zero is transmitted as all-ones (0xFFFF),
        // because zero means "no checksum" in UDP.
        let c = if checksum == 0 { 0xFFFF } else { checksum };
        buffer[start + 6] = (c >> 8) as u8;
        buffer[start + 7] = (c & 0xFF) as u8;
    }

    /// True when the datagram's checksum field is correct (or absent,
    /// which is legal in IPv4).
    pub fn checksum_valid(source: [u8; 4], dest: [u8; 4], datagram: &[u8]) -> bool {
        if datagram.len() < 8 {
            return false;
        }
        let declared = ((datagram[6] as u16) << 8) | (datagram[7] as u16);
        if declared == 0 {
            return true; // IPv4 permits a zero (unused) checksum.
        }
        compute_udp_checksum(source, dest, datagram) == 0
    }
}

/// One's-complement checksum over the IPv4 pseudo-header (src, dst, protocol
/// 17, UDP length) followed by the UDP datagram.
pub fn compute_udp_checksum(source: [u8; 4], dest: [u8; 4], udp_datagram: &[u8]) -> u16 {
    use crate::ipv4::{checksum_accumulate, checksum_finish};
    let mut sum = 0;
    sum = checksum_accumulate(sum, &source);
    sum = checksum_accumulate(sum, &dest);
    sum = checksum_accumulate(sum, &[0, 17]);
    sum = checksum_accumulate(sum, &(udp_datagram.len() as u16).to_be_bytes());
    sum = checksum_accumulate(sum, udp_datagram);
    checksum_finish(sum)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_datagram() -> (Vec<u8>, [u8; 4], [u8; 4]) {
        let hdr = UdpHeader {
            src_port: 1234,
            dest_port: 53,
            length: 0, // filled by serialize
            checksum: 0,
            payload: b"hello",
        };
        let src = [192, 168, 0, 1];
        let dst = [192, 168, 0, 2];
        let mut buf = Vec::new();
        hdr.serialize_checksummed(src, dst, &mut buf);
        (buf, src, dst)
    }

    #[test]
    fn checksummed_datagram_validates() {
        let (buf, src, dst) = sample_datagram();
        // Checksum field must be non-zero and the whole datagram must
        // validate to zero.
        assert_ne!(((buf[6] as u16) << 8) | buf[7] as u16, 0);
        assert!(UdpHeader::checksum_valid(src, dst, &buf));
        assert_eq!(compute_udp_checksum(src, dst, &buf), 0);
    }

    #[test]
    fn corrupt_payload_fails_validation() {
        let (mut buf, src, dst) = sample_datagram();
        buf[8] ^= 0xFF; // flip a payload byte
        assert!(!UdpHeader::checksum_valid(src, dst, &buf));
    }

    #[test]
    fn zero_checksum_accepted_in_ipv4() {
        let (mut buf, src, dst) = sample_datagram();
        buf[6] = 0;
        buf[7] = 0;
        assert!(UdpHeader::checksum_valid(src, dst, &buf));
    }

    #[test]
    fn checksum_covers_pseudo_header() {
        let (buf, src_a, dst) = sample_datagram();
        let src_b = [10, 0, 0, 1];
        assert!(!UdpHeader::checksum_valid(src_b, dst, &buf));
        assert!(!UdpHeader::checksum_valid(src_a, [10, 0, 0, 9], &buf));
    }
}
