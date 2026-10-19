slint::include_modules!();

use slint::{Color, ModelRc, VecModel};
use std::sync::{Arc, OnceLock};
use tvhme_core::session::{ConnectOptions, Session, SignalSource, SignalWatch};
use tvhme_core::signal::SignalStatus;
use tvhme_core::theme::{Accent, Flavor, ThemeChoice};

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap())
}

fn col(c: u32) -> Color { Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8) }

pub fn apply_theme(ui: &MainWindow, choice: ThemeChoice) {
    let t = ui.global::<Theme>();
    match choice.colors() {
        None => { t.set_use_system(true); }
        Some(c) => {
            t.set_use_system(false);
            t.set_dark(c.dark);
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

fn signal_texts(s: &SignalStatus) -> (f32, String, f32, String) {
    let snr = match (s.snr_pct, s.snr_db) {
        (Some(p), _) => (p, format!("{:.0}%", p)),
        (None, Some(db)) => ((db / 30.0 * 100.0).clamp(0.0, 100.0), format!("{db:.1} dB")),
        _ => (-1.0, String::new()),
    };
    let ss = match (s.signal_pct, s.signal_db) {
        (Some(p), _) => (p, format!("{:.0}%", p)),
        (None, Some(db)) => (-1.0, format!("{db:.1} dB")), // no known full-scale for absolute signal: text only
        _ => (-1.0, String::new()),
    };
    (snr.0, snr.1, ss.0, ss.1)
}

pub fn run() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    apply_theme(&ui, ThemeChoice::default());
    let session: Arc<tokio::sync::Mutex<Option<Arc<Session>>>> = Default::default();
    let watch: Arc<std::sync::Mutex<Option<(SignalWatch, tokio::task::JoinHandle<()>)>>> = Default::default();

    ui.on_apply_theme({
        let w = ui.as_weak();
        move |f, a, m| { if let Some(ui) = w.upgrade() { apply_theme(&ui, choice_from_indices(f, a, m)); } }
    });

    ui.on_connect({
        let w = ui.as_weak();
        let session = session.clone();
        move |address, htsp, user, pass, insecure| {
            if let Some(ui) = w.upgrade() { ui.set_busy(true); ui.set_error("".into()); }
            let opts = ConnectOptions {
                address: address.to_string(), user: user.to_string(), pass: pass.to_string(),
                htsp_port: htsp.parse::<u16>().ok(), insecure_tls: insecure,
            };
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                let res = Session::connect(opts.clone()).await;
                match res {
                    Ok(s) => {
                        let s = Arc::new(s);
                        let items: Vec<ChannelItem> = s.channels.iter().map(|c| ChannelItem {
                            uuid: c.uuid.clone().into(), number: c.number as i32, name: c.name.clone().into(),
                            now: c.now.as_ref().map(|n| n.title.clone()).unwrap_or_default().into(),
                        }).collect();
                        let admin = s.permissions.admin;
                        let info = format!("Connected to {} as {}{}", s.api.addr.base_url(), opts.user, if s.has_htsp() { "" } else { " (HTSP not reachable: live signal only for admins)" });
                        *session.lock().await = Some(s);
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(ui) = w.upgrade() else { return };
                            ui.set_busy(false);
                            ui.set_channels(ModelRc::new(VecModel::from(items)));
                            ui.set_is_admin(admin);
                            ui.set_server_info(info.into());
                            ui.set_screen(1);
                        });
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = w.upgrade() { ui.set_busy(false); ui.set_error(msg.into()); }
                        });
                    }
                }
            });
        }
    });

    ui.on_play({
        let (w, session, watch) = (ui.as_weak(), session.clone(), watch.clone());
        move |uuid| {
            let (w, session, watch) = (w.clone(), session.clone(), watch.clone());
            let uuid = uuid.to_string();
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let name = s.channels.iter().find(|c| c.uuid == uuid).map(|c| c.name.clone()).unwrap_or_default();
                drop(watch.lock().unwrap().take()); // stops the previous subscription/poller
                let w2 = w.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w2.upgrade() { ui.set_now_playing(name.into()); ui.set_play_note("".into()); ui.set_has_signal(false); }
                });
                if let Err(e) = open_player(&s.stream_url(&uuid)) {
                    let w2 = w.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w2.upgrade() { ui.set_play_note(e.into()); }
                    });
                }
                let sw = s.watch_signal(&uuid, 1).await;
                if sw.source == SignalSource::Unavailable {
                    let w2 = w.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w2.upgrade() { ui.set_play_note("Signal info needs HTSP access or an admin account".into()); }
                    });
                }
                let mut rx = sw.rx.clone();
                let h = tokio::spawn(async move {
                    while rx.changed().await.is_ok() {
                        let Some(st) = rx.borrow().clone() else { continue };
                        let (snr, snr_t, ss, ss_t) = signal_texts(&st);
                        let w = w.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = w.upgrade() {
                                ui.set_snr(snr); ui.set_ss(ss);
                                ui.set_snr_text(snr_t.into()); ui.set_ss_text(ss_t.into());
                                ui.set_has_signal(true);
                            }
                        });
                    }
                });
                *watch.lock().unwrap() = Some((sw, h));
            });
        }
    });

    ui.on_stop_playback({
        let (w, watch) = (ui.as_weak(), watch.clone());
        move || {
            if let Some((_sw, h)) = watch.lock().unwrap().take() { h.abort(); }
            if let Some(ui) = w.upgrade() { ui.set_now_playing("".into()); ui.set_has_signal(false); }
        }
    });

    // Developer aid: TVHME_AUTOCONNECT=address,htsp_port,user,pass[,play_channel_uuid]
    if let Ok(v) = std::env::var("TVHME_AUTOCONNECT") {
        let p: Vec<String> = v.split(',').map(str::to_string).collect();
        if p.len() >= 4 {
            ui.invoke_connect(p[0].clone().into(), p[1].clone().into(), p[2].clone().into(), p[3].clone().into(), false);
            if let Some(ch) = p.get(4).cloned() {
                let w = ui.as_weak();
                rt().spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    let _ = slint::invoke_from_event_loop(move || { if let Some(ui) = w.upgrade() { ui.invoke_play(ch.into()); } });
                });
            }
        }
    }
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
