use crate::capture::Record;
use rusqlite::Connection;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

pub fn open(db_path: &str) -> Result<Connection, String> {
    if let Some(dir) = std::path::Path::new(db_path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    let conn = Connection::open(db_path).map_err(|e| format!("open {db_path}: {e}"))?;
    // auto_vacuum must be set before the first table is created.
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA auto_vacuum  = INCREMENTAL;
         PRAGMA synchronous  = NORMAL;
         CREATE TABLE IF NOT EXISTS q(
           ts    INTEGER NOT NULL,
           src   TEXT    NOT NULL,
           dst   TEXT    NOT NULL,
           name  TEXT    NOT NULL,
           qtype INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS q_ts ON q(ts);",
    )
    .map_err(|e| format!("schema: {e}"))?;
    Ok(conn)
}

/// Batched writer: drain up to ~500 records per transaction, flush at least
/// once a second so the console stays near-live.
pub fn writer(conn: Connection, rx: Receiver<Record>) {
    let mut batch: Vec<Record> = Vec::with_capacity(500);
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(rec) => {
                batch.push(rec);
                while batch.len() < 500 {
                    match rx.try_recv() {
                        Ok(r) => batch.push(r),
                        Err(_) => break,
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                flush(&conn, &mut batch);
                return;
            }
        }
        flush(&conn, &mut batch);
    }
}

fn flush(conn: &Connection, batch: &mut Vec<Record>) {
    if batch.is_empty() {
        return;
    }
    let result = (|| -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("INSERT INTO q(ts,src,dst,name,qtype) VALUES(?,?,?,?,?)")?;
            for r in batch.iter() {
                stmt.execute(rusqlite::params![r.ts, r.src, r.dst, r.name, r.qtype])?;
            }
        }
        tx.commit()
    })();
    if let Err(e) = result {
        eprintln!("writer: dropped batch of {}: {e}", batch.len());
    }
    batch.clear();
}

/// Once a day: delete rows past retention, then reclaim pages without the
/// long global lock a full VACUUM would take.
pub fn pruner(db_path: String, retention_days: u32) {
    loop {
        match open(&db_path) {
            Ok(conn) => {
                let cutoff = now_secs() - (retention_days as i64) * 86_400;
                match conn.execute("DELETE FROM q WHERE ts < ?", [cutoff]) {
                    Ok(n) if n > 0 => eprintln!("pruner: removed {n} rows older than {retention_days}d"),
                    Ok(_) => {}
                    Err(e) => eprintln!("pruner: delete failed: {e}"),
                }
                if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                    eprintln!("pruner: vacuum failed: {e}");
                }
            }
            Err(e) => eprintln!("pruner: {e}"),
        }
        std::thread::sleep(Duration::from_secs(86_400));
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
