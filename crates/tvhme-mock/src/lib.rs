//! Mock Tvheadend. HTTP side follows the documented JSON API:
//!   /api/config/capabilities (any valid account), /api/config/load (admin only -> 403 otherwise),
//!   /api/channel/grid, /api/epg/events/grid?mode=now, /api/status/inputs (admin only),
//!   channeltag/grid, epg/events/grid (mode=now, channel=, title=, fulltext=), dvr/config/grid,
//!   dvr/entry/{grid_upcoming,grid_finished,grid_failed,create_by_event,create,stop,cancel,remove} (recorder permission),
//!   /stream/channel/<uuid>. 401 = bad credentials, 404 = unknown function.
//! HTSP side: hello / authenticate (sha1 digest) / enableAsyncMetadata / subscribe / signalStatus.
//! Users: admin/admin (admin), user/user (normal). Optional webroot prefix, e.g. "/tvh".
use sha1::{Digest, Sha1};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tvhme_core::htsmsg::{self, Map, MapExt, Value};

pub const CHALLENGE: [u8; 32] = [7u8; 32];
pub const USERS: [(&str, &str, bool); 3] = [("admin", "admin", true), ("user", "user", false), ("viewer", "viewer", false)];
/// "viewer" may watch but has no recorder permission (DVR calls answer 403), like a restricted Tvheadend account.
const NO_RECORDER: &str = "viewer";
pub const CHANNELS: [(&str, u32, &str); 3] = [("uuid-one", 1, "Mock One"), ("uuid-two", 2, "Mock Two"), ("uuid-three", 3, "Mock Three")];

#[derive(Clone)]
struct Rec { uuid: String, event_id: u32, channel: String, title: String, start: i64, stop: i64, state: &'static str }

#[derive(Default)]
struct State { recs: Vec<Rec>, next: u32 }

type Shared = Arc<Mutex<State>>;

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
    let state: Shared = Default::default();
    tokio::spawn(async move { loop { if let Ok((s, _)) = w.accept().await { tokio::spawn(http_conn(s, root.clone(), state.clone())); } } });
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

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => { o.push(b' '); i += 1; }
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) { o.push(v); i += 3; } else { o.push(b'%'); i += 1; }
            }
            c => { o.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&o).to_string()
}

fn parse_params(s: &str) -> Vec<(String, String)> {
    s.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (pct_decode(k), pct_decode(v))
    }).collect()
}

fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64 }

/// (id, channel uuid, title, summary, start offset, stop offset) relative to now.
const EVENTS: [(u32, &str, &str, &str, i64, i64); 4] = [
    (1, "uuid-two", "Evening News", "Headlines of the day", -1800, 1800),
    (2, "uuid-two", "Late Movie", "A film about tuners", 1800, 9000),
    (3, "uuid-one", "Morning Show", "Coffee and chat", 3600, 7200),
    (4, "uuid-three", "Nature Documentary", "Birds of the delta", 600, 4200),
];

fn channel_name(uuid: &str) -> &'static str { CHANNELS.iter().find(|c| c.0 == uuid).map(|c| c.2).unwrap_or("") }

