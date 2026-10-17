use crate::parse;
use arc_swap::ArcSwap;
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub struct Record {
    pub ts: i64,
    pub src: String,
    pub dst: String,
    pub name: String,
    pub qtype: u16,
}

/// Case-insensitive suffix match against the (already-normalized) exclude
/// list: "example.com" also matches "a.example.com".
pub fn excluded(name: &str, list: &[String]) -> bool {
    let n = name.trim_end_matches('.').to_ascii_lowercase();
    list.iter()
        .any(|e| n == *e || n.ends_with(&format!(".{e}")))
}

/// Blocking capture loop. Runs until the pcap handle errors out or the
/// writer side of the channel is gone.
pub fn run(
    interface: &str,
    exclude: Arc<ArcSwap<Vec<String>>>,
    tx: Sender<Record>,
) -> Result<(), String> {
    let cap = pcap::Capture::from_device(interface)
        .map_err(|e| format!("pcap device {interface}: {e}"))?
        .snaplen(512)
        .immediate_mode(true)
        .timeout(1000);
    let mut cap = cap.open().map_err(|e| format!("pcap open {interface}: {e}"))?;

    // Queries head toward a resolver, so `dst port 53` approximates outbound
    // even where the driver can't tag direction.
    if let Err(e) = cap.direction(pcap::Direction::Out) {
        eprintln!("warn: Direction::Out unsupported on {interface} ({e}); relying on BPF only");
    }
    cap.filter("udp dst port 53 or tcp dst port 53", true)
        .map_err(|e| format!("BPF filter: {e}"))?;

    eprintln!("capturing on {interface}");
    loop {
        match cap.next_packet() {
            Ok(pkt) => {
                let Some(q) = parse::extract(pkt.data) else {
                    continue;
                };
                // Drop at capture: excluded names never reach the DB.
                if excluded(&q.name, &exclude.load()) {
                    continue;
                }
                let rec = Record {
                    // tv_sec is i32 on some targets; cast keeps this portable
                    #[allow(clippy::unnecessary_cast)]
                    ts: pkt.header.ts.tv_sec as i64,
                    src: q.src,
                    dst: q.dst,
                    name: q.name,
                    qtype: q.qtype,
                };
                if tx.send(rec).is_err() {
                    return Ok(()); // writer gone, shutting down
                }
            }
            Err(pcap::Error::TimeoutExpired) => continue,
            Err(e) => return Err(format!("capture: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::excluded;

    #[test]
    fn suffix_semantics() {
        let list = vec!["example.com".to_string(), "in-addr.arpa".to_string()];
        assert!(excluded("example.com", &list));
        assert!(excluded("a.example.com", &list));
        assert!(excluded("A.EXAMPLE.COM.", &list));
        assert!(excluded("1.2.0.192.in-addr.arpa", &list));
        assert!(!excluded("notexample.com", &list));
        assert!(!excluded("example.com.evil.io", &list));
    }
}
