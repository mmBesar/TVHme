//! Permissions are decided by the server; the client only discovers them.
//!
//! Probes (from the Tvheadend API docs):
//! - `config/capabilities` needs the "Web interface" or "HTSP" privilege -> any usable account gets 200.
//! - `config/load` needs the admin privilege -> 200 for admins, 403 for everyone else.
//! Errors: 401 bad credentials, 403 missing privilege, 404 unknown function.
use crate::api::{Api, ApiError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section { Channels, Guide, Recordings, Settings, Status }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub admin: bool,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ProbeError {
    #[error("wrong username or password")]
    BadCredentials,
    #[error("this account has no \"Web interface\" permission on the server")]
    NoWebAccess,
    #[error("this does not look like a Tvheadend server (HTTP {0}). Check the address, port and webroot")]
    NotTvheadend(u16),
    #[error(transparent)]
    Api(#[from] ApiError),
}

impl Permissions {
    pub fn from_statuses(capabilities: u16, config_load: u16) -> Result<Self, ProbeError> {
        match capabilities {
            200 => {}
            401 => return Err(ProbeError::BadCredentials),
            403 => return Err(ProbeError::NoWebAccess),
            other => return Err(ProbeError::NotTvheadend(other)),
        }
        Ok(Self { admin: config_load == 200 })
    }

    pub async fn probe(api: &Api) -> Result<Self, ProbeError> {
        let caps = api.status("/api/config/capabilities").await?;
        // Only ask the admin-only endpoint when the account is otherwise valid.
        let load = if caps == 200 { api.status("/api/config/load").await? } else { 0 };
        Self::from_statuses(caps, load)
    }

    /// Sections the UI should offer. Management only for admins; the server still enforces every action.
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
        assert!(Permissions::from_statuses(200, 200).unwrap().admin);
        assert!(!Permissions::from_statuses(200, 403).unwrap().admin);
        assert_eq!(Permissions::from_statuses(401, 0), Err(ProbeError::BadCredentials));
        assert_eq!(Permissions::from_statuses(403, 0), Err(ProbeError::NoWebAccess));
        assert_eq!(Permissions::from_statuses(404, 0), Err(ProbeError::NotTvheadend(404)));
    }
    #[test]
    fn non_admin_has_no_management() {
        let s = Permissions { admin: false }.sections();
        assert!(!s.contains(&Section::Settings) && !s.contains(&Section::Status));
        assert!(Permissions { admin: true }.sections().contains(&Section::Settings));
    }
}
