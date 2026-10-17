use rusqlite::Connection;
use tiny_http::{Header, Method, Response, Server};

const INDEX: &str = include_str!("../static/index.html");
const FAVICON: &str = include_str!("../static/favicon.svg");
const PAGE_SIZE_DEFAULT: i64 = 50;
const PAGE_SIZE_MAX: i64 = 1000;

pub fn run(bind: &str, db_path: &str, runtime: crate::config::Shared) -> Result<(), String> {
    let server = Server::http(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    // Reads use a read-only connection; deletes get a separate read-write
    // one. WAL serializes it against the writer thread; busy_timeout rides
    // out batch commits instead of failing with SQLITE_BUSY.
    let rconn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("web db open {db_path}: {e}"))?;
    let wconn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("web db open {db_path}: {e}"))?;
    wconn
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("busy_timeout: {e}"))?;
    eprintln!("console on http://{bind}");

    for mut req in server.incoming_requests() {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
        let resp = match (req.method(), path) {
            (Method::Get, "/") | (Method::Get, "/index.html") => Response::from_string(INDEX)
                .with_header(header("Content-Type", "text/html; charset=utf-8")),
            (Method::Get, "/favicon.svg") => Response::from_string(FAVICON)
                .with_header(header("Content-Type", "image/svg+xml"))
                .with_header(header("Cache-Control", "max-age=86400")),
            (Method::Get, "/api/settings") => {
                let state = runtime.lock().unwrap();
                let devices = pcap::Device::list();
                let (interfaces, error) = match devices {
                    Ok(devices) => (
                        devices
                            .iter()
                            .map(|d| json_str(&d.name))
                            .collect::<Vec<_>>()
                            .join(","),
                        String::new(),
                    ),
                    Err(e) => (String::new(), e.to_string()),
                };
                Response::from_string(format!("{{\"interface\":{},\"exclude\":[{}],\"status\":{},\"interfaces\":[{}],\"error\":{}}}",
                    json_str(&state.settings.interface), state.settings.exclude.iter().map(|s| json_str(s)).collect::<Vec<_>>().join(","),
                    json_str(&state.status), interfaces, json_str(&error)))
                    .with_header(header("Content-Type", "application/json"))
                    .with_header(header("Cache-Control", "no-store"))
            }
            (Method::Post, "/api/settings") => {
                let mut body = String::new();
                use std::io::Read;
                let result = req
                    .as_reader()
                    .take(65537)
                    .read_to_string(&mut body)
                    .map_err(|e| e.to_string())
                    .and_then(|_| {
                        if body.len() > 65536 {
                            Err("Settings too large".into())
                        } else {
                            toml::from_str::<crate::config::RuntimeConfig>(&body)
                                .map_err(|e| e.to_string())
                        }
                    })
                    .and_then(|settings| runtime.lock().unwrap().save(settings));
                match result {
                    Ok(()) => Response::from_string("Saved"),
                    Err(e) => Response::from_string(e).with_status_code(400),
                }
            }
            (Method::Get, "/api/records") => match records_json(&rconn, query) {
                Ok(json) => Response::from_string(json)
                    .with_header(header("Content-Type", "application/json")),
                Err(e) => Response::from_string(format!("query failed: {e}")).with_status_code(500),
            },
            (Method::Delete, "/api/records") => match delete_records(&wconn, query) {
                Ok(Some(n)) => Response::from_string(format!("{{\"deleted\":{n}}}"))
                    .with_header(header("Content-Type", "application/json")),
                Ok(None) => Response::from_string("need id=, name=, all=1 or a from=/to= range")
                    .with_status_code(400),
                Err(e) => {
                    Response::from_string(format!("delete failed: {e}")).with_status_code(500)
                }
            },
            (Method::Get, _) => Response::from_string("not found").with_status_code(404),
            _ => Response::from_string("method not allowed").with_status_code(405),
        };
        let _ = req.respond(resp);
    }
    Ok(())
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("static header")
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn num_param(query: &str, key: &str) -> Option<i64> {
    query_param(query, key).and_then(|v| v.parse::<i64>().ok())
}

