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

impl ApiError {
    /// Short text for the user. 403 is the server telling us this account may not do that.
    pub fn user_message(&self) -> String {
        match self {
            ApiError::Status(401) => "Wrong username or password.".into(),
            ApiError::Status(403) => "Your account is not allowed to do this on the server.".into(),
            ApiError::Status(404) => "The server does not know this item (it may be gone already).".into(),
            other => other.to_string(),
        }
    }
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
    /// UUIDs of the channel tags (groups) this channel belongs to.
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tag { pub uuid: String, pub name: String, pub index: i64 }

/// One EPG event from `epg/events/grid`.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: u32,
    pub channel_uuid: String,
    pub channel_name: String,
    pub start: i64,
    pub stop: i64,
    pub title: String,
    pub summary: String,
    /// Set when this event is scheduled/recording/recorded (`dvrUuid`, `dvrState`).
    pub dvr_uuid: Option<String>,
    pub dvr_state: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingKind { Upcoming, Finished, Failed }

/// One DVR entry (`dvr/entry/grid_*`).
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub uuid: String,
    pub channel_name: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub start: i64,
    pub stop: i64,
    pub status: String,
    pub sched_status: String,
    pub filesize: u64,
    pub errors: u32,
    pub enabled: bool,
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
    #[serde(default)] tags: Vec<String>,
}

#[derive(Deserialize)]
struct RawTag {
    uuid: String,
    #[serde(default)] name: String,
    #[serde(default)] index: i64,
    #[serde(default)] enabled: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFullEvent {
    #[serde(rename = "eventId")] event_id: u32,
    #[serde(rename = "channelUuid")] channel_uuid: String,
    #[serde(rename = "channelName")] channel_name: String,
    start: i64,
    stop: i64,
    title: String,
    summary: String,
    description: String,
    #[serde(rename = "dvrUuid")] dvr_uuid: Option<String>,
    #[serde(rename = "dvrState")] dvr_state: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawRecording {
    uuid: String,
    channelname: String,
    disp_title: String,
    disp_subtitle: String,
    disp_description: String,
    start: i64,
    stop: i64,
    status: String,
    sched_status: String,
    filesize: u64,
    errors: u32,
    enabled: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawDvrConfig { uuid: String, name: String }

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

    async fn send_q(&self, method: reqwest::Method, path: &str, query: &[(&str, &str)], form: &[(&str, &str)]) -> Result<reqwest::Response, ApiError> {
        let mut req = self.http.request(method, self.addr.url(path));
        if !query.is_empty() { req = req.query(query); }
        if !form.is_empty() { req = req.form(form); }
        if !self.user.is_empty() { req = req.basic_auth(&self.user, Some(&self.pass)); }
        req.send().await.map_err(|e| classify(e, &self.addr))
    }

    async fn send(&self, path: &str) -> Result<reqwest::Response, ApiError> {
        self.send_q(reqwest::Method::GET, path, &[], &[]).await
    }

    /// Status code only (the body is not read).
    pub async fn status(&self, path: &str) -> Result<u16, ApiError> {
        Ok(self.send(path).await?.status().as_u16())
    }

    async fn json_q<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, ApiError> {
        let r = self.send_q(reqwest::Method::GET, path, query, &[]).await?;
        let code = r.status().as_u16();
        if code != 200 { return Err(ApiError::Status(code)); }
        r.json::<T>().await.map_err(|e| ApiError::BadResponse(e.to_string()))
    }

