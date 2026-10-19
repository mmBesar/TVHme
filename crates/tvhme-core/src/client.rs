//! Async HTSP client: request/response matching by `seq`, async events on a channel.
use crate::htsmsg::{self, Map, MapExt, Value};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("connection closed")]
    Closed,
    #[error("server error: {0}")]
    Server(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("unexpected reply: {0}")]
    Protocol(&'static str),
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Map>>>>;

pub struct Client {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    seq: AtomicI64,
}

pub type Events = mpsc::UnboundedReceiver<Map>;

impl Client {
    /// Connect and start the reader task. Unsolicited messages arrive on `Events`.
    pub async fn connect(host: &str, port: u16) -> Result<(Arc<Client>, Events), Error> {
        let stream = TcpStream::connect((host, port)).await?;
        stream.set_nodelay(true).ok();
        let (mut rd, wr) = stream.into_split();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (etx, erx) = mpsc::unbounded_channel();
        let p2 = pending.clone();
        tokio::spawn(async move {
            while let Ok(msg) = htsmsg::read_message(&mut rd).await {
                if let Some(seq) = msg.i64("seq") {
                    if let Some(tx) = p2.lock().unwrap().remove(&seq) {
                        let _ = tx.send(msg);
                        continue;
                    }
                }
                if etx.send(msg).is_err() { break; }
            }
            // Dropping senders wakes waiters with Closed.
            p2.lock().unwrap().clear();
        });
        let c = Client { writer: tokio::sync::Mutex::new(wr), pending, seq: AtomicI64::new(1) };
        Ok((Arc::new(c), erx))
    }

    pub async fn request(&self, method: &str, mut fields: Map) -> Result<Map, Error> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        fields.insert("method".into(), Value::Str(method.into()));
        fields.insert("seq".into(), Value::S64(seq));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(seq, tx);
        {
            let mut w = self.writer.lock().await;
            w.write_all(&htsmsg::encode_message(&fields)).await?;
        }
        let reply = rx.await.map_err(|_| Error::Closed)?;
        if let Some(e) = reply.str("error") { return Err(Error::Server(e.to_string())); }
        Ok(reply)
    }

    /// `hello` then `authenticate` using sha1(password ++ challenge).
    pub async fn login(&self, user: &str, pass: &str) -> Result<Map, Error> {
        let mut f = Map::new();
        f.insert("htspversion".into(), Value::S64(34));
        f.insert("clientname".into(), Value::Str("TVHme".into()));
        let hello = self.request("hello", f).await?;
        let challenge = hello.bin("challenge").ok_or(Error::Protocol("hello without challenge"))?.to_vec();
        if user.is_empty() { return Ok(hello); }
        let mut h = Sha1::new();
        h.update(pass.as_bytes());
        h.update(&challenge);
        let mut a = Map::new();
        a.insert("username".into(), Value::Str(user.into()));
        a.insert("digest".into(), Value::Bin(h.finalize().to_vec()));
        let reply = self.request("authenticate", a).await?;
        if reply.i64("noaccess").unwrap_or(0) != 0 { return Err(Error::AuthFailed); }
        Ok(hello)
    }
}
