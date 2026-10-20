//! High-level session: HTTP API for everything (channels, now-playing, permissions, streams);
//! HTSP is optional and used only for live signal (SNR/SS) on a subscription.
use crate::address::{ServerAddress, DEFAULT_HTSP_PORT};
use crate::api::{Api, ApiError, Channel, ConnectionStatus, InputStatus, ServerInfo, SubscriptionStatus};
use crate::client::Client;
use crate::htsmsg::{Map, MapExt, Value};
use crate::permissions::{Permissions, ProbeError};
use crate::signal::SignalStatus;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch};

#[derive(Debug, Clone, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Probe(#[from] ProbeError),
    #[error(transparent)]
    Api(#[from] ApiError),
}

#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    pub address: String,
    pub user: String,
    pub pass: String,
    pub htsp_port: Option<u16>,
    pub insecure_tls: bool,
}

/// Everything the admin "Server" screen shows, fetched in one go.
#[derive(Clone, Debug, Default)]
pub struct ServerOverview {
    pub info: ServerInfo,
    pub inputs: Vec<InputStatus>,
    pub subscriptions: Vec<SubscriptionStatus>,
    pub connections: Vec<ConnectionStatus>,
}

pub struct Session {
    pub api: Api,
    pub permissions: Permissions,
    pub channels: Vec<Channel>,
    htsp: Option<Htsp>,
}

struct Htsp {
    client: Arc<Client>,
    /// channel uuid -> HTSP channelId
    ids: HashMap<String, u32>,
    signal_tx: broadcast::Sender<(u32, SignalStatus)>,
}

/// Where live signal numbers come from for the channel being watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalSource { Htsp, StatusInputs, Unavailable }

pub struct SignalWatch {
    pub source: SignalSource,
    pub rx: watch::Receiver<Option<SignalStatus>>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for SignalWatch {
    fn drop(&mut self) { if let Some(f) = self.stop.take() { f(); } }
}

impl Session {
    pub async fn connect(o: ConnectOptions) -> Result<Session, SessionError> {
        let addr = ServerAddress::parse(&o.address).map_err(|e| ApiError::BadResponse(e.to_string()))?;
        let api = Api::new(addr.clone(), &o.user, &o.pass, o.insecure_tls)?;
        let permissions = Permissions::probe(&api).await?;
        let mut channels = api.channels().await?;
        // Now-playing titles are a nicety; never fail the login over them.
        if let Ok(now) = api.now_events().await {
            for c in &mut channels { c.now = now.get(&c.uuid).cloned(); }
        }
        let htsp = Self::connect_htsp(&addr, &o, &channels).await;
        Ok(Session { api, permissions, channels, htsp })
    }

    /// Best effort: HTSP is a separate TCP port that is often not exposed behind a reverse proxy.
    async fn connect_htsp(addr: &ServerAddress, o: &ConnectOptions, channels: &[Channel]) -> Option<Htsp> {
        let port = o.htsp_port.unwrap_or(DEFAULT_HTSP_PORT);
        let attempt = async {
            let (client, mut events) = Client::connect(&addr.host, port).await.ok()?;
            client.login(&o.user, &o.pass).await.ok()?;
            client.request("enableAsyncMetadata", Map::new()).await.ok()?;
            let mut by_uuid = HashMap::new();
            let mut by_key = HashMap::new();
            while let Some(ev) = events.recv().await {
                match ev.str("method") {
                    Some("channelAdd") => {
                        if let Some(id) = ev.u32("channelId") {
                            if let Some(u) = ev.str("channelUuid") { by_uuid.insert(u.to_string(), id); }
                            by_key.insert((ev.u32("channelNumber").unwrap_or(0), ev.str("channelName").unwrap_or("").to_string()), id);
                        }
                    }
                    Some("initialSyncCompleted") | Some("initialSyncComplete") => break,
                    _ => {}
                }
            }
            // Prefer uuid; fall back to (number, name) when this server's HTSP does not send channelUuid.
            let ids: HashMap<String, u32> = channels.iter().filter_map(|c| {
                by_uuid.get(&c.uuid).or_else(|| by_key.get(&(c.number, c.name.clone()))).map(|id| (c.uuid.clone(), *id))
            }).collect();
            let (signal_tx, _) = broadcast::channel(64);
            let tx = signal_tx.clone();
            tokio::spawn(async move {
                while let Some(ev) = events.recv().await {
                    if let Some(s) = SignalStatus::from_htsp(&ev) { let _ = tx.send(s); }
                }
            });
            Some(Htsp { client, ids, signal_tx })
        };
        tokio::time::timeout(Duration::from_secs(8), attempt).await.ok().flatten()
    }

