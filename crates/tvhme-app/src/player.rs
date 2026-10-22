//! Handing a stream URL to an external video player.
//!
//! Linux/desktop: start mpv or VLC as a child process we can wait for and stop.
//! Android: ask the system to open the URL, optionally in a specific app (VLC or mpv).

/// Names for the player choice in Settings. Index 0 is always "let the system/automatic decide".
#[cfg(not(target_os = "android"))]
pub fn names() -> Vec<&'static str> { vec!["Automatic (mpv, else VLC)", "mpv", "VLC"] }
#[cfg(target_os = "android")]
pub fn names() -> Vec<&'static str> { vec!["Ask every time", "VLC", "mpv"] }

/// What `play` did: reused the mpv window that is already open, or started a new player process.
#[cfg(not(target_os = "android"))]
pub enum Launched { Reused, Spawned { child: tokio::process::Child, mpv: bool } }

#[cfg(not(target_os = "android"))]
mod ctl {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use serde_json::json;

    /// Socket of the mpv we started (mpv's JSON IPC), if one is running.
    static MPV: Mutex<Option<PathBuf>> = Mutex::new(None);
    /// File that is being written by "record while watching", if any.
    static RECORDING: Mutex<Option<PathBuf>> = Mutex::new(None);

    #[cfg(unix)]
    fn send_raw(sock: &Path, line: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut s = std::os::unix::net::UnixStream::connect(sock)?;
        s.write_all(line.as_bytes())
    }
    #[cfg(not(unix))]
    fn send_raw(_sock: &Path, _line: &str) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "player control needs a Unix system"))
    }

    fn sock() -> Option<PathBuf> { MPV.lock().ok().and_then(|g| g.clone()) }

    fn ipc(cmd: serde_json::Value) -> Result<(), String> {
        let sock = sock().ok_or("mpv is not running")?;
        send_raw(&sock, &format!("{cmd}\n")).map_err(|e| e.to_string())
    }

    pub fn mpv_alive() -> bool {
        match sock() { Some(p) => send_raw(&p, "\n").is_ok(), None => false }
    }

    pub fn forget() {
        if let Ok(mut g) = MPV.lock() { if let Some(p) = g.take() { let _ = std::fs::remove_file(p); } }
    }

    fn socket_path() -> PathBuf {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        dir.join(format!("tvhme-mpv-{}.sock", std::process::id()))
    }

    /// Play `url`. mpv that we already started is reused (the channel changes, the window stays);
    /// otherwise mpv or VLC is started. mpv gets a control socket so Stop and recording can reach it.
    pub fn play(url: &str, title: &str, choice: i32, langs: &str) -> Result<super::Launched, String> {
        use tokio::process::Command;
        if choice != 2 && mpv_alive() {
            if !langs.is_empty() { let _ = ipc(json!({"command": ["set_property", "alang", langs]})); }
            let _ = ipc(json!({"command": ["set_property", "force-media-title", title]}));
            ipc(json!({"command": ["loadfile", url, "replace"]}))?;
            return Ok(super::Launched::Reused);
        }
        let order: &[&str] = match choice { 1 => &["mpv"], 2 => &["vlc"], _ => &["mpv", "vlc"] };
        for p in order {
            let mut c = Command::new(p);
            let is_mpv = *p == "mpv";
            let sock = socket_path();
            if is_mpv {
                let _ = std::fs::remove_file(&sock);
                c.arg("--no-terminal").arg("--idle=yes").arg("--force-window=yes")
                    .arg(format!("--force-media-title={title}"))
                    .arg(format!("--input-ipc-server={}", sock.display()));
                if !langs.is_empty() { c.arg(format!("--alang={langs}")); }
            }
            match c.arg(url).kill_on_drop(false).spawn() {
                Ok(child) => {
                    if is_mpv { if let Ok(mut g) = MPV.lock() { *g = Some(sock); } }
                    return Ok(super::Launched::Spawned { child, mpv: is_mpv });
                }
                Err(_) => continue,
            }
        }
        Err(format!("{} not found: install it, or pick another player in Settings", order.join(" / ")))
    }

    /// Close the player we started (Stop, sign out, quit). Closing it also closes the stream, so the tuner is free.
    pub fn quit() { let _ = ipc(json!({"command": ["quit"]})); forget(); }

    pub fn is_recording() -> bool { RECORDING.lock().map(|g| g.is_some()).unwrap_or(false) }

    /// Start saving what is playing right now. `dir` empty means Videos/TVHme.
    pub fn record_start(dir: &str, title: &str) -> Result<PathBuf, String> {
        if !mpv_alive() { return Err("Recording while watching needs mpv (started from TVHme)".into()); }
        let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M").to_string();
        let path = super::record_path(dir, title, &stamp).ok_or("Could not create the recording folder")?;
        ipc(json!({"command": ["set_property", "stream-record", path.to_string_lossy()]}))?;
        if let Ok(mut g) = RECORDING.lock() { *g = Some(path.clone()); }
        Ok(path)
    }

    /// Stop saving. Returns the file that was written, if one was being written.
    pub fn record_stop() -> Option<PathBuf> {
        let p = RECORDING.lock().ok()?.take()?;
        let _ = ipc(json!({"command": ["set_property", "stream-record", ""]}));
        Some(p)
    }
}
#[cfg(not(target_os = "android"))]
pub use ctl::{forget, is_recording, play, quit, record_start, record_stop};

