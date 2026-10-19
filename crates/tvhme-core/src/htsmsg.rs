//! HTSP binary message codec (htsmsg).
//!
//! Frame: u32 big-endian length, then fields. Field: type u8, name length u8,
//! data length u32 BE, name bytes, data bytes. Integers (S64) are little-endian
//! with trailing zero bytes trimmed. Lists are fields with empty names.
use std::collections::BTreeMap;
use tokio::io::{AsyncRead, AsyncReadExt};

pub const T_MAP: u8 = 1;
pub const T_S64: u8 = 2;
pub const T_STR: u8 = 3;
pub const T_BIN: u8 = 4;
pub const T_LIST: u8 = 5;
const MAX_FRAME: usize = 16 * 1024 * 1024;

pub type Map = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Map(Map),
    S64(i64),
    Str(String),
    Bin(Vec<u8>),
    List(Vec<Value>),
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("truncated message")]
    Truncated,
    #[error("unknown field type {0}")]
    UnknownType(u8),
    #[error("integer longer than 8 bytes")]
    BadInt,
    #[error("invalid utf-8")]
    BadUtf8,
    #[error("frame too large ({0} bytes)")]
    TooLarge(usize),
}

pub trait MapExt {
    fn str(&self, k: &str) -> Option<&str>;
    fn i64(&self, k: &str) -> Option<i64>;
    fn u32(&self, k: &str) -> Option<u32>;
    fn bin(&self, k: &str) -> Option<&[u8]>;
}
impl MapExt for Map {
    fn str(&self, k: &str) -> Option<&str> {
        match self.get(k) { Some(Value::Str(s)) => Some(s), _ => None }
    }
    fn i64(&self, k: &str) -> Option<i64> {
        match self.get(k) { Some(Value::S64(n)) => Some(*n), _ => None }
    }
    fn u32(&self, k: &str) -> Option<u32> { self.i64(k).and_then(|n| u32::try_from(n).ok()) }
    fn bin(&self, k: &str) -> Option<&[u8]> {
        match self.get(k) { Some(Value::Bin(b)) => Some(b), _ => None }
    }
}

fn put_field(out: &mut Vec<u8>, name: &str, v: &Value) {
    let (t, data): (u8, Vec<u8>) = match v {
        Value::Map(m) => (T_MAP, encode_fields(m)),
        Value::S64(n) => {
            let mut b = (*n as u64).to_le_bytes().to_vec();
            while b.last() == Some(&0) { b.pop(); }
            (T_S64, b)
        }
        Value::Str(s) => (T_STR, s.as_bytes().to_vec()),
        Value::Bin(b) => (T_BIN, b.clone()),
        Value::List(l) => {
            let mut d = Vec::new();
            for e in l { put_field(&mut d, "", e); }
            (T_LIST, d)
        }
    };
    out.push(t);
    out.push(name.len() as u8);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&data);
}

fn encode_fields(m: &Map) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in m { put_field(&mut out, k, v); }
    out
}

/// Encode a message including the 4-byte length prefix.
pub fn encode_message(m: &Map) -> Vec<u8> {
    let body = encode_fields(m);
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

fn decode_fields(mut b: &[u8]) -> Result<Vec<(String, Value)>, DecodeError> {
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 6 { return Err(DecodeError::Truncated); }
        let t = b[0];
        let nl = b[1] as usize;
        let dl = u32::from_be_bytes([b[2], b[3], b[4], b[5]]) as usize;
        b = &b[6..];
        if b.len() < nl + dl { return Err(DecodeError::Truncated); }
        let name = std::str::from_utf8(&b[..nl]).map_err(|_| DecodeError::BadUtf8)?.to_string();
        let data = &b[nl..nl + dl];
        b = &b[nl + dl..];
        let v = match t {
            T_MAP => Value::Map(decode_fields(data)?.into_iter().collect()),
            T_S64 => {
                if data.len() > 8 { return Err(DecodeError::BadInt); }
                let mut a = [0u8; 8];
                a[..data.len()].copy_from_slice(data);
                Value::S64(u64::from_le_bytes(a) as i64)
            }
            T_STR => Value::Str(std::str::from_utf8(data).map_err(|_| DecodeError::BadUtf8)?.to_string()),
            T_BIN => Value::Bin(data.to_vec()),
            T_LIST => Value::List(decode_fields(data)?.into_iter().map(|(_, v)| v).collect()),
            other => return Err(DecodeError::UnknownType(other)),
        };
        out.push((name, v));
    }
    Ok(out)
}

/// Decode a message body (without the length prefix).
pub fn decode_body(b: &[u8]) -> Result<Map, DecodeError> {
    Ok(decode_fields(b)?.into_iter().collect())
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

/// Read one framed message from a stream.
pub async fn read_message<R: AsyncRead + Unpin>(r: &mut R) -> Result<Map, ReadError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME { return Err(DecodeError::TooLarge(n).into()); }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(decode_body(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Map {
        let mut inner = Map::new();
        inner.insert("x".into(), Value::S64(-5));
        let mut m = Map::new();
        m.insert("method".into(), Value::Str("hello".into()));
        m.insert("zero".into(), Value::S64(0));
        m.insert("big".into(), Value::S64(0x1_0000_0001));
        m.insert("bin".into(), Value::Bin(vec![0, 1, 2]));
        m.insert("list".into(), Value::List(vec![Value::S64(7), Value::Str("a".into()), Value::Map(inner)]));
        m
    }
    #[test]
    fn roundtrip() {
        let m = sample();
        let enc = encode_message(&m);
        assert_eq!(u32::from_be_bytes(enc[..4].try_into().unwrap()) as usize, enc.len() - 4);
        assert_eq!(decode_body(&enc[4..]).unwrap(), m);
    }
    #[test]
    fn s64_is_trimmed_little_endian() {
        let mut m = Map::new();
        m.insert("a".into(), Value::S64(0x0102));
        let enc = encode_message(&m);
        // len(4) type(1) namelen(1) datalen(4) name(1) data(2)
        assert_eq!(&enc[4..], &[T_S64, 1, 0, 0, 0, 2, b'a', 0x02, 0x01]);
        let mut z = Map::new();
        z.insert("a".into(), Value::S64(0));
        assert_eq!(&encode_message(&z)[4..], &[T_S64, 1, 0, 0, 0, 0, b'a']);
    }
    #[test]
    fn rejects_garbage() {
        assert!(decode_body(&[9, 0, 0, 0, 0, 0]).is_err());
        assert!(decode_body(&[T_STR, 0, 0, 0, 0, 9]).is_err());
    }
}
