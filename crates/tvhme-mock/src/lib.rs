//! Mock Tvheadend. HTTP side follows the documented JSON API:
//!   /api/config/capabilities (any valid account), /api/config/load (admin only -> 403 otherwise),
//!   /api/channel/grid, /api/epg/events/grid?mode=now, /api/status/inputs (admin only),
//!   /stream/channel/<uuid>. 401 = bad credentials, 404 = unknown function.
//! HTSP side: hello / authenticate (sha1 digest) / enableAsyncMetadata / subscribe / signalStatus.
//! Users: admin/admin (admin), user/user (normal). Optional webroot prefix, e.g. "/tvh".
use sha1::{Digest, Sha1};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tvhme_core::htsmsg::{self, Map, MapExt, Value};

pub const CHALLENGE: [u8; 32] = [7u8; 32];
pub const USERS: [(&str, &str, bool); 2] = [("admin", "admin", true), ("user", "user", false)];
pub const CHANNELS: [(&str, u32, &str); 3] = [("uuid-one", 1, "Mock One"), ("uuid-two", 2, "Mock Two"), ("uuid-three", 3, "Mock Three")];

pub struct MockServer {
    pub htsp: SocketAddr,
    pub http: SocketAddr,
}

pub async fn spawn() -> MockServer { spawn_with("", true).await }

/// `webroot` like "/tvh" (or ""); `with_htsp=false` leaves the HTSP port closed (as behind a reverse proxy).
pub async fn spawn_with(webroot: &str, with_htsp: bool) -> MockServer {
    let h = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let w = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv = MockServer { htsp: h.local_addr().unwrap(), http: w.local_addr().unwrap() };
    if with_htsp {
        tokio::spawn(async move { loop { if let Ok((s, _)) = h.accept().await { tokio::spawn(htsp_conn(s)); } } });
    } else {
        drop(h);
    }
    let root = webroot.to_string();
    tokio::spawn(async move { loop { if let Ok((s, _)) = w.accept().await { tokio::spawn(http_conn(s, root.clone())); } } });
    srv
}

fn s(v: &str) -> Value { Value::Str(v.into()) }
fn n(v: i64) -> Value { Value::S64(v) }

async fn send(w: &mut tokio::net::tcp::OwnedWriteHalf, m: Map) -> std::io::Result<()> {
    w.write_all(&htsmsg::encode_message(&m)).await
}

fn reply(req: &Map) -> Map {
    let mut r = Map::new();
    if let Some(q) = req.get("seq") { r.insert("seq".into(), q.clone()); }
    r
}

async fn htsp_conn(stream: TcpStream) {
    let (mut rd, mut wr) = stream.into_split();
    let mut authed = false;
    let mut sub: Option<i64> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        tokio::select! {
            msg = htsmsg::read_message(&mut rd) => {
                let Ok(req) = msg else { return };
                let mut r = reply(&req);
                match req.str("method").unwrap_or("") {
                    "hello" => {
                        r.insert("htspversion".into(), n(34));
                        r.insert("servername".into(), s("MockTVH"));
                        r.insert("challenge".into(), Value::Bin(CHALLENGE.to_vec()));
                    }
                    "authenticate" => {
                        let user = req.str("username").unwrap_or("");
                        let digest = req.bin("digest").unwrap_or(&[]);
                        let ok = USERS.iter().any(|(u, p, _)| {
                            let mut h = Sha1::new();
                            h.update(p.as_bytes());
                            h.update(CHALLENGE);
                            *u == user && h.finalize().as_slice() == digest
                        });
                        if ok { authed = true } else { r.insert("noaccess".into(), n(1)); }
                    }
                    _ if !authed => { r.insert("error".into(), s("Authentication required")); }
                    "enableAsyncMetadata" => {
                        if send(&mut wr, r).await.is_err() { return; }
                        for (i, (_, num, name)) in CHANNELS.iter().enumerate() {
                            let mut c = Map::new();
                            c.insert("method".into(), s("channelAdd"));
                            c.insert("channelId".into(), n(100 + i as i64));
                            c.insert("channelNumber".into(), n(*num as i64));
                            c.insert("channelName".into(), s(name));
                            if send(&mut wr, c).await.is_err() { return; }
                        }
                        let mut d = Map::new();
                        d.insert("method".into(), s("initialSyncCompleted"));
                        if send(&mut wr, d).await.is_err() { return; }
                        continue;
                    }
                    "getTicket" => {
                        r.insert("path".into(), s(&format!("/stream/channelid/{}", req.i64("channelId").unwrap_or(0))));
                        r.insert("ticket".into(), s("TICKET123"));
                    }
                    "subscribe" => { sub = req.i64("subscriptionId"); r.insert("timeshiftPeriod".into(), n(0)); }
                    "unsubscribe" => { sub = None; }
                    _ => { r.insert("error".into(), s("Unknown method")); }
                }
                if send(&mut wr, r).await.is_err() { return; }
            }
            _ = tick.tick(), if sub.is_some() => {
                let mut m = Map::new();
                m.insert("method".into(), s("signalStatus"));
                m.insert("subscriptionId".into(), n(sub.unwrap()));
                m.insert("feStatus".into(), s("GOOD"));
                m.insert("feSNR".into(), n(49151));    // 75%
                m.insert("feSignal".into(), n(32768)); // ~50%
                if send(&mut wr, m).await.is_err() { return; }
            }
        }
    }
}