async fn http_conn(mut stream: TcpStream, webroot: String, state: Shared) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break p + 4; }
        match stream.read(&mut chunk).await { Ok(0) | Err(_) => return, Ok(k) => buf.extend_from_slice(&chunk[..k]) }
        if buf.len() > 65536 { return; }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let clen: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
    while buf.len() < head_end + clen {
        match stream.read(&mut chunk).await { Ok(0) | Err(_) => break, Ok(k) => buf.extend_from_slice(&chunk[..k]) }
    }
    let body = String::from_utf8_lossy(&buf[head_end..(head_end + clen).min(buf.len())]).to_string();
    let req = head;
    let full_path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let creds = req.lines().find_map(|l| l.strip_prefix("authorization: Basic ").or_else(|| l.strip_prefix("Authorization: Basic ")))
        .map(|b| String::from_utf8_lossy(&b64_decode(b.trim())).to_string());
    let user: Option<(String, bool)> = creds.and_then(|c| {
        let (u, p) = c.split_once(':')?;
        USERS.iter().find(|(uu, pp, _)| *uu == u && *pp == p).map(|x| (x.0.to_string(), x.2))
    });
    let path_q = match full_path.strip_prefix(webroot.as_str()) { Some(p) if p.starts_with('/') => p.to_string(), _ => String::new() };
    let path = path_q.split('?').next().unwrap_or("").to_string();
    let mut params = parse_params(path_q.split_once('?').map(|x| x.1).unwrap_or(""));
    params.extend(parse_params(&body));
    let param = |k: &str| params.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());

    let json_ok = |v: String| ("200 OK", "application/json", v.into_bytes());
    let forbidden = || ("403 Forbidden", "text/plain", b"forbidden".to_vec());
    let ticket_ok = path.starts_with("/stream/channelid/") && path_q.contains("ticket=TICKET123");
    let (code, ctype, out): (&str, &str, Vec<u8>) = if path.is_empty() {
        ("404 Not Found", "text/plain", b"not found".to_vec())
    } else if ticket_ok {
        ("200 OK", "video/mp2t", b"MOCK-TS-DATA".to_vec())
    } else if user.is_none() {
        ("401 Unauthorized", "text/plain", b"unauthorized".to_vec())
    } else {
        let (uname, is_admin) = user.clone().unwrap();
        let can_record = is_admin || uname != NO_RECORDER;
        let ev_json = |st: &State, e: &(u32, &str, &str, &str, i64, i64)| {
            let n = now();
            let rec = st.recs.iter().find(|r| r.event_id == e.0);
            let dvr = rec.map(|r| format!(",\"dvrUuid\":\"{}\",\"dvrState\":\"{}\"", r.uuid, r.state)).unwrap_or_default();
            format!("{{\"eventId\":{},\"channelUuid\":\"{}\",\"channelName\":\"{}\",\"start\":{},\"stop\":{},\"title\":\"{}\",\"summary\":\"{}\"{}}}",
                e.0, e.1, channel_name(e.1), n + e.4, n + e.5, e.2, e.3, dvr)
        };
        let rec_json = |r: &Rec| format!("{{\"uuid\":\"{}\",\"channelname\":\"{}\",\"disp_title\":\"{}\",\"disp_subtitle\":\"\",\"disp_description\":\"\",\"start\":{},\"stop\":{},\"status\":\"{}\",\"sched_status\":\"{}\",\"filesize\":0,\"errors\":0,\"enabled\":true}}",
            r.uuid, channel_name(&r.channel), r.title, r.start, r.stop, r.state, r.state);
        match path.as_str() {
            "/api/config/capabilities" => json_ok("[\"linuxdvb\"]".into()),
            "/api/config/load" => if is_admin { json_ok("{\"entries\":[]}".into()) } else { forbidden() },
            "/api/channel/grid" => {
                let tags = |u: &str| match u { "uuid-one" => "[\"tag-news\"]", "uuid-two" => "[\"tag-news\",\"tag-movies\"]", _ => "[\"tag-movies\"]" };
                let e: Vec<String> = CHANNELS.iter().map(|(u, num, name)| format!("{{\"uuid\":\"{u}\",\"enabled\":true,\"name\":\"{name}\",\"number\":{num},\"tags\":{}}}", tags(u))).collect();
                json_ok(format!("{{\"entries\":[{}],\"total\":{}}}", e.join(","), CHANNELS.len()))
            }
            "/api/channeltag/grid" => json_ok("{\"entries\":[{\"uuid\":\"tag-movies\",\"name\":\"Movies\",\"enabled\":true,\"index\":2},{\"uuid\":\"tag-news\",\"name\":\"News\",\"enabled\":true,\"index\":1}],\"total\":2}".into()),
            "/api/epg/events/grid" => {
                let st = state.lock().unwrap();
                let mode_now = param("mode").as_deref() == Some("now");
                let chan = param("channel");
                let q = param("title").map(|s| s.to_lowercase());
                let full = param("fulltext").as_deref() == Some("1");
                let sel: Vec<String> = EVENTS.iter()
                    .filter(|e| !mode_now || (e.4 <= 0 && e.5 > 0))
                    .filter(|e| chan.as_deref().map(|c| c == e.1).unwrap_or(true))
                    .filter(|e| q.as_ref().map(|q| e.2.to_lowercase().contains(q) || (full && e.3.to_lowercase().contains(q))).unwrap_or(true))
                    .map(|e| ev_json(&st, e)).collect();
                json_ok(format!("{{\"entries\":[{}],\"totalCount\":{}}}", sel.join(","), sel.len()))
            }
            "/api/dvr/config/grid" if can_record => json_ok("{\"entries\":[{\"uuid\":\"cfg-default\",\"name\":\"\"}],\"total\":1}".into()),
            "/api/dvr/entry/grid_upcoming" | "/api/dvr/entry/grid_finished" | "/api/dvr/entry/grid_failed" if can_record => {
                let want = match path.as_str() { "/api/dvr/entry/grid_upcoming" => "scheduled", "/api/dvr/entry/grid_finished" => "completed", _ => "failed" };
                let st = state.lock().unwrap();
                let e: Vec<String> = st.recs.iter().filter(|r| r.state == want).map(rec_json).collect();
                json_ok(format!("{{\"entries\":[{}],\"total\":{}}}", e.join(","), e.len()))
            }
            "/api/dvr/entry/create_by_event" if can_record => {
                let id: u32 = param("event_id").and_then(|v| v.parse().ok()).unwrap_or(0);
                match (param("config_uuid").as_deref(), EVENTS.iter().find(|e| e.0 == id)) {
                    (Some("cfg-default"), Some(e)) => {
                        let mut st = state.lock().unwrap();
                        st.next += 1;
                        let n = now();
                        let uuid = format!("rec-{}", st.next);
                        st.recs.push(Rec { uuid, event_id: id, channel: e.1.to_string(), title: e.2.to_string(), start: n + e.4, stop: n + e.5, state: "scheduled" });
                        json_ok("{}".into())
                    }
                    _ => ("400 Bad Request", "text/plain", b"bad parameters".to_vec()),
                }
            }
            "/api/dvr/entry/create" if can_record => {
                let conf: serde_json::Value = serde_json::from_str(&param("conf").unwrap_or_default()).unwrap_or(serde_json::Value::Null);
                let (Some(ch), Some(start), Some(stop)) = (conf["channel"].as_str(), conf["start"].as_i64(), conf["stop"].as_i64()) else {
                    return respond(&mut stream, "400 Bad Request", "text/plain", b"bad conf".to_vec()).await;
                };
                let mut st = state.lock().unwrap();
                st.next += 1;
                let uuid = format!("rec-{}", st.next);
                let title = conf["disp_title"].as_str().unwrap_or("Manual").to_string();
                st.recs.push(Rec { uuid: uuid.clone(), event_id: 0, channel: ch.to_string(), title, start, stop, state: "scheduled" });
                json_ok(format!("{{\"uuid\":\"{uuid}\"}}"))
            }
            "/api/dvr/entry/stop" | "/api/dvr/entry/cancel" | "/api/dvr/entry/remove" if can_record => {
                let u = param("uuid").unwrap_or_default();
                let mut st = state.lock().unwrap();
                let before = st.recs.len();
                match path.as_str() {
                    "/api/dvr/entry/stop" => { for r in st.recs.iter_mut().filter(|r| r.uuid == u) { r.state = "completed"; } }
                    _ => st.recs.retain(|r| r.uuid != u),
                }
                if before == st.recs.len() && path != "/api/dvr/entry/stop" { ("404 Not Found", "text/plain", b"no such entry".to_vec()) } else { json_ok("{}".into()) }
            }
            p if p.starts_with("/api/dvr/") => forbidden(),
            "/api/serverinfo" => json_ok("{\"sw_version\":\"4.3-mock\",\"api_version\":19,\"name\":\"tvheadend\",\"capabilities\":[\"linuxdvb\"]}".into()),
            "/api/status/subscriptions" => if is_admin {
                json_ok("{\"entries\":[{\"id\":1,\"title\":\"HTSP\",\"service\":\"Mock Two\",\"state\":\"Running\",\"errors\":0}],\"totalCount\":1}".into())
            } else { forbidden() },
            "/api/status/connections" => if is_admin {
                json_ok("{\"entries\":[{\"id\":1,\"user\":\"user\",\"peer\":\"10.0.0.7\",\"type\":\"HTSP\",\"started\":1700000000}],\"totalCount\":1}".into())
            } else { forbidden() },
            "/api/status/inputs" => if is_admin {
                json_ok("{\"entries\":[{\"uuid\":\"in1\",\"input\":\"Mock DVB-S #0\",\"stream\":\"10773H\",\"subs\":1,\"signal\":49151,\"signal_scale\":1,\"snr\":18500,\"snr_scale\":2,\"ber\":0,\"unc\":0}],\"totalCount\":1}".into())
            } else { forbidden() },
            p if p.starts_with("/dvrfile/") => if can_record { ("200 OK", "video/mp2t", b"MOCK-DVR-DATA".to_vec()) } else { forbidden() },
            p if p.starts_with("/stream/channel/") => ("200 OK", "video/mp2t", b"MOCK-TS-DATA".to_vec()),
            _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
        }
    };
    respond(&mut stream, code, ctype, out).await;
}

async fn respond(stream: &mut TcpStream, code: &str, ctype: &str, body: Vec<u8>) {
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;
}