    async fn json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.json_q(path, &[]).await
    }

    /// State-changing call (POST, form-encoded). Success is HTTP 200; the body is ignored.
    async fn action(&self, path: &str, form: &[(&str, &str)]) -> Result<(), ApiError> {
        let r = self.send_q(reqwest::Method::POST, path, &[], form).await?;
        match r.status().as_u16() { 200 => Ok(()), c => Err(ApiError::Status(c)) }
    }

    /// Channels this account may see (the server filters them), sorted by number then name.
    pub async fn channels(&self) -> Result<Vec<Channel>, ApiError> {
        let g: Grid<RawChannel> = self.json("/api/channel/grid?start=0&limit=10000").await?;
        let mut v: Vec<Channel> = g.entries.into_iter()
            .filter(|c| c.enabled.unwrap_or(true))
            .map(|c| Channel { uuid: c.uuid, number: c.number, name: c.name, icon: c.icon_public_url.or(c.icon).filter(|s| !s.is_empty()), now: None, tags: c.tags })
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

    /// Channel tags (groups) this account may see, ordered by their index then name.
    pub async fn tags(&self) -> Result<Vec<Tag>, ApiError> {
        let g: Grid<RawTag> = self.json("/api/channeltag/grid?start=0&limit=1000").await?;
        let mut v: Vec<Tag> = g.entries.into_iter().filter(|t| t.enabled.unwrap_or(true))
            .map(|t| Tag { uuid: t.uuid, name: t.name, index: t.index }).collect();
        v.sort_by(|a, b| (a.index, &a.name).cmp(&(b.index, &b.name)));
        Ok(v)
    }

    fn events(g: Grid<RawFullEvent>) -> Vec<Event> {
        g.entries.into_iter().map(|e| Event {
            id: e.event_id, channel_uuid: e.channel_uuid, channel_name: e.channel_name, start: e.start, stop: e.stop,
            title: e.title,
            summary: if e.summary.is_empty() { e.description } else { e.summary },
            dvr_uuid: e.dvr_uuid.filter(|s| !s.is_empty()), dvr_state: e.dvr_state.filter(|s| !s.is_empty()),
        }).collect()
    }

    /// Upcoming and running events of one channel (server default order: by start).
    pub async fn epg_channel(&self, channel_uuid: &str, limit: u32) -> Result<Vec<Event>, ApiError> {
        let l = limit.to_string();
        Ok(Self::events(self.json_q("/api/epg/events/grid", &[("channel", channel_uuid), ("limit", &l)]).await?))
    }

    /// EPG search. `fulltext` also searches summaries and descriptions.
    pub async fn epg_search(&self, text: &str, fulltext: bool, limit: u32) -> Result<Vec<Event>, ApiError> {
        let l = limit.to_string();
        let mut q = vec![("title", text), ("limit", l.as_str())];
        if fulltext { q.push(("fulltext", "1")); }
        Ok(Self::events(self.json_q("/api/epg/events/grid", &q).await?))
    }

    /// Recordings of one kind. Needs the server's recorder permission.
    pub async fn recordings(&self, kind: RecordingKind) -> Result<Vec<Recording>, ApiError> {
        let path = match kind {
            RecordingKind::Upcoming => "/api/dvr/entry/grid_upcoming",
            RecordingKind::Finished => "/api/dvr/entry/grid_finished",
            RecordingKind::Failed => "/api/dvr/entry/grid_failed",
        };
        let g: Grid<RawRecording> = self.json_q(path, &[("start", "0"), ("limit", "10000")]).await?;
        Ok(g.entries.into_iter().map(|r| Recording {
            uuid: r.uuid, channel_name: r.channelname, title: r.disp_title, subtitle: r.disp_subtitle, description: r.disp_description,
            start: r.start, stop: r.stop, status: r.status, sched_status: r.sched_status, filesize: r.filesize, errors: r.errors, enabled: r.enabled,
        }).collect())
    }

    /// Uuid of the default recording profile (the one with an empty name), else the first one.
    pub async fn default_dvr_config(&self) -> Result<String, ApiError> {
        let g: Grid<RawDvrConfig> = self.json("/api/dvr/config/grid").await?;
        g.entries.iter().find(|c| c.name.is_empty()).or(g.entries.first()).map(|c| c.uuid.clone())
            .ok_or_else(|| ApiError::BadResponse("the server has no recording profile".into()))
    }

    /// Schedule a recording from an EPG event.
    pub async fn record_event(&self, event_id: u32) -> Result<(), ApiError> {
        let cfg = self.default_dvr_config().await?;
        self.action("/api/dvr/entry/create_by_event", &[("config_uuid", &cfg), ("event_id", &event_id.to_string())]).await
    }

    /// Record a channel right now for `minutes` (a one-off timer). UNVERIFIED against a real server:
    /// field names follow the `dvr/entry/create` `conf` object (start, stop, channel, title).
    pub async fn record_now(&self, channel_uuid: &str, title: &str, minutes: u32) -> Result<(), ApiError> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let conf = serde_json::json!({
            "start": now, "stop": now + minutes as i64 * 60, "channel": channel_uuid,
            "title": { "eng": title }, "disp_title": title, "enabled": true
        });
        self.action("/api/dvr/entry/create", &[("conf", &conf.to_string())]).await
    }

    /// Stop a running recording gracefully (keeps the file).
    pub async fn stop_recording(&self, uuid: &str) -> Result<(), ApiError> { self.action("/api/dvr/entry/stop", &[("uuid", uuid)]).await }
    /// Delete a scheduled recording, or abort a running one.
    pub async fn cancel_recording(&self, uuid: &str) -> Result<(), ApiError> { self.action("/api/dvr/entry/cancel", &[("uuid", uuid)]).await }
    /// Remove a finished recording from storage.
    pub async fn remove_recording(&self, uuid: &str) -> Result<(), ApiError> { self.action("/api/dvr/entry/remove", &[("uuid", uuid)]).await }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forbidden_is_explained_as_account_rights() {
        assert!(ApiError::Status(403).user_message().contains("not allowed"));
        assert!(ApiError::Status(500).user_message().contains("500"));
    }
}
