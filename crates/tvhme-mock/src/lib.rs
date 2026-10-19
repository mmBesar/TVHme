//! Mock Tvheadend: HTSP (hello/authenticate/enableAsyncMetadata/subscribe/signalStatus)
//! and HTTP (admin-only /api/config, authenticated /stream/channelid/N).
//! Users: admin/admin (admin), user/user (normal). Anything else is rejected.
use sha1::{Digest, Sha1};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tvhme_core::htsmsg::{self, Map, MapExt, Value};

pub const CHALLENGE: [u8; 32] = [7u8; 32];
pub const USERS: [(&str, &str, bool); 2] = [("admin", "admin", true), ("user", "user", false)];

pub struct MockServer {
    pub htsp: SocketAddr,
    pub http: SocketAddr,
}

pub async fn spawn() -> MockServer {
    let h = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let w = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let srv = MockServer { htsp: h.local_addr().unwrap(), http: w.local_addr().unwrap() };
    tokio::spawn(async move { loop { if let Ok((s, _)) = h.accept().await { tokio::spawn(htsp_conn(s)); } } });
    tokio::spawn(async move { loop { if let Ok((s, _)) = w.accept().await { tokio::spawn(http_conn(s)); } } });
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
                        for (id, num, name) in [(1, 1, "Mock One"), (2, 2, "Mock Two"), (3, 3, "Mock Three")] {
                            let mut c = Map::new();
                            c.insert("method".into(), s("channelAdd"));
                            c.insert("channelId".into(), n(id));
                            c.insert("channelNumber".into(), n(num));
                            c.insert("channelName".into(), s(name));
                            if send(&mut wr, c).await.is_err() { return; }
                        }
                        let mut d = Map::new();
                        d.insert("method".into(), s("initialSyncCompleted"));
                        if send(&mut wr, d).await.is_err() { return; }
                        continue;
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
                m.insert("feSNR".into(), n(49151));   // 75%
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

async fn http_conn(mut stream: TcpStream) {
    let mut buf = vec![0u8; 8192];
    let Ok(len) = stream.read(&mut buf).await else { return };
    let req = String::from_utf8_lossy(&buf[..len]).to_string();
    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let creds = req.lines().find_map(|l| l.strip_prefix("Authorization: Basic ")).map(|b| String::from_utf8_lossy(&b64_decode(b.trim())).to_string());
    let user = creds.and_then(|c| {
        let (u, p) = c.split_once(':')?;
        USERS.iter().find(|(uu, pp, _)| *uu == u && *pp == p).map(|x| x.2)
    });
    let (code, body): (&str, &[u8]) = match (user, path.as_str()) {
        (None, _) => ("401 Unauthorized", b"unauthorized"),
        (Some(true), "/api/config") => ("200 OK", b"{}"),
        (Some(false), "/api/config") => ("403 Forbidden", b"forbidden"),
        (Some(_), p) if p.starts_with("/stream/channelid/") => ("200 OK", b"MOCK-TS-DATA"),
        _ => ("404 Not Found", b"not found"),
    };
    let head = format!("HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
}
