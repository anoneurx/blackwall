extern crate alloc;
use crate::ipv4::IpAddress;
use alloc::vec::Vec;

pub struct DnsHeader {
    pub transaction_id: u16,
    pub flags: u16,
    pub questions: u16,
    pub answers: u16,
    pub authority_records: u16,
    pub additional_records: u16,
}

impl DnsHeader {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }
        let transaction_id = ((data[0] as u16) << 8) | (data[1] as u16);
        let flags = ((data[2] as u16) << 8) | (data[3] as u16);
        let questions = ((data[4] as u16) << 8) | (data[5] as u16);
        let answers = ((data[6] as u16) << 8) | (data[7] as u16);
        let authority_records = ((data[8] as u16) << 8) | (data[9] as u16);
        let additional_records = ((data[10] as u16) << 8) | (data[11] as u16);

        Some(Self {
            transaction_id,
            flags,
            questions,
            answers,
            authority_records,
            additional_records,
        })
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.push((self.transaction_id >> 8) as u8);
        buffer.push((self.transaction_id & 0xFF) as u8);
        buffer.push((self.flags >> 8) as u8);
        buffer.push((self.flags & 0xFF) as u8);
        buffer.push((self.questions >> 8) as u8);
        buffer.push((self.questions & 0xFF) as u8);
        buffer.push((self.answers >> 8) as u8);
        buffer.push((self.answers & 0xFF) as u8);
        buffer.push((self.authority_records >> 8) as u8);
        buffer.push((self.authority_records & 0xFF) as u8);
        buffer.push((self.additional_records >> 8) as u8);
        buffer.push((self.additional_records & 0xFF) as u8);
    }
}

pub fn build_a_query(transaction_id: u16, domain: &str) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(64);

    // Header (RD = 1 flag set for recursion desired)
    let header = DnsHeader {
        transaction_id,
        flags: 0x0100, // standard query with recursion
        questions: 1,
        answers: 0,
        authority_records: 0,
        additional_records: 0,
    };
    header.serialize(&mut buffer);

    // Question Section: Name
    for label in domain.split('.') {
        if label.is_empty() {
            continue;
        }
        buffer.push(label.len() as u8);
        buffer.extend_from_slice(label.as_bytes());
    }
    buffer.push(0); // terminate name

    // QTYPE: A record (1)
    buffer.push(0);
    buffer.push(1);

    // QCLASS: IN class (1)
    buffer.push(0);
    buffer.push(1);

    buffer
}

pub fn parse_a_response(data: &[u8]) -> Option<IpAddress> {
    let header = DnsHeader::parse(data)?;
    if (header.flags & 0x000F) != 0 {
        return None; // RCODE error
    }

    let mut offset = 12;

    // Skip questions
    for _ in 0..header.questions {
        offset = skip_name(data, offset)?;
        if offset + 4 > data.len() {
            return None;
        }
        offset += 4; // Skip QTYPE (2) & QCLASS (2)
    }

    // Parse answers
    for _ in 0..header.answers {
        offset = skip_name(data, offset)?;
        if offset + 10 > data.len() {
            return None;
        }

        let type_code = ((data[offset] as u16) << 8) | (data[offset + 1] as u16);
        let rd_length = ((data[offset + 8] as u16) << 8) | (data[offset + 9] as u16);
        offset += 10;

        if offset + rd_length as usize > data.len() {
            return None;
        }

        if type_code == 1 && rd_length == 4 {
            // A record (IPv4 address)
            let mut ip = [0u8; 4];
            ip.copy_from_slice(&data[offset..offset + 4]);
            return Some(IpAddress(ip));
        }

        offset += rd_length as usize;
    }

    None
}

fn skip_name(data: &[u8], mut offset: usize) -> Option<usize> {
    loop {
        if offset >= data.len() {
            return None;
        }
        let len = data[offset];
        if len == 0 {
            return Some(offset + 1);
        } else if (len & 0xC0) == 0xC0 {
            // Pointer (compression)
            return Some(offset + 2);
        } else {
            // Label
            offset += 1 + len as usize;
        }
    }
}
