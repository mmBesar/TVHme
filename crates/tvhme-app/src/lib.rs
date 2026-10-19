slint::include_modules!();

use slint::{Color, ModelRc, VecModel};
use std::sync::{Arc, OnceLock};
use tvhme_core::session::Session;
use tvhme_core::stream::Server;
use tvhme_core::theme::{Accent, Flavor, ThemeChoice};

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap())
}

fn col(c: u32) -> Color { Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8) }

pub fn apply_theme(ui: &MainWindow, choice: ThemeChoice) {
    let t = ui.global::<Theme>();
    match choice.colors() {
        None => t.set_use_system(true),
        Some(c) => {
            t.set_use_system(false);
            t.set_accent(col(c.accent));
            t.set_c_text(col(c.text));
            t.set_c_subtext(col(c.subtext));
            t.set_c_base(col(c.base));
            t.set_c_mantle(col(c.mantle));
            t.set_c_surface0(col(c.surface0));
            t.set_c_surface1(col(c.surface1));
            t.set_c_ok(col(c.ok));
            t.set_c_warn(col(c.warn));
            t.set_c_bad(col(c.bad));
        }
    }
}

pub fn choice_from_indices(flavor: i32, accent: i32, mode: i32) -> ThemeChoice {
    if mode == 2 { return ThemeChoice::System; }
    ThemeChoice::Catppuccin {
        flavor: Flavor::ALL[(flavor.clamp(0, 3)) as usize],
        accent: Accent::ALL[(accent.clamp(0, 13)) as usize],
        amoled: mode == 1,
    }
}

fn open_player(url: &str) -> Result<(), String> {
    #[cfg(target_os = "android")]
    { let _ = url; Err("Android playback is not implemented yet".into()) }
    #[cfg(not(target_os = "android"))]
    {
        for p in ["mpv", "vlc", "xdg-open"] {
            if std::process::Command::new(p).arg(url).spawn().is_ok() { return Ok(()); }
        }
        Err("No player found (install mpv or vlc)".into())
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    apply_theme(&ui, ThemeChoice::default());
    let session: Arc<tokio::sync::Mutex<Option<Arc<Session>>>> = Default::default();
    let watcher: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>> = Default::default();

    ui.on_apply_theme({
        let w = ui.as_weak();
        move |f, a, m| { if let Some(ui) = w.upgrade() { apply_theme(&ui, choice_from_indices(f, a, m)); } }
    });

    ui.on_connect({
        let w = ui.as_weak();
        let session = session.clone();
        move |host, user, pass| {
            let (host, user, pass) = (host.to_string(), user.to_string(), pass.to_string());
            if let Some(ui) = w.upgrade() { ui.set_busy(true); ui.set_error("".into()); }
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                let server = Server::new(host.clone(), 9982, 9981);
                let res = Session::connect(server, &user, &pass).await;
                let w0 = w.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = w0.upgrade() else { return };
                    ui.set_busy(false);
                });
                match res {
                    Ok(s) => {
                        let s = Arc::new(s);
                        let items: Vec<ChannelItem> = s.channels.iter()
                            .map(|c| ChannelItem { id: c.id as i32, number: c.number as i32, name: c.name.clone().into() }).collect();
                        let admin = s.permissions.admin;
                        let info = format!("Connected to {host} as {user}");
                        *session.lock().await = Some(s);
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(ui) = w.upgrade() else { return };
                            ui.set_channels(ModelRc::new(VecModel::from(items)));
                            ui.set_is_admin(admin);
                            ui.set_server_info(info.into());
                            ui.set_screen(1);
                        });
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = w.upgrade() { ui.set_error(msg.into()); }
                        });
                    }
                }
            });
        }
    });

    ui.on_play({
        let (w, session, watcher) = (ui.as_weak(), session.clone(), watcher.clone());
        move |id| {
            let (w, session, watcher) = (w.clone(), session.clone(), watcher.clone());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let id = id as u32;
                let name = s.channels.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default();
                if let Some(h) = watcher.lock().unwrap().take() { h.abort(); }
                let _ = s.stop_signal(1).await;
                let w2 = w.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w2.upgrade() { ui.set_now_playing(name.into()); ui.set_snr(-1.0); ui.set_ss(-1.0); }
                });
                if let Err(e) = open_player(&s.stream_url(id)) {
                    let w2 = w.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w2.upgrade() { ui.set_now_playing(format!("{e}").into()); }
                    });
                }
                if let Ok(mut rx) = s.watch_signal(id, 1).await {
                    let h = tokio::spawn(async move {
                        while let Ok(st) = rx.recv().await {
                            let w = w.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = w.upgrade() {
                                    ui.set_snr(st.snr_pct.unwrap_or(-1.0));
                                    ui.set_ss(st.signal_pct.unwrap_or(-1.0));
                                }
                            });
                        }
                    });
                    *watcher.lock().unwrap() = Some(h);
                }
            });
        }
    });

    ui.on_stop_playback({
        let (w, session, watcher) = (ui.as_weak(), session.clone(), watcher.clone());
        move || {
            if let Some(h) = watcher.lock().unwrap().take() { h.abort(); }
            if let Some(ui) = w.upgrade() { ui.set_now_playing("".into()); }
            let session = session.clone();
            rt().spawn(async move { if let Some(s) = session.lock().await.clone() { let _ = s.stop_signal(1).await; } });
        }
    });

    ui.run()
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).unwrap();
    run().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_indices_map_correctly() {
        assert_eq!(choice_from_indices(2, 3, 0), ThemeChoice::default());
        assert_eq!(choice_from_indices(3, 12, 1), ThemeChoice::Catppuccin { flavor: Flavor::Mocha, accent: Accent::Blue, amoled: true });
        assert_eq!(choice_from_indices(0, 0, 2), ThemeChoice::System);
        assert_eq!(choice_from_indices(99, -4, 0), ThemeChoice::Catppuccin { flavor: Flavor::Mocha, accent: Accent::Rosewater, amoled: false });
    }
}
