//! Tiny HTTP/1.1 GET client (no TLS) used for permission probing.
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

pub fn basic_auth(user: &str, pass: &str) -> String {
    format!("Basic {}", b64(format!("{user}:{pass}").as_bytes()))
}

/// GET `path`; returns (status code, body). Body capped at 1 MiB.
pub async fn get(host: &str, port: u16, path: &str, user: &str, pass: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let mut s = TcpStream::connect((host, port)).await?;
    let auth = if user.is_empty() { String::new() } else { format!("Authorization: {}\r\n", basic_auth(user, pass)) };
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\n{auth}Connection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    s.take(1 << 20).read_to_end(&mut buf).await?;
    let text_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4).unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..text_end]);
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad HTTP status line"))?;
    Ok((status, buf[text_end..].to_vec()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_basic() {
        assert_eq!(super::b64(b"Aladdin:open sesame"), "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert_eq!(super::b64(b"a"), "YQ==");
    }
}