/// Shared WHERE clause for the record list and range deletes, so the console
/// deletes exactly the rows the current filter shows.
///
/// `from`/`to` are epoch seconds; `q` is a case-insensitive substring match on
/// the qname. Returns the SQL fragment plus its bound parameters.
fn filter_sql(query: &str) -> (String, Vec<rusqlite::types::Value>) {
    use rusqlite::types::Value;
    let mut sql = String::from(" WHERE 1=1");
    let mut params: Vec<Value> = Vec::new();
    if let Some(from) = num_param(query, "from") {
        sql.push_str(" AND ts >= ?");
        params.push(Value::Integer(from));
    }
    if let Some(to) = num_param(query, "to") {
        sql.push_str(" AND ts <= ?");
        params.push(Value::Integer(to));
    }
    let needle = query_param(query, "q")
        .map(percent_decode)
        .unwrap_or_default();
    let needle = needle.trim().to_string();
    if !needle.is_empty() {
        // LIKE with the default (non-ASCII-aware) collation is fine here:
        // qnames are ASCII after the parser's normalization.
        sql.push_str(" AND name LIKE ? ESCAPE '\\'");
        params.push(Value::Text(format!("%{}%", like_escape(&needle))));
    }
    (sql, params)
}

/// Escape LIKE wildcards so a qname containing `%` or `_` is matched literally.
fn like_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// `{"total":N,"pageNo":N,"pageSize":N,"rows":[…]}`.
///
/// `collapse=1` groups consecutive-in-time duplicates of the same qname into a
/// single row carrying `count` and the id list, so a host hammering one name
/// does not bury everything else. Grouping is per (name, src, dst) over the
/// whole filtered set, not just the page, so counts stay stable across pages.
fn records_json(conn: &Connection, query: &str) -> rusqlite::Result<String> {
    let page_size = num_param(query, "pageSize")
        .unwrap_or(PAGE_SIZE_DEFAULT)
        .clamp(1, PAGE_SIZE_MAX);
    let page_no = num_param(query, "pageNo").unwrap_or(1).max(1);
    let offset = (page_no - 1) * page_size;
    let collapse = query_param(query, "collapse") == Some("1");
    let (where_sql, params) = filter_sql(query);
    let pref: Vec<&dyn rusqlite::ToSql> =
        params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

    let total: i64 = if collapse {
        conn.query_row(
            &format!("SELECT count(*) FROM (SELECT 1 FROM q{where_sql} GROUP BY name,src,dst)"),
            pref.as_slice(),
            |r| r.get(0),
        )?
    } else {
        conn.query_row(
            &format!("SELECT count(*) FROM q{where_sql}"),
            pref.as_slice(),
            |r| r.get(0),
        )?
    };

    let sql = if collapse {
        format!(
            "SELECT max(rowid), max(ts), src, dst, name, max(qtype), count(*), min(ts), \
                    group_concat(rowid) \
             FROM q{where_sql} GROUP BY name,src,dst ORDER BY max(ts) DESC LIMIT ? OFFSET ?"
        )
    } else {
        format!(
            // CAST so `ids` is text in both branches, matching group_concat.
            "SELECT rowid, ts, src, dst, name, qtype, 1, ts, CAST(rowid AS TEXT) \
             FROM q{where_sql} ORDER BY ts DESC, rowid DESC LIMIT ? OFFSET ?"
        )
    };

    let mut args = params.clone();
    args.push(rusqlite::types::Value::Integer(page_size));
    args.push(rusqlite::types::Value::Integer(offset));
    let aref: Vec<&dyn rusqlite::ToSql> = args.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(aref.as_slice())?;
    let mut out =
        format!("{{\"total\":{total},\"pageNo\":{page_no},\"pageSize\":{page_size},\"rows\":[");
    let mut first = true;
    while let Some(row) = rows.next()? {
        if !first {
            out.push(',');
        }
        first = false;
        let id: i64 = row.get(0)?;
        let ts: i64 = row.get(1)?;
        let src: String = row.get(2)?;
        let dst: String = row.get(3)?;
        let name: String = row.get(4)?;
        let qtype: u16 = row.get(5)?;
        let count: i64 = row.get(6)?;
        let first_ts: i64 = row.get(7)?;
        let ids: String = row.get(8)?;
        out.push_str(&format!(
            "{{\"id\":{id},\"ts\":{ts},\"src\":{},\"dst\":{},\"name\":{},\"qtype\":{qtype},\
              \"count\":{count},\"firstTs\":{first_ts},\"ids\":{}}}",
            json_str(&src),
            json_str(&dst),
            json_str(&name),
            json_str(&ids)
        ));
    }
    out.push_str("]}");
    Ok(out)
}

