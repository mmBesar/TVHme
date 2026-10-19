//! Permissions are decided by the server; the client only discovers them.
use crate::http;

/// Admin-only endpoint used as the probe: 200 = admin, 403 = authenticated non-admin.
/// ASSUMPTION (not verified against a real server): `/api/config` is admin-only.
pub const ADMIN_PROBE_PATH: &str = "/api/config";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section { Channels, Guide, Recordings, Settings, Status }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub admin: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("invalid username or password")]
    BadCredentials,
    #[error("unexpected server response {0}")]
    Unexpected(u16),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Permissions {
    pub fn from_probe_status(status: u16) -> Result<Self, ProbeError> {
        match status {
            200 => Ok(Self { admin: true }),
            403 => Ok(Self { admin: false }),
            401 => Err(ProbeError::BadCredentials),
            s => Err(ProbeError::Unexpected(s)),
        }
    }
    pub async fn probe(host: &str, http_port: u16, user: &str, pass: &str) -> Result<Self, ProbeError> {
        let (status, _) = http::get(host, http_port, ADMIN_PROBE_PATH, user, pass).await?;
        Self::from_probe_status(status)
    }
    /// Sections the UI should offer. Management screens only for admins; every
    /// action is still enforced by the server.
    pub fn sections(&self) -> Vec<Section> {
        let mut v = vec![Section::Channels, Section::Guide, Section::Recordings];
        if self.admin { v.push(Section::Settings); v.push(Section::Status); }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_status_codes() {
        assert!(Permissions::from_probe_status(200).unwrap().admin);
        assert!(!Permissions::from_probe_status(403).unwrap().admin);
        assert!(matches!(Permissions::from_probe_status(401), Err(ProbeError::BadCredentials)));
        assert!(Permissions::from_probe_status(500).is_err());
    }
    #[test]
    fn non_admin_has_no_management() {
        let s = Permissions { admin: false }.sections();
        assert!(!s.contains(&Section::Settings) && !s.contains(&Section::Status));
        assert!(Permissions { admin: true }.sections().contains(&Section::Settings));
    }
}
