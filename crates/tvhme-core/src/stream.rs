//! Playback URLs for Tvheadend's HTTP streaming.
#[derive(Clone, Debug)]
pub struct Server {
    pub host: String,
    pub htsp_port: u16,
    pub http_port: u16,
}

impl Server {
    pub fn new(host: impl Into<String>, htsp_port: u16, http_port: u16) -> Self {
        Self { host: host.into(), htsp_port, http_port }
    }
    /// Stream URL for a channel id. Credentials are embedded as userinfo so
    /// players such as mpv/ExoPlayer authenticate; empty user = anonymous.
    pub fn stream_url(&self, channel_id: u32, user: &str, pass: &str) -> String {
        let auth = if user.is_empty() { String::new() } else { format!("{}:{}@", pct(user), pct(pass)) };
        format!("http://{}{}:{}/stream/channelid/{}", auth, self.host, self.http_port, channel_id)
    }
}

fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { o.push(b as char) } else { o.push_str(&format!("%{:02X}", b)) }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_urls() {
        let s = Server::new("10.0.0.5", 9982, 9981);
        assert_eq!(s.stream_url(42, "bob", "p@ss"), "http://bob:p%40ss@10.0.0.5:9981/stream/channelid/42");
        assert_eq!(s.stream_url(1, "", ""), "http://10.0.0.5:9981/stream/channelid/1");
    }
}
