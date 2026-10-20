//! Handing a stream URL to an external video player.
//!
//! Linux/desktop: start mpv or VLC as a child process we can wait for and stop.
//! Android: ask the system to open the URL, optionally in a specific app (VLC or mpv).

/// Names for the player choice in Settings. Index 0 is always "let the system/automatic decide".
#[cfg(not(target_os = "android"))]
pub fn names() -> Vec<&'static str> { vec!["Automatic (mpv, else VLC)", "mpv", "VLC"] }
#[cfg(target_os = "android")]
pub fn names() -> Vec<&'static str> { vec!["Ask every time", "VLC", "mpv"] }

#[cfg(not(target_os = "android"))]
pub fn spawn(url: &str, title: &str, choice: i32) -> Result<tokio::process::Child, String> {
    use tokio::process::Command;
    let order: &[&str] = match choice { 1 => &["mpv"], 2 => &["vlc"], _ => &["mpv", "vlc"] };
    for p in order {
        let mut c = Command::new(p);
        if *p == "mpv" { c.arg("--no-terminal").arg(format!("--force-media-title={title}")); }
        match c.arg(url).kill_on_drop(false).spawn() {
            Ok(child) => return Ok(child),
            Err(_) => continue,
        }
    }
    Err(format!("{} not found: install it, or pick another player in Settings", order.join(" / ")))
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
