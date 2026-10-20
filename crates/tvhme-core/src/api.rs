//! Tvheadend HTTP JSON API client (works over HTTP or HTTPS, behind reverse proxies and webroots).
use crate::address::ServerAddress;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ApiError {
    #[error("could not resolve the host name \"{0}\". Check the spelling (type only the address, for example tv.example.com)")]
    Dns(String),
    #[error("could not connect to {0}: {1}")]
    Connect(String, String),
    #[error("the server's certificate is not trusted ({0}). Enable \"Allow self-signed certificate\" if you trust this server")]
    Tls(String),
    #[error("the server did not answer in time")]
    Timeout,
    #[error("the server answered HTTP {0}")]
    Status(u16),
    #[error("unexpected answer from the server: {0}")]
    BadResponse(String),
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur { s.push_str(": "); s.push_str(&c.to_string()); cur = c.source(); }
    s
}

pub(crate) fn classify(e: reqwest::Error, addr: &ServerAddress) -> ApiError {
    let full = chain(&e);
    let low = full.to_lowercase();
    if e.is_timeout() { return ApiError::Timeout; }
    if low.contains("dns error") || low.contains("failed to lookup") || low.contains("name or service not known")
        || low.contains("temporary failure in name resolution") || low.contains("no address associated") {
        return ApiError::Dns(addr.host.clone());
    }
    if low.contains("certificate") || low.contains("unknownissuer") || low.contains("invalid peer") {
        return ApiError::Tls(full);
    }
    if e.is_connect() { return ApiError::Connect(format!("{}:{}", addr.host, addr.port), full); }
    ApiError::BadResponse(full)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub uuid: String,
    pub number: u32,
    pub name: String,
    pub icon: Option<String>,
    pub now: Option<NowEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NowEvent {
    pub title: String,
    pub start: i64,
    pub stop: i64,
}

/// One tuner input from `status/inputs` (admin only).
#[derive(Clone, Debug, PartialEq)]
pub struct InputStatus {
    pub input: String,
    pub stream: String,
    pub subs: u32,
    pub signal: Option<u32>,
    pub signal_scale: u32,
    pub snr: Option<u32>,
    pub snr_scale: u32,
    pub ber: Option<u32>,
    pub unc: Option<u32>,
}

/// `/api/serverinfo`
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ServerInfo { pub name: String, pub sw_version: String, pub api_version: u32 }

/// One entry of `/api/status/subscriptions` (admin only).
#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionStatus { pub title: String, pub service: String, pub state: String, pub errors: u32 }

/// One entry of `/api/status/connections` (admin only).
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionStatus { pub user: String, pub peer: String, pub kind: String, pub started: i64 }

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawServerInfo { name: String, sw_version: String, api_version: u32 }

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSubscription { title: String, service: String, state: String, errors: u32 }

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConnection { user: String, peer: String, #[serde(rename = "type")] kind: String, started: i64 }

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct Grid<T> { #[serde(default = "Vec::new")] entries: Vec<T> }

#[derive(Deserialize)]
struct RawChannel {
    uuid: String,
    #[serde(default)] name: String,
    #[serde(default)] number: u32,
    #[serde(default)] enabled: Option<bool>,
    #[serde(default)] icon_public_url: Option<String>,
    #[serde(default)] icon: Option<String>,
}

#[derive(Deserialize)]
struct RawEvent {
    #[serde(rename = "channelUuid")] channel_uuid: String,
    #[serde(default)] title: String,
    #[serde(default)] start: i64,
    #[serde(default)] stop: i64,
}

#[derive(Deserialize)]
struct RawInput {
    #[serde(default)] input: String,
    #[serde(default)] stream: String,
    #[serde(default)] subs: u32,
    #[serde(default)] signal: Option<u32>,
    #[serde(default)] signal_scale: u32,
    #[serde(default)] snr: Option<u32>,
    #[serde(default)] snr_scale: u32,
    #[serde(default)] ber: Option<u32>,
    #[serde(default)] unc: Option<u32>,
}

pub struct Api {
    http: reqwest::Client,
    pub addr: ServerAddress,
    user: String,
    pass: String,
}

impl Api {
    /// `insecure_tls` accepts self-signed certificates (common on home servers).
    pub fn new(addr: ServerAddress, user: &str, pass: &str, insecure_tls: bool) -> Result<Api, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("TVHme/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(6))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .no_proxy() // media server on the LAN or behind its own reverse proxy: ignore system proxy variables
            .danger_accept_invalid_certs(insecure_tls)
            .build()
            .map_err(|e| ApiError::BadResponse(e.to_string()))?;
        Ok(Api { http, addr, user: user.into(), pass: pass.into() })
    }

    pub fn stream_url(&self, uuid: &str) -> String { self.addr.stream_url(uuid, &self.user, &self.pass) }

    async fn send(&self, path: &str) -> Result<reqwest::Response, ApiError> {
        let mut req = self.http.get(self.addr.url(path));
        if !self.user.is_empty() { req = req.basic_auth(&self.user, Some(&self.pass)); }
        req.send().await.map_err(|e| classify(e, &self.addr))
    }

    /// Status code only (the body is not read).
    pub async fn status(&self, path: &str) -> Result<u16, ApiError> {
        Ok(self.send(path).await?.status().as_u16())
    }

    async fn json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let r = self.send(path).await?;
        let code = r.status().as_u16();
        if code != 200 { return Err(ApiError::Status(code)); }
        r.json::<T>().await.map_err(|e| ApiError::BadResponse(e.to_string()))
    }

    /// Channels this account may see (the server filters them), sorted by number then name.
    pub async fn channels(&self) -> Result<Vec<Channel>, ApiError> {
        let g: Grid<RawChannel> = self.json("/api/channel/grid?start=0&limit=10000").await?;
        let mut v: Vec<Channel> = g.entries.into_iter()
            .filter(|c| c.enabled.unwrap_or(true))
            .map(|c| Channel { uuid: c.uuid, number: c.number, name: c.name, icon: c.icon_public_url.or(c.icon).filter(|s| !s.is_empty()), now: None })
            .collect();
        v.sort_by(|a, b| (a.number == 0, a.number, &a.name).cmp(&(b.number == 0, b.number, &b.name)));
        Ok(v)
    }

    /// Currently airing event per channel uuid.
    pub async fn now_events(&self) -> Result<HashMap<String, NowEvent>, ApiError> {
        let g: Grid<RawEvent> = self.json("/api/epg/events/grid?mode=now&start=0&limit=10000").await?;
        Ok(g.entries.into_iter().map(|e| (e.channel_uuid, NowEvent { title: e.title, start: e.start, stop: e.stop })).collect())
    }

    /// Tuner statistics. Admin only on the server.
    pub async fn inputs(&self) -> Result<Vec<InputStatus>, ApiError> {
        let g: Grid<RawInput> = self.json("/api/status/inputs").await?;
        Ok(g.entries.into_iter().map(|i| InputStatus {
            input: i.input, stream: i.stream, subs: i.subs, signal: i.signal, signal_scale: i.signal_scale,
            snr: i.snr, snr_scale: i.snr_scale, ber: i.ber, unc: i.unc,
        }).collect())
    }

    pub async fn server_info(&self) -> Result<ServerInfo, ApiError> {
        let r: RawServerInfo = self.json("/api/serverinfo").await?;
        Ok(ServerInfo { name: r.name, sw_version: r.sw_version, api_version: r.api_version })
    }

    /// Streams currently being served. Admin only on the server.
    pub async fn subscriptions(&self) -> Result<Vec<SubscriptionStatus>, ApiError> {
        let g: Grid<RawSubscription> = self.json("/api/status/subscriptions").await?;
        Ok(g.entries.into_iter().map(|s| SubscriptionStatus { title: s.title, service: s.service, state: s.state, errors: s.errors }).collect())
    }

    /// Clients connected to the server. Admin only on the server.
    pub async fn connections(&self) -> Result<Vec<ConnectionStatus>, ApiError> {
        let g: Grid<RawConnection> = self.json("/api/status/connections").await?;
        Ok(g.entries.into_iter().map(|c| ConnectionStatus { user: c.user, peer: c.peer, kind: c.kind, started: c.started }).collect())
    }
}
