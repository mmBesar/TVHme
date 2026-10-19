//! High-level session: login, permissions, channel list, live signal status.
use crate::client::{Client, Error, Events};
use crate::htsmsg::{Map, MapExt, Value};
use crate::permissions::{Permissions, ProbeError};
use crate::signal::SignalStatus;
use crate::stream::Server;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: u32,
    pub number: u32,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Htsp(#[from] Error),
    #[error(transparent)]
    Probe(#[from] ProbeError),
}

pub struct Session {
    pub server: Server,
    pub user: String,
    pass: String,
    pub permissions: Permissions,
    pub channels: Vec<Channel>,
    client: Arc<Client>,
    signal_tx: broadcast::Sender<SignalStatus>,
}

fn channel_from(m: &Map) -> Option<Channel> {
    Some(Channel {
        id: m.u32("channelId")?,
        number: m.u32("channelNumber").unwrap_or(0),
        name: m.str("channelName")?.to_string(),
        icon: m.str("channelIcon").map(str::to_string),
    })
}

impl Session {
    pub async fn connect(server: Server, user: &str, pass: &str) -> Result<Session, SessionError> {
        let permissions = Permissions::probe(&server.host, server.http_port, user, pass).await?;
        let (client, mut events): (Arc<Client>, Events) = Client::connect(&server.host, server.htsp_port).await?;
        client.login(user, pass).await?;
        client.request("enableAsyncMetadata", Map::new()).await?;
        let mut channels = Vec::new();
        while let Some(ev) = events.recv().await {
            match ev.str("method") {
                Some("channelAdd") => { if let Some(c) = channel_from(&ev) { channels.push(c); } }
                Some("initialSyncCompleted") => break,
                _ => {}
            }
        }
        channels.sort_by_key(|c| (c.number, c.name.clone()));
        let (signal_tx, _) = broadcast::channel(64);
        let tx = signal_tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                if let Some(s) = SignalStatus::from_msg(&ev) { let _ = tx.send(s); }
            }
        });
        Ok(Session { server, user: user.into(), pass: pass.into(), permissions, channels, client, signal_tx })
    }

    pub fn stream_url(&self, channel_id: u32) -> String {
        self.server.stream_url(channel_id, &self.user, &self.pass)
    }

    /// Start an HTSP subscription so the server reports `signalStatus` for it.
    pub async fn watch_signal(&self, channel_id: u32, subscription_id: u32) -> Result<broadcast::Receiver<SignalStatus>, Error> {
        let rx = self.signal_tx.subscribe();
        let mut f = Map::new();
        f.insert("channelId".into(), Value::S64(channel_id as i64));
        f.insert("subscriptionId".into(), Value::S64(subscription_id as i64));
        self.client.request("subscribe", f).await?;
        Ok(rx)
    }

    pub async fn stop_signal(&self, subscription_id: u32) -> Result<(), Error> {
        let mut f = Map::new();
        f.insert("subscriptionId".into(), Value::S64(subscription_id as i64));
        self.client.request("unsubscribe", f).await?;
        Ok(())
    }
}
