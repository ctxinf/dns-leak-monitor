//! L2–L4 walk (Ethernet, optional 802.1Q, IPv4/IPv6, UDP/TCP) plus DNS
//! question extraction. Returns queries only — the QR bit rejects responses.

pub struct Query {
    pub src: String,
    pub dst: String,
    pub name: String,
    pub qtype: u16,
}

/// Walk one captured frame down to a DNS payload headed for port 53.
pub fn extract(pkt: &[u8]) -> Option<Query> {
    if pkt.len() < 14 {
        return None;
    }
    let mut ethertype = u16::from_be_bytes([pkt[12], pkt[13]]);
    let mut off = 14;
    // Single 802.1Q tag. QinQ (0x88A8) not handled — add a branch if the
    // egress path double-tags.
    if ethertype == 0x8100 {
        if pkt.len() < 18 {
            return None;
        }
        ethertype = u16::from_be_bytes([pkt[16], pkt[17]]);
        off = 18;
    }
    match ethertype {
        0x0800 => extract_v4(&pkt[off..]),
        0x86DD => extract_v6(&pkt[off..]),
        _ => None,
    }
}

fn extract_v4(ip: &[u8]) -> Option<Query> {
    if ip.len() < 20 || ip[0] >> 4 != 4 {
        return None;
    }
    let ihl = ((ip[0] & 0x0F) as usize) * 4;
    if ihl < 20 || ip.len() < ihl {
        return None;
    }
    // Fragments other than the first carry no L4 header.
    let frag_offset = u16::from_be_bytes([ip[6], ip[7]]) & 0x1FFF;
    if frag_offset != 0 {
        return None;
    }
    let proto = ip[9];
    let src = format!("{}.{}.{}.{}", ip[12], ip[13], ip[14], ip[15]);
    let dst = format!("{}.{}.{}.{}", ip[16], ip[17], ip[18], ip[19]);
    extract_l4(proto, &ip[ihl..], src, dst)
}

fn extract_v6(ip: &[u8]) -> Option<Query> {
    // Fixed 40-byte header; extension-header chains are not walked (rare for
    // plain DNS queries).
    if ip.len() < 40 || ip[0] >> 4 != 6 {
        return None;
    }
    let next = ip[6];
    let src = v6_addr(&ip[8..24]);
    let dst = v6_addr(&ip[24..40]);
    extract_l4(next, &ip[40..], src, dst)
}

fn v6_addr(b: &[u8]) -> String {
    let mut seg = [0u16; 8];
    for i in 0..8 {
        seg[i] = u16::from_be_bytes([b[2 * i], b[2 * i + 1]]);
    }
    std::net::Ipv6Addr::from(seg).to_string()
}

fn extract_l4(proto: u8, l4: &[u8], src: String, dst: String) -> Option<Query> {
    let dns = match proto {
        17 => {
            // UDP
            if l4.len() < 8 || u16::from_be_bytes([l4[2], l4[3]]) != 53 {
                return None;
            }
            &l4[8..]
        }
        6 => {
            // TCP: variable header, then 2-byte DNS length prefix. Only
            // queries fully contained in the first segment are parsed.
            if l4.len() < 20 || u16::from_be_bytes([l4[2], l4[3]]) != 53 {
                return None;
            }
            let doff = ((l4[12] >> 4) as usize) * 4;
            if doff < 20 || l4.len() < doff + 2 {
                return None;
            }
            &l4[doff + 2..]
        }
        _ => return None,
    };
    let (name, qtype) = parse_query(dns)?;
    Some(Query { src, dst, name, qtype })
}

/// Parse a DNS message; return (qname, qtype) for queries only.
fn parse_query(d: &[u8]) -> Option<(String, u16)> {
    if d.len() < 12 {
        return None;
    }
    // QR bit set -> response -> drop.
    if d[2] & 0x80 != 0 {
        return None;
    }
    let qdcount = u16::from_be_bytes([d[4], d[5]]);
    if qdcount == 0 {
        return None;
    }
    let mut i = 12;
    let mut name = String::new();
    loop {
        let len = *d.get(i)? as usize;
        if len == 0 {
            i += 1;
            break;
        }
        // Compression pointers never appear in a first question; reject.
        if len & 0xC0 != 0 {
            return None;
        }
        let label = d.get(i + 1..i + 1 + len)?;
        if !name.is_empty() {
            name.push('.');
        }
        for &b in label {
            // Keep output printable even for hostile qnames.
            if b.is_ascii_graphic() && b != b'.' {
                name.push(b as char);
            } else {
                name.push_str(&format!("\\{:03}", b));
            }
        }
        i += 1 + len;
        if name.len() > 512 {
            return None;
        }
    }
    if name.is_empty() {
        return None;
    }
    let qtype = u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]);
    Some((name, qtype))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dns_query(name: &str, qtype: u16) -> Vec<u8> {
        let mut d = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            d.push(label.len() as u8);
            d.extend_from_slice(label.as_bytes());
        }
        d.push(0);
        d.extend_from_slice(&qtype.to_be_bytes());
        d.extend_from_slice(&[0, 1]); // qclass IN
        d
    }

    fn v4_udp_frame(dns: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 12];
        p.extend_from_slice(&[0x08, 0x00]); // ethertype v4
        let udp_len = 8 + dns.len();
        let ip_len = 20 + udp_len;
        p.push(0x45);
        p.push(0);
        p.extend_from_slice(&(ip_len as u16).to_be_bytes());
        p.extend_from_slice(&[0, 0, 0, 0, 64, 17, 0, 0]); // ttl, proto UDP
        p.extend_from_slice(&[192, 0, 2, 1]);
        p.extend_from_slice(&[198, 51, 100, 1]);
        p.extend_from_slice(&60320u16.to_be_bytes());
        p.extend_from_slice(&53u16.to_be_bytes());
        p.extend_from_slice(&(udp_len as u16).to_be_bytes());
        p.extend_from_slice(&[0, 0]);
        p.extend_from_slice(dns);
        p
    }

    #[test]
    fn parses_v4_udp_query() {
        let frame = v4_udp_frame(&dns_query("example.com", 1));
        let q = extract(&frame).expect("should parse");
        assert_eq!(q.src, "192.0.2.1");
        assert_eq!(q.dst, "198.51.100.1");
        assert_eq!(q.name, "example.com");
        assert_eq!(q.qtype, 1);
    }

    #[test]
    fn rejects_response_qr_bit() {
        let mut dns = dns_query("example.com", 1);
        dns[2] |= 0x80;
        assert!(extract(&v4_udp_frame(&dns)).is_none());
    }

    #[test]
    fn rejects_non_dns_port() {
        let mut frame = v4_udp_frame(&dns_query("x.com", 1));
        // dst port bytes live at 14 (eth) + 20 (ip) + 2
        frame[36] = 0;
        frame[37] = 80;
        assert!(extract(&frame).is_none());
    }
}
