//! Desktop integration: a launcher entry and icon in the user's own folders, so panels, docks and
//! app menus can show the TVHme icon even when the program was started as a loose binary or AppImage.
//! Written only when missing or when it was written by us for a program that has since moved.

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");
const MARK: &str = "X-TVHme-Managed=true";

/// The text of the launcher entry for a program at `exec`.
pub fn desktop_entry(exec: &str) -> String {
    // Exec values are split on spaces: quote the path and escape what the format treats specially
    let quoted = format!("\"{}\"", exec.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`"));
    format!("[Desktop Entry]\nType=Application\nName=TVHme\nGenericName=Tvheadend client\nComment=Watch and manage Tvheadend\nExec={quoted}\nIcon=tvhme\nTerminal=false\nCategories=AudioVideo;Video;TV;Network;\nStartupWMClass=tvhme\n{MARK}\n")
}

/// Whether the entry at `existing` should be (re)written for `exec`.
fn needs_write(existing: Option<&str>, exec: &str) -> bool {
    match existing {
        None => true,
        Some(t) => t.contains(MARK) && !t.contains(&desktop_entry(exec)),
    }
}

/// Best effort: any failure is ignored (read-only home, sandbox...).
#[cfg(not(target_os = "android"))]
pub fn ensure() {
    use std::path::PathBuf;
    // An AppImage runs from a temporary mount: the file the user started is the one to launch again
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe.and_then(|p| p.to_str().map(str::to_string)) else { return };
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")));
    let Some(data) = data else { return };
    let entry = data.join("applications").join("tvhme.desktop");
    let existing = std::fs::read_to_string(&entry).ok();
    if !needs_write(existing.as_deref(), &exe) { return; }
    let icon_dir = data.join("icons").join("hicolor").join("256x256").join("apps");
    if std::fs::create_dir_all(&icon_dir).is_err() || std::fs::create_dir_all(data.join("applications")).is_err() { return; }
    let _ = std::fs::write(icon_dir.join("tvhme.png"), ICON_PNG);
    let _ = std::fs::write(&entry, desktop_entry(&exe));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entry_names_the_program_and_icon() {
        let t = desktop_entry("/home/me/My Apps/TVHme.AppImage");
        assert!(t.contains("Exec=\"/home/me/My Apps/TVHme.AppImage\""));
        assert!(t.contains("Icon=tvhme") && t.contains("StartupWMClass=tvhme") && t.contains(MARK));
    }
    #[test]
    fn writes_only_when_missing_or_stale_and_ours() {
        assert!(needs_write(None, "/a"));
        let ours = desktop_entry("/a");
        assert!(!needs_write(Some(&ours), "/a"));
        assert!(needs_write(Some(&ours), "/b")); // we moved: update our own entry
        assert!(!needs_write(Some("[Desktop Entry]\nExec=tvhme\n"), "/b")); // the user's or the installer's entry stays
    }
    #[test]
    fn icon_is_a_png() { assert_eq!(&ICON_PNG[1..4], b"PNG"); }
}
