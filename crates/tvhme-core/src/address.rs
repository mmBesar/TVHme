//! Forgiving parsing of what a user types into the "server" field.
//!
//! Accepts `10.0.0.5`, `10.0.0.5:9981`, `tv.example.com`, `https://tv.example.com`,
//! `http://host:9981/tvh/` (webroot). A bare address without a port uses Tvheadend's
//! default HTTP port 9981; an explicit `http://`/`https://` without a port uses 80/443
//! (typical for a reverse proxy).
use reqwest::Url;

pub const DEFAULT_HTTP_PORT: u16 = 9981;
pub const DEFAULT_HTSP_PORT: u16 = 9982;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAddress {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Path prefix when Tvheadend runs under a webroot, e.g. "/tvh". Empty or starts with '/', no trailing '/'.
    pub webroot: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("enter the server address, for example 10.0.0.5 or https://tv.example.com")]
    Empty,
    #[error("only http:// and https:// addresses are supported")]
    BadScheme,
    #[error("that does not look like a server address: {0}")]
    Invalid(String),
}

impl ServerAddress {
    pub fn parse(input: &str) -> Result<Self, AddressError> {
        let s = input.trim();
        if s.is_empty() { return Err(AddressError::Empty); }
        let explicit_scheme = s.contains("://");
        let with_scheme = if explicit_scheme { s.to_string() } else { format!("http://{s}") };
        let url = Url::parse(&with_scheme).map_err(|e| AddressError::Invalid(e.to_string()))?;
        let tls = match url.scheme() { "http" => false, "https" => true, _ => return Err(AddressError::BadScheme) };
        let host = url.host_str().filter(|h| !h.is_empty()).ok_or_else(|| AddressError::Invalid(s.to_string()))?.to_string();
        let port = match url.port() {
            Some(p) => p,
            None if !explicit_scheme => DEFAULT_HTTP_PORT,
            None => if tls { 443 } else { 80 },
        };
        let webroot = url.path().trim_end_matches('/').to_string();
        Ok(Self { tls, host, port, webroot })
    }

    fn default_port(&self) -> u16 { if self.tls { 443 } else { 80 } }

    /// `http(s)://host[:port]` + webroot, no trailing slash.
    pub fn base_url(&self) -> String { self.base_with_userinfo("") }

    fn base_with_userinfo(&self, userinfo: &str) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        let port = if self.port == self.default_port() { String::new() } else { format!(":{}", self.port) };
        format!("{scheme}://{userinfo}{}{port}{}", self.host, self.webroot)
    }

    pub fn url(&self, path_and_query: &str) -> String {
        format!("{}{}", self.base_url(), path_and_query)
    }

    /// Playback URL for a channel uuid, with credentials as URL userinfo (mpv/VLC/ExoPlayer accept this).
    pub fn stream_url(&self, uuid: &str, user: &str, pass: &str) -> String {
        let ui = if user.is_empty() { String::new() } else { format!("{}:{}@", pct(user), pct(pass)) };
        format!("{}/stream/channel/{}", self.base_with_userinfo(&ui), uuid)
    }

    /// URL of a finished (or still running) recording file, `/dvrfile/<uuid>`, credentials as URL userinfo.
    pub fn dvr_url(&self, uuid: &str, user: &str, pass: &str) -> String {
        let ui = if user.is_empty() { String::new() } else { format!("{}:{}@", pct(user), pct(pass)) };
        format!("{}/dvrfile/{}", self.base_with_userinfo(&ui), uuid)
    }
}

/// `http://user:pass@host:9981/path?ticket=abc` -> `http://host:9981/path (ticket)`: where a URL points and
/// how it logs in, without any secret.
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let (hostpath, query) = rest.split_once('?').map_or((rest, ""), |x| x);
    let (auth, hostpath) = match hostpath.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (hostpath, String::new()),
    };
    let (has_userinfo, host) = match auth.rsplit_once('@') { Some((_, h)) => (true, h), None => (false, auth) };
    let mut how: Vec<&str> = Vec::new();
    if has_userinfo { how.push("password"); }
    for kv in query.split('&').filter(|k| !k.is_empty()) {
        match kv.split('=').next().unwrap_or("") { "ticket" => how.push("ticket"), "auth" | "AUTH" => how.push("code"), _ => {} }
    }
    let how = if how.is_empty() { String::new() } else { format!(" ({})", how.join(", ")) };
    format!("{}://{}{}{}", scheme, host, hostpath, how)
}

pub(crate) fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { o.push(b as char) } else { o.push_str(&format!("%{:02X}", b)) }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(s: &str) -> ServerAddress { ServerAddress::parse(s).unwrap() }

    #[test]
    fn bare_ip_and_host_use_tvheadend_default_port() {
        assert_eq!(p("10.0.0.5"), ServerAddress { tls: false, host: "10.0.0.5".into(), port: 9981, webroot: "".into() });
        assert_eq!(p("tv.lan:9999").port, 9999);
        assert_eq!(p("  tv.lan  ").host, "tv.lan");
    }
    #[test]
    fn explicit_scheme_uses_standard_ports() {
        let a = p("https://tv.example.com");
        assert!(a.tls && a.port == 443);
        assert_eq!(a.base_url(), "https://tv.example.com");
        assert_eq!(p("http://tv.example.com").port, 80);
        assert_eq!(p("https://tv.example.com:8443/").base_url(), "https://tv.example.com:8443");
    }
    #[test]
    fn webroot_is_kept_without_trailing_slash() {
        let a = p("https://example.com/tvh/");
        assert_eq!(a.webroot, "/tvh");
        assert_eq!(a.url("/api/x"), "https://example.com/tvh/api/x");
    }
    #[test]
    fn rejects_bad_input() {
        assert_eq!(ServerAddress::parse("  "), Err(AddressError::Empty));
        assert_eq!(ServerAddress::parse("ftp://host"), Err(AddressError::BadScheme));
        assert!(ServerAddress::parse("http://").is_err());
    }
    #[test]
    fn stream_url_has_encoded_credentials_and_uuid() {
        let a = p("tv.lan");
        assert_eq!(a.stream_url("abc123", "bob", "p@ss"), "http://bob:p%40ss@tv.lan:9981/stream/channel/abc123");
        assert_eq!(a.stream_url("abc123", "", ""), "http://tv.lan:9981/stream/channel/abc123");
        assert_eq!(redact_url("http://bob:p%40ss@tv.lan:9981/stream/channel/abc"), "http://tv.lan:9981/stream/channel/abc (password)");
        assert_eq!(redact_url("http://tv.lan:9981/stream/channelid/5?ticket=XYZ&profile=pass"), "http://tv.lan:9981/stream/channelid/5 (ticket)");
        assert_eq!(redact_url("https://t.example/tvh/stream/channel/a?auth=SECRET"), "https://t.example/tvh/stream/channel/a (code)");
        assert_eq!(a.dvr_url("r1", "bob", "p@ss"), "http://bob:p%40ss@tv.lan:9981/dvrfile/r1");
    }
}
