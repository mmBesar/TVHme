//! Optional desktop integration, only when the user asks for it ("Add to app menu" in Settings, or
//! `tvhme --install-launcher`): a launcher entry and the icon in all sizes in the user's own folders.
//! Nothing here runs on its own. A portable AppImage stays portable until the person decides otherwise.

const ICONS: [(u32, &[u8]); 6] = [
    (32, include_bytes!("../../../assets/icons/32x32.png")),
    (48, include_bytes!("../../../assets/icons/48x48.png")),
    (64, include_bytes!("../../../assets/icons/64x64.png")),
    (128, include_bytes!("../../../assets/icons/128x128.png")),
    (256, include_bytes!("../../../assets/icons/256x256.png")),
    (512, include_bytes!("../../../assets/icons/512x512.png")),
];
const ICON_SVG: &[u8] = include_bytes!("../../../assets/icon.svg");
const MARK: &str = "X-TVHme-Managed=true";

/// The text of the launcher entry for a program at `exec`.
pub fn desktop_entry(exec: &str) -> String {
    // Exec values are split on spaces: quote the path and escape what the format treats specially
    let quoted = format!("\"{}\"", exec.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`"));
    format!("[Desktop Entry]\nType=Application\nName=TVHme\nGenericName=Tvheadend client\nComment=Watch and manage Tvheadend\nExec={quoted}\nIcon=tvhme\nTerminal=false\nCategories=AudioVideo;Video;TV;Network;\nStartupWMClass=tvhme\n{MARK}\n")
}

#[cfg(not(target_os = "android"))]
mod fs_part {
    use super::*;
    use std::path::PathBuf;

    fn data_dir() -> Result<PathBuf, String> {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
            .ok_or_else(|| "No home folder found".to_string())
    }

    fn entry_path() -> Result<PathBuf, String> { Ok(data_dir()?.join("applications").join("tvhme.desktop")) }

    /// The file the person started: the AppImage itself when run from one, else this program.
    fn program() -> Result<String, String> {
        std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok())
            .and_then(|p| p.to_str().map(str::to_string)).ok_or_else(|| "Cannot tell where TVHme is stored".to_string())
    }

    /// Is a launcher entry made by TVHme in place?
    pub fn installed() -> bool {
        entry_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| t.contains(MARK))
    }

    pub fn install() -> Result<String, String> {
        let (data, exec) = (data_dir()?, program()?);
        let entry = entry_path()?;
        // never replace an entry the person or a package made
        if let Ok(t) = std::fs::read_to_string(&entry) {
            if !t.contains(MARK) { return Err(format!("{} already exists and was not made by TVHme; left alone", entry.display())); }
        }
        let err = |e: std::io::Error| e.to_string();
        for (size, png) in ICONS {
            let dir = data.join("icons").join("hicolor").join(format!("{size}x{size}")).join("apps");
            std::fs::create_dir_all(&dir).map_err(err)?;
            std::fs::write(dir.join("tvhme.png"), png).map_err(err)?;
        }
        let sdir = data.join("icons").join("hicolor").join("scalable").join("apps");
        std::fs::create_dir_all(&sdir).map_err(err)?;
        std::fs::write(sdir.join("tvhme.svg"), ICON_SVG).map_err(err)?;
        std::fs::create_dir_all(data.join("applications")).map_err(err)?;
        std::fs::write(&entry, desktop_entry(&exec)).map_err(err)?;
        Ok(format!("Added TVHme to the app menu ({exec}). Some panels need a restart to notice."))
    }

    pub fn uninstall() -> Result<String, String> {
        let data = data_dir()?;
        let entry = entry_path()?;
        match std::fs::read_to_string(&entry) {
            Ok(t) if t.contains(MARK) => { std::fs::remove_file(&entry).map_err(|e| e.to_string())?; }
            Ok(_) => return Err("The menu entry was not made by TVHme; left alone".into()),
            Err(_) => {}
        }
        for (size, _) in ICONS { let _ = std::fs::remove_file(data.join("icons").join("hicolor").join(format!("{size}x{size}")).join("apps").join("tvhme.png")); }
        let _ = std::fs::remove_file(data.join("icons").join("hicolor").join("scalable").join("apps").join("tvhme.svg"));
        Ok("Removed TVHme from the app menu".into())
    }

    /// Command-line switches that do their job and exit. `None` means: start the app normally.
    pub fn cli() -> Option<i32> {
        let arg = std::env::args().nth(1)?;
        let result = match arg.as_str() {
            "--install-launcher" => install(),
            "--uninstall-launcher" => uninstall(),
            "--help" | "-h" => {
                println!("TVHme\n\n  --install-launcher    add TVHme and its icon to your app menu (this user only)\n  --uninstall-launcher  remove them again");
                return Some(0);
            }
            _ => return None,
        };
        Some(match result { Ok(m) => { println!("{m}"); 0 } Err(e) => { eprintln!("{e}"); 1 } })
    }
}
#[cfg(not(target_os = "android"))]
pub use fs_part::{cli, install, installed, uninstall};

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
    fn icons_are_pngs_of_the_right_size() {
        for (size, png) in ICONS {
            assert_eq!(&png[1..4], b"PNG");
            let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
            assert_eq!(w, size);
        }
    }
    #[cfg(not(target_os = "android"))]
    #[test]
    fn install_and_remove_in_a_scratch_home_and_never_touch_foreign_entries() {
        let home = std::env::temp_dir().join(format!("tvhme-integrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("XDG_DATA_HOME", &home);
        assert!(!installed());
        install().unwrap();
        assert!(installed());
        assert!(home.join("icons/hicolor/512x512/apps/tvhme.png").exists() && home.join("icons/hicolor/scalable/apps/tvhme.svg").exists());
        uninstall().unwrap();
        assert!(!installed() && !home.join("icons/hicolor/128x128/apps/tvhme.png").exists());
        // an entry made by someone else is left alone
        std::fs::create_dir_all(home.join("applications")).unwrap();
        std::fs::write(home.join("applications/tvhme.desktop"), "[Desktop Entry]\nExec=tvhme\n").unwrap();
        assert!(install().is_err() && uninstall().is_err());
        assert!(home.join("applications/tvhme.desktop").exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}
