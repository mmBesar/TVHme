//! What the app remembers between runs: the last server and login, the theme and the player.
//!
//! The file lives in the user's config folder (the app's private folder on Android). On Unix it is
//! readable by the owner only. The password is stored as plain text, like most Tvheadend clients do:
//! "Sign out" in Settings removes it.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub address: String,
    pub htsp_port: String,
    pub user: String,
    pub pass: String,
    pub insecure: bool,
    /// Persistent authentication code (optional alternative to the password for streams).
    pub auth_code: String,
    /// Go straight into the app at startup (cleared by "Sign out").
    pub auto_login: bool,
    /// Last channel group opened in Live and in the Guide (tag uuid; empty = all channels).
    pub last_tag: String,
    pub last_guide_tag: String,
    pub mode: i32,
    pub flavor: i32,
    pub accent: i32,
    pub player: i32,
    /// Desktop: also save live TV to a file on this computer while watching (mpv only).
    pub record_local: bool,
    /// Where recordings made on this computer go; empty = Videos/TVHme.
    pub record_dir: String,
    /// Preferred audio languages as ISO codes, e.g. "ara,eng". Used for playback and, optionally, for saved files.
    pub audio_langs: String,
    /// When saving on this computer keep only the preferred languages (needs ffmpeg); otherwise keep every audio track.
    pub audio_only_preferred: bool,
    /// Stream profile name (empty = server default).
    pub stream_profile: String,
}

impl Default for Saved {
    fn default() -> Self {
        // Catppuccin Macchiato + Mauve, HTSP on Tvheadend's usual port
        Saved { address: String::new(), htsp_port: "9982".into(), user: String::new(), pass: String::new(),
                insecure: false, auth_code: String::new(), auto_login: false, last_tag: String::new(), last_guide_tag: String::new(), mode: 0, flavor: 2, accent: 3, player: 0, record_local: false, record_dir: String::new(), audio_langs: String::new(), audio_only_preferred: false, stream_profile: String::new() }
    }
}

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Android has no home folder: the activity tells us where the app may write.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn set_dir(p: PathBuf) { let _ = DIR.set(p); }

fn default_path() -> Option<PathBuf> {
    let base = DIR.get().cloned()
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|p| PathBuf::from(p).join("tvhme")))
        .or_else(|| std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("tvhme")))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config").join("tvhme")))?;
    Some(base.join("config.json"))
}

pub fn load_from(path: &Path) -> Saved {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_to(path: &Path, c: &Saved) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(c).unwrap_or_default())?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?; }
    std::fs::rename(tmp, path) // replace in one step so a crash cannot leave half a file
}

pub fn load() -> Saved { default_path().map(|p| load_from(&p)).unwrap_or_default() }

/// Change some fields and write the file back; failures are ignored (remembering is a convenience).
pub fn update(f: impl FnOnce(&mut Saved)) {
    if let Some(p) = default_path() { let mut c = load_from(&p); f(&mut c); let _ = save_to(&p, &c); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_missing_file_and_partial_file() {
        let d = std::env::temp_dir().join(format!("tvhme-cfg-{}", std::process::id()));
        let p = d.join("config.json");
        assert_eq!(load_from(&p), Saved::default());
        let c = Saved { address: "10.0.0.5:9981".into(), user: "u".into(), pass: "p@ss".into(), accent: 12, ..Saved::default() };
        save_to(&p, &c).unwrap();
        assert_eq!(load_from(&p), c);
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600); }
        std::fs::write(&p, r#"{"user":"only"}"#).unwrap(); // older/partial files still load
        assert_eq!(load_from(&p).user, "only");
        std::fs::write(&p, "not json").unwrap();
        assert_eq!(load_from(&p), Saved::default());
        let _ = std::fs::remove_dir_all(d);
    }
}