/// After a recording: optionally keep only the audio languages the user prefers (needs ffmpeg).
/// Returns a one-line message for the user. The original file stays unless the filtered copy worked.
#[cfg(not(target_os = "android"))]
pub fn finalize_recording(path: &std::path::Path, langs: &str, only_preferred: bool) -> String {
    use std::process::Command;
    let shown = path.display();
    let wanted: Vec<String> = langs.split(',').map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
    if !only_preferred || wanted.is_empty() { return format!("Saved {shown}"); }
    let probe = Command::new("ffprobe").args(["-v", "error", "-select_streams", "a", "-show_entries", "stream_tags=language", "-of", "csv=p=0"]).arg(path).output();
    let Ok(probe) = probe else { return format!("Saved {shown} with all audio (ffmpeg is not installed)") };
    let have: Vec<String> = String::from_utf8_lossy(&probe.stdout).lines().map(|l| l.trim().to_lowercase()).collect();
    if !wanted.iter().any(|w| have.contains(w)) { return format!("Saved {shown} with all audio (none of {langs} found)"); }
    let tmp = path.with_extension("filtered.ts");
    let mut c = Command::new("ffmpeg");
    c.args(["-y", "-v", "error", "-i"]).arg(path).args(["-map", "0:v?", "-map", "0:s?"]);
    for w in &wanted { c.arg("-map").arg(format!("0:a:m:language:{w}?")); }
    c.args(["-c", "copy", "-f", "mpegts"]).arg(&tmp);
    match c.status() {
        Ok(st) if st.success() && std::fs::rename(&tmp, path).is_ok() => format!("Saved {shown} (audio: {langs})"),
        _ => { let _ = std::fs::remove_file(&tmp); format!("Saved {shown} with all audio (could not filter)") }
    }
}

/// Open `url` in an Android app. `choice`: 0 = system chooser, 1 = VLC, 2 = mpv.
#[cfg(target_os = "android")]
pub fn open(url: &str, choice: i32) -> Result<(), String> {
    use jni::objects::{JObject, JValue};
    let (package, label) = match choice { 1 => (Some("org.videolan.vlc"), "VLC"), 2 => (Some("is.xyz.mpv"), "mpv"), _ => (None, "a video player") };
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let res: Result<(), jni::errors::Error> = (|| {
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
        let jurl = env.new_string(url)?;
        let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&jurl)])?.l()?;
        let mime = env.new_string("video/*")?;
        env.call_method(&intent, "setDataAndType", "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&uri), JValue::Object(&mime)])?;
        if let Some(p) = package {
            let jp = env.new_string(p)?;
            env.call_method(&intent, "setPackage", "(Ljava/lang/String;)Landroid/content/Intent;", &[JValue::Object(&jp)])?;
        }
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])?; // FLAG_ACTIVITY_NEW_TASK
        env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    })();
    if env.exception_check().unwrap_or(false) { let _ = env.exception_clear(); return Err(format!("Could not open {label}. Is it installed?")); }
    res.map_err(|e| format!("Could not open {label}: {e}"))
}

/// Where "record on this computer" saves to: `<Videos>/TVHme/<title> <date time>.ts`. Creates the folder.
#[cfg(not(target_os = "android"))]
pub fn record_path(dir: &str, title: &str, stamp: &str) -> Option<std::path::PathBuf> {
    let dir = if dir.trim().is_empty() {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        std::path::PathBuf::from(home).join("Videos").join("TVHme")
    } else { std::path::PathBuf::from(dir.trim()) };
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(record_file_name(title, stamp)))
}

#[cfg(not(target_os = "android"))]
fn record_file_name(title: &str, stamp: &str) -> String {
    let clean: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let clean = clean.trim();
    format!("{} {}.ts", if clean.is_empty() { "recording" } else { clean }, stamp)
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    #[test]
    fn file_names_are_safe() {
        assert_eq!(record_file_name("BBC One / HD", "2026-10-22 05-00"), "BBC One _ HD 2026-10-22 05-00.ts");
        assert!(!record_file_name("../..", "x").contains('/'));
        assert_eq!(record_file_name("", "x"), "recording x.ts");
    }

    fn have(tool: &str) -> bool { std::process::Command::new(tool).arg("-version").output().is_ok() }
    fn audio_langs_of(p: &std::path::Path) -> Vec<String> {
        let o = std::process::Command::new("ffprobe").args(["-v", "error", "-select_streams", "a", "-show_entries", "stream_tags=language", "-of", "csv=p=0"]).arg(p).output().unwrap();
        String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).collect()
    }

    #[test]
    fn keeps_only_preferred_audio_languages() {
        if !have("ffmpeg") || !have("ffprobe") { return; } // needs ffmpeg, as the feature does
        let dir = std::env::temp_dir().join(format!("tvhme-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("two-audio.ts");
        let st = std::process::Command::new("ffmpeg").args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=d=1:s=64x64:r=10", "-f", "lavfi", "-i", "sine=d=1", "-f", "lavfi", "-i", "sine=f=800:d=1",
            "-map", "0", "-map", "1", "-map", "2", "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=ara", "-c:v", "mpeg2video", "-c:a", "mp2", "-f", "mpegts"]).arg(&f).status().unwrap();
        assert!(st.success());
        assert_eq!(audio_langs_of(&f).len(), 2);
        // not asked to filter: untouched
        assert!(finalize_recording(&f, "ara", false).starts_with("Saved"));
        assert_eq!(audio_langs_of(&f).len(), 2);
        // a language that is not there: keep everything
        assert!(finalize_recording(&f, "fra", true).contains("all audio"));
        assert_eq!(audio_langs_of(&f).len(), 2);
        // filter to Arabic
        let msg = finalize_recording(&f, "ara", true);
        assert!(msg.contains("audio: ara"), "{msg}");
        assert_eq!(audio_langs_of(&f), vec!["ara".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