    /// Admin-only overview. `serverinfo` must succeed; the status lists degrade to empty if one call fails.
    pub async fn server_overview(&self) -> Result<ServerOverview, ApiError> {
        let (info, inputs, subs, conns) = tokio::join!(
            self.api.server_info(), self.api.inputs(), self.api.subscriptions(), self.api.connections());
        Ok(ServerOverview {
            info: info?,
            inputs: inputs.unwrap_or_default(),
            subscriptions: subs.unwrap_or_default(),
            connections: conns.unwrap_or_default(),
        })
    }

    pub fn has_htsp(&self) -> bool { self.htsp.is_some() }

    /// Credentials-in-URL fallback (`/stream/channel/<uuid>`). Prefer `playback_url`.
    pub fn stream_url(&self, uuid: &str) -> String { self.api.stream_url(uuid) }

    /// Preferred playback URL. As tvhclient does, ask the server for a one-time ticket over HTSP
    /// (`getTicket`) and play `<base><path>?ticket=..&profile=pass`, so no password is put in the URL.
    /// Falls back to the credentials URL when HTSP is not reachable or the server refuses.
    pub async fn playback_url(&self, uuid: &str) -> String {
        if let Some(h) = &self.htsp {
            if let Some(&cid) = h.ids.get(uuid) {
                let mut f = Map::new();
                f.insert("channelId".into(), Value::S64(cid as i64));
                if let Ok(r) = h.client.request("getTicket", f).await {
                    if let (Some(path), Some(ticket)) = (r.str("path"), r.str("ticket")) {
                        return format!("{}{}?ticket={}&profile=pass", self.api.addr.base_url(), path, ticket);
                    }
                }
            }
        }
        self.stream_url(uuid)
    }

    /// Start watching signal for a channel. Order: HTSP (any account) -> `status/inputs` (admin) -> unavailable.
    /// Dropping the returned value stops the subscription/polling.
    pub async fn watch_signal(self: &Arc<Self>, uuid: &str, subscription_id: u32) -> SignalWatch {
        let (tx, rx) = watch::channel(None);
        if let Some(h) = &self.htsp {
            if let Some(&cid) = h.ids.get(uuid) {
                let mut sub_rx = h.signal_tx.subscribe();
                let mut f = Map::new();
                f.insert("channelId".into(), Value::S64(cid as i64));
                f.insert("subscriptionId".into(), Value::S64(subscription_id as i64));
                if h.client.request("subscribe", f).await.is_ok() {
                    let task = tokio::spawn(async move {
                        while let Ok((id, s)) = sub_rx.recv().await {
                            if id == subscription_id { let _ = tx.send(Some(s)); }
                        }
                    });
                    let client = h.client.clone();
                    return SignalWatch { source: SignalSource::Htsp, rx, stop: Some(Box::new(move || {
                        task.abort();
                        tokio::spawn(async move {
                            let mut f = Map::new();
                            f.insert("subscriptionId".into(), Value::S64(subscription_id as i64));
                            let _ = client.request("unsubscribe", f).await;
                        });
                    })) };
                }
            }
        }
        if self.permissions.admin {
            let me = self.clone();
            let task = tokio::spawn(async move {
                loop {
                    if let Ok(inputs) = me.api.inputs().await {
                        // The input currently serving subscriptions with usable values.
                        if let Some(s) = inputs.iter().filter(|i| i.subs > 0).map(SignalStatus::from_input).find(|s| s.has_values()) {
                            let _ = tx.send(Some(s));
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            });
            return SignalWatch { source: SignalSource::StatusInputs, rx, stop: Some(Box::new(move || task.abort())) };
        }
        SignalWatch { source: SignalSource::Unavailable, rx, stop: None }
    }
}