/// `id=<rowid>` deletes one row (comma-separated ids allowed, for collapsed
/// groups); `name=<qname>` deletes every row with that exact qname
/// (case-insensitive, DNS names are); `all=1` deletes everything matching the
/// current filter — with no filter params that is the whole table. Returns
/// Ok(None) when no recognized parameter is present.
fn delete_records(conn: &Connection, query: &str) -> rusqlite::Result<Option<usize>> {
    if let Some(raw) = query_param(query, "id") {
        let ids: Vec<i64> = percent_decode(raw)
            .split(',')
            .filter_map(|v| v.trim().parse::<i64>().ok())
            .collect();
        if ids.is_empty() {
            return Ok(None);
        }
        let holes = vec!["?"; ids.len()].join(",");
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let n = conn.execute(
            &format!("DELETE FROM q WHERE rowid IN ({holes})"),
            params.as_slice(),
        )?;
        return Ok(Some(n));
    }
    if let Some(raw) = query_param(query, "name") {
        let name = percent_decode(raw);
        if name.is_empty() {
            return Ok(None);
        }
        let n = conn.execute(
            "DELETE FROM q WHERE name = ? COLLATE NOCASE",
            [name.trim_end_matches('.')],
        )?;
        return Ok(Some(n));
    }
    if query_param(query, "all") == Some("1") {
        let (where_sql, params) = filter_sql(query);
        let pref: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let n = conn.execute(&format!("DELETE FROM q{where_sql}"), pref.as_slice())?;
        return Ok(Some(n));
    }
    Ok(None)
}

