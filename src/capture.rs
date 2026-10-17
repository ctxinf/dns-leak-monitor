use crate::parse;
use std::sync::mpsc::Sender;

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
    runtime: &crate::config::Shared,
    tx: &Sender<Record>,
) -> Result<(), String> {
    let cap = pcap::Capture::from_device(interface)
        .map_err(|e| format!("pcap device {interface}: {e}"))?
        .snaplen(512)
        .immediate_mode(true)
        .timeout(1000);
    let mut cap = cap
        .open()
        .map_err(|e| format!("pcap open {interface}: {e}"))?;

    // Queries head toward a resolver, so `dst port 53` approximates outbound
    // even where the driver can't tag direction.
    if let Err(e) = cap.direction(pcap::Direction::Out) {
        eprintln!("warn: Direction::Out unsupported on {interface} ({e}); relying on BPF only");
    }
    cap.filter("udp dst port 53 or tcp dst port 53", true)
        .map_err(|e| format!("BPF filter: {e}"))?;

    let mut cap = cap.setnonblock().map_err(|e| e.to_string())?;
    runtime.lock().unwrap().status = format!("Capturing on {interface}");
    let mut last_check = std::time::Instant::now() - std::time::Duration::from_secs(3);
    loop {
        if runtime.lock().unwrap().settings.interface != interface {
            return Ok(());
        }
        if last_check.elapsed().as_secs() >= 2 {
            let devices = pcap::Device::list().map_err(|e| e.to_string())?;
            if !devices
                .iter()
                .any(|d| d.name == interface && d.flags.is_up() && d.flags.is_running())
            {
                return Err(format!("Interface {interface} is unavailable or down"));
            }
            last_check = std::time::Instant::now();
        }
        match cap.next_packet() {
            Ok(pkt) => {
                let Some(q) = parse::extract(pkt.data) else {
                    continue;
                };
                // Drop at capture: excluded names never reach the DB.
                if excluded(&q.name, &runtime.lock().unwrap().settings.exclude) {
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
            Err(pcap::Error::TimeoutExpired) => {
                std::thread::sleep(std::time::Duration::from_millis(200))
            }
            Err(e) => return Err(format!("capture: {e}")),
        }
    }
}

pub fn supervise(runtime: crate::config::Shared, tx: Sender<Record>) {
    let mut delay = 5u64;
    loop {
        let interface = runtime.lock().unwrap().settings.interface.clone();
        let started = std::time::Instant::now();
        match run(&interface, &runtime, &tx) {
            Ok(()) => {
                delay = 5;
                continue;
            }
            Err(e) => {
                if started.elapsed().as_secs() >= 60 {
                    delay = 5;
                }
                let message = format!("Paused: {e}. Retrying in {delay}s");
                eprintln!("{message}");
                runtime.lock().unwrap().status = message;
            }
        }
        for remaining in (1..=delay).rev() {
            if runtime.lock().unwrap().settings.interface != interface {
                break;
            }
            // Sleeping in short increments keeps UI changes responsive.
            std::thread::sleep(std::time::Duration::from_secs(1));
            if remaining == 1 {
                runtime.lock().unwrap().status = "Retrying capture".into();
            }
        }
        delay = (delay * 2).min(60);
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