fn b64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' => 62, b'/' => 63, _ => continue } as u32;
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8); buf &= (1 << bits) - 1; }
    }
    out
}

async fn http_conn(mut stream: TcpStream, webroot: String) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk).await { Ok(0) | Err(_) => return, Ok(k) => buf.extend_from_slice(&chunk[..k]) }
        if buf.len() > 16384 { return; }
    }
    let req = String::from_utf8_lossy(&buf).to_string();
    let full_path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let creds = req.lines().find_map(|l| l.strip_prefix("authorization: Basic ").or_else(|| l.strip_prefix("Authorization: Basic ")))
        .map(|b| String::from_utf8_lossy(&b64_decode(b.trim())).to_string());
    let admin: Option<bool> = creds.and_then(|c| {
        let (u, p) = c.split_once(':')?;
        USERS.iter().find(|(uu, pp, _)| *uu == u && *pp == p).map(|x| x.2)
    });
    let path_q = match full_path.strip_prefix(webroot.as_str()) { Some(p) if p.starts_with('/') => p.to_string(), _ => String::new() };
    let path = path_q.split('?').next().unwrap_or("").to_string();

    let ticket_ok = path.starts_with("/stream/channelid/") && path_q.contains("ticket=TICKET123");
    let json_ok = |v: String| ("200 OK", "application/json", v.into_bytes());
    let (code, ctype, body): (&str, &str, Vec<u8>) = if path.is_empty() {
        ("404 Not Found", "text/plain", b"not found".to_vec())
    } else if ticket_ok {
        ("200 OK", "video/mp2t", b"MOCK-TS-DATA".to_vec())
    } else if admin.is_none() {
        ("401 Unauthorized", "text/plain", b"unauthorized".to_vec())
    } else {
        let is_admin = admin.unwrap();
        match path.as_str() {
            "/api/config/capabilities" => json_ok("[\"linuxdvb\"]".into()),
            "/api/config/load" => if is_admin { json_ok("{\"entries\":[]}".into()) } else { ("403 Forbidden", "text/plain", b"forbidden".to_vec()) },
            "/api/channel/grid" => {
                let e: Vec<String> = CHANNELS.iter().map(|(u, num, name)| format!("{{\"uuid\":\"{u}\",\"enabled\":true,\"name\":\"{name}\",\"number\":{num}}}")).collect();
                json_ok(format!("{{\"entries\":[{}],\"total\":{}}}", e.join(","), CHANNELS.len()))
            }
            "/api/epg/events/grid" => json_ok("{\"entries\":[{\"eventId\":1,\"channelUuid\":\"uuid-two\",\"title\":\"Evening News\",\"start\":1000,\"stop\":4600}],\"totalCount\":1}".into()),
            "/api/serverinfo" => json_ok("{\"sw_version\":\"4.3-mock\",\"api_version\":19,\"name\":\"tvheadend\",\"capabilities\":[\"linuxdvb\"]}".into()),
            "/api/status/subscriptions" => if is_admin {
                json_ok("{\"entries\":[{\"id\":1,\"title\":\"HTSP\",\"service\":\"Mock Two\",\"state\":\"Running\",\"errors\":0}],\"totalCount\":1}".into())
            } else { ("403 Forbidden", "text/plain", b"forbidden".to_vec()) },
            "/api/status/connections" => if is_admin {
                json_ok("{\"entries\":[{\"id\":1,\"user\":\"user\",\"peer\":\"10.0.0.7\",\"type\":\"HTSP\",\"started\":1700000000}],\"totalCount\":1}".into())
            } else { ("403 Forbidden", "text/plain", b"forbidden".to_vec()) },
            "/api/status/inputs" => if is_admin {
                json_ok("{\"entries\":[{\"uuid\":\"in1\",\"input\":\"Mock DVB-S #0\",\"stream\":\"10773H\",\"subs\":1,\"signal\":49151,\"signal_scale\":1,\"snr\":18500,\"snr_scale\":2,\"ber\":0,\"unc\":0}],\"totalCount\":1}".into())
            } else { ("403 Forbidden", "text/plain", b"forbidden".to_vec()) },
            p if p.starts_with("/stream/channel/") => ("200 OK", "video/mp2t", b"MOCK-TS-DATA".to_vec()),
            _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
        }
    };
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;
}