fn percent_decode(s: &str) -> String {
    fn hex(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
            } else {
                out.push(b'%');
                i += 1;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Minimal JSON string encoder — qnames come off the wire and may contain
/// anything the parser let through.
fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE q(ts INTEGER, src TEXT, dst TEXT, name TEXT, qtype INTEGER);
             INSERT INTO q VALUES
               (100,'192.0.2.1','198.51.100.1','old.example.com',1),
               (200,'192.0.2.1','198.51.100.2','new.example.com',28),
               (300,'192.0.2.1','198.51.100.2','NEW.example.com',1);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn records_json_orders_and_paginates() {
        let conn = seeded();
        let json = records_json(&conn, "pageSize=1").unwrap();
        assert!(json.starts_with(r#"{"total":3,"pageNo":1,"pageSize":1,"rows":["#));
        assert!(json.contains(r#""name":"NEW.example.com""#));
        assert!(!json.contains("old.example.com"));

        // page 2 skips the newest row
        let json = records_json(&conn, "pageSize=1&pageNo=2").unwrap();
        assert!(json.contains(r#""name":"new.example.com""#));
        assert!(json.contains(r#""ts":200"#));

        // page past the end is empty but still reports the true total
        let json = records_json(&conn, "pageSize=10&pageNo=9").unwrap();
        assert!(json.contains(r#""total":3"#) && json.ends_with(r#""rows":[]}"#));

        // garbage falls back to defaults rather than erroring
        let json = records_json(&conn, "pageSize=zzz&pageNo=-4").unwrap();
        assert!(json.contains("old.example.com") && json.contains(r#""pageNo":1"#));
    }

    #[test]
    fn records_json_time_range_and_search() {
        let conn = seeded();
        let json = records_json(&conn, "from=150&to=250").unwrap();
        assert!(json.contains(r#""total":1"#) && json.contains(r#""ts":200"#));

        let json = records_json(&conn, "q=old").unwrap();
        assert!(json.contains(r#""total":1"#) && json.contains("old.example.com"));

        // substring match is case-insensitive and URL-decoded
        let json = records_json(&conn, "q=NEW.EXAMPLE").unwrap();
        assert!(json.contains(r#""total":2"#));

        // LIKE wildcards in the needle are literal, not patterns
        let json = records_json(&conn, "q=%25").unwrap();
        assert!(json.contains(r#""total":0"#));
    }

    #[test]
    fn records_json_collapse_groups_duplicates() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE q(ts INTEGER, src TEXT, dst TEXT, name TEXT, qtype INTEGER);
             INSERT INTO q VALUES
               (100,'192.0.2.1','198.51.100.1','loud.example.com',1),
               (150,'192.0.2.1','198.51.100.1','loud.example.com',1),
               (200,'192.0.2.1','198.51.100.1','loud.example.com',1),
               (300,'192.0.2.1','198.51.100.1','quiet.example.com',1);",
        )
        .unwrap();
        let json = records_json(&conn, "collapse=1").unwrap();
        // two groups, not four rows; newest group first
        assert!(json.contains(r#""total":2"#));
        assert!(json.contains(r#""name":"quiet.example.com","qtype":1,"count":1"#));
        assert!(json.contains(r#""name":"loud.example.com","qtype":1,"count":3"#));
        // the group carries every id so the console can delete it in one call
        assert!(json.contains(r#""ids":"1,2,3""#));
        assert!(json.contains(r#""firstTs":100"#));
    }

    #[test]
    fn delete_by_id_and_name() {
        let conn = seeded();
        assert_eq!(delete_records(&conn, "id=1").unwrap(), Some(1));
        assert_eq!(delete_records(&conn, "id=999").unwrap(), Some(0));
        // case-insensitive exact name match, trailing dot tolerated
        assert_eq!(
            delete_records(&conn, "name=new.example.com.").unwrap(),
            Some(2)
        );
        assert_eq!(delete_records(&conn, "").unwrap(), None);
        assert_eq!(delete_records(&conn, "id=abc").unwrap(), None);
        let left: i64 = conn
            .query_row("SELECT count(*) FROM q", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn delete_id_list_clears_a_collapsed_group() {
        let conn = seeded();
        assert_eq!(delete_records(&conn, "id=2%2C3").unwrap(), Some(2));
        let left: i64 = conn
            .query_row("SELECT count(*) FROM q", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);
    }

    #[test]
    fn delete_all_respects_the_active_filter() {
        let conn = seeded();
        // range-scoped delete removes only what the filter selects
        assert_eq!(
            delete_records(&conn, "all=1&from=150&to=250").unwrap(),
            Some(1)
        );
        // all=1 with no filter empties the table
        assert_eq!(delete_records(&conn, "all=1").unwrap(), Some(2));
        let left: i64 = conn
            .query_row("SELECT count(*) FROM q", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        // all=0 is not a delete-everything instruction
        assert_eq!(delete_records(&conn, "all=0").unwrap(), None);
    }

    #[test]
    fn query_param_matches_whole_keys_only() {
        // a `pageNo` param must not satisfy a lookup for `page`
        assert_eq!(query_param("pageNo=2", "page"), None);
        assert_eq!(query_param("pageSize=10&pageNo=2", "pageNo"), Some("2"));
    }

    #[test]
    fn percent_decode_basics() {
        assert_eq!(percent_decode("a%2Db+c"), "a-b c");
        assert_eq!(percent_decode("plain.example.com"), "plain.example.com");
        assert_eq!(percent_decode("bad%2"), "bad%2");
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(json_str("x\ny"), "\"x\\u000ay\"");
    }
}
