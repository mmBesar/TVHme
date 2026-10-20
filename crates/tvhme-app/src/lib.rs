slint::include_modules!();

mod config;
mod fmt;
mod player;

use slint::{Color, Model, ModelRc, VecModel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tvhme_core::api::{Event, RecordingKind};
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


type Shared = Arc<tokio::sync::Mutex<Option<Arc<Session>>>>;

fn ui_do(w: &slint::Weak<MainWindow>, f: impl FnOnce(&MainWindow) + Send + 'static) {
    let w = w.clone();
    let _ = slint::invoke_from_event_loop(move || { if let Some(ui) = w.upgrade() { f(&ui); } });
}

fn event_items(evs: &[Event]) -> Vec<EventItem> {
    let (now, off) = (fmt::now_ts(), fmt::local_offset());
    evs.iter().map(|e| EventItem {
        id: e.id as i32,
        channel: e.channel_name.clone().into(),
        title: e.title.clone().into(),
        when: fmt::range(e.start, e.stop, now, off).into(),
        summary: e.summary.clone().into(),
        state: e.dvr_state.clone().unwrap_or_default().into(),
        dvr_uuid: e.dvr_uuid.clone().unwrap_or_default().into(),
    }).collect()
}

fn channel_items(s: &Session, tag: &str) -> Vec<ChannelItem> {
    s.channels.iter().filter(|c| tag.is_empty() || c.tags.iter().any(|t| t == tag)).map(|c| ChannelItem {
        uuid: c.uuid.clone().into(), number: c.number as i32, name: c.name.clone().into(),
        now: c.now.as_ref().map(|n| n.title.clone()).unwrap_or_default().into(),
    }).collect()
}

fn nav_items(admin: bool) -> Vec<NavItem> {
    let mut v = vec![(0, "Live"), (1, "Guide"), (2, "Recordings"), (3, "Search")];
    if admin { v.push((4, "Server")); }
    v.push((5, "Settings"));
    v.into_iter().map(|(id, l)| NavItem { id, label: l.into() }).collect()
}

fn load_recordings(w: &slint::Weak<MainWindow>, session: &Shared, kind: i32) {
    let (w, session) = (w.clone(), session.clone());
    rt().spawn(async move {
        let Some(s) = session.lock().await.clone() else { return };
        let k = match kind { 1 => RecordingKind::Finished, 2 => RecordingKind::Failed, _ => RecordingKind::Upcoming };
        let res = s.api.recordings(k).await;
        ui_do(&w, move |ui| match res {
            Ok(list) => {
                let (now, off) = (fmt::now_ts(), fmt::local_offset());
                let items: Vec<RecItem> = list.iter().map(|r| RecItem {
                    uuid: r.uuid.clone().into(), title: r.title.clone().into(), channel: r.channel_name.clone().into(),
                    when: fmt::range(r.start, r.stop, now, off).into(),
                    status: (if r.status.is_empty() { r.sched_status.clone() } else { r.status.clone() }).into(),
                    running: r.sched_status == "recording",
                }).collect();
                ui.set_rec_note(if items.is_empty() { "Nothing here".into() } else { "".into() });
                ui.set_recordings(ModelRc::new(VecModel::from(items)));
            }
            Err(e) => { ui.set_recordings(ModelRc::default()); ui.set_rec_note(e.user_message().into()); }
        });
    });
}


/// What is currently being watched: the external player (killed when this is dropped) and, for live TV,
/// the signal subscription. Dropping it releases the tuner on the server.
struct Playback {
    gen: u64,
    _kill: tokio::sync::oneshot::Sender<()>,
    signal: Option<(SignalWatch, tokio::task::JoinHandle<()>)>,
}
impl Drop for Playback {
    fn drop(&mut self) { if let Some((_, h)) = &self.signal { h.abort(); } }
}
type Slot = Arc<std::sync::Mutex<Option<Playback>>>;
static GEN: AtomicU64 = AtomicU64::new(1);

enum Target { Channel(String), Recording(String) }

fn start_playback(w: slint::Weak<MainWindow>, session: Shared, slot: Slot, target: Target, title: String, choice: i32) {
    rt().spawn(async move {
        let Some(s) = session.lock().await.clone() else { return };
        let gen = GEN.fetch_add(1, Ordering::SeqCst);
        drop(slot.lock().unwrap().take()); // stop the previous player and subscription first
        {
            let t = title.clone();
            ui_do(&w, move |ui| { ui.set_now_playing(t.into()); ui.set_play_note("".into()); ui.set_has_signal(false); });
        }
        let url = match &target { Target::Channel(u) => s.playback_url(u).await, Target::Recording(u) => s.recording_url(u) };
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();

        #[cfg(not(target_os = "android"))]
        let child = match player::spawn(&url, &title, choice) {
            Ok(c) => Some(c),
            Err(e) => { ui_do(&w, move |ui| ui.set_play_note(e.into())); None }
        };
        #[cfg(target_os = "android")]
        {
            if let Err(e) = player::open(&url, choice) { ui_do(&w, move |ui| ui.set_play_note(e.into())); }
        }

        let mut signal = None;
        if let Target::Channel(uuid) = &target {
            let sw = s.watch_signal(uuid, 1).await;
            if sw.source == SignalSource::Unavailable {
                ui_do(&w, |ui| ui.set_play_note("Signal info needs HTSP access or an admin account".into()));
            }
            let mut rx = sw.rx.clone();
            let w2 = w.clone();
            let h = tokio::spawn(async move {
                while rx.changed().await.is_ok() {
                    let Some(st) = rx.borrow().clone() else { continue };
                    let (snr, snr_t, ss, ss_t) = signal_texts(&st);
                    ui_do(&w2, move |ui| {
                        ui.set_snr(snr); ui.set_ss(ss);
                        ui.set_snr_text(snr_t.into()); ui.set_ss_text(ss_t.into());
                        ui.set_has_signal(true);
                    });
                }
            });
            signal = Some((sw, h));
        }
        {
            // If the user already started something newer while we were launching, the newer one wins.
            let mut g = slot.lock().unwrap();
            let pb = Playback { gen, _kill: kill_tx, signal };
            if g.as_ref().map_or(true, |p| p.gen < gen) { *g = Some(pb); } else { drop(pb); }
        }

        // When the player window is closed, release the signal/tuner (but not if a newer playback replaced us).
        #[cfg(not(target_os = "android"))]
        if let Some(mut child) = child {
            let (w, slot) = (w.clone(), slot.clone());
            tokio::spawn(async move {
                let mut kill_rx = kill_rx;
                tokio::select! {
                    _ = child.wait() => {
                        let mut g = slot.lock().unwrap();
                        if g.as_ref().map(|p| p.gen) == Some(gen) {
                            drop(g.take());
                            drop(g);
                            ui_do(&w, |ui| { ui.set_now_playing("".into()); ui.set_has_signal(false); ui.set_play_note("".into()); });
                        }
                    }
                    _ = &mut kill_rx => { let _ = child.kill().await; } // Stop pressed, or another channel started
                }
            });
        }
        #[cfg(target_os = "android")]
        drop(kill_rx);
    });
}

pub fn run() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    // Fallback in case the Android backend reports no safe-area inset: keep content below the status bar.
    #[cfg(target_os = "android")]
    ui.set_extra_top(28.0);
    let saved = config::load();
    ui.set_host(saved.address.clone().into());
    ui.set_htsp_port(saved.htsp_port.clone().into());
    ui.set_user(saved.user.clone().into());
    ui.set_password(saved.pass.clone().into());
    ui.set_insecure(saved.insecure);
    ui.set_t_mode(saved.mode); ui.set_t_flavor(saved.flavor); ui.set_t_accent(saved.accent);
    ui.set_player(saved.player);
    ui.set_player_names(ModelRc::new(VecModel::from(player::names().into_iter().map(slint::SharedString::from).collect::<Vec<_>>())));
    apply_theme(&ui, choice_from_indices(saved.flavor, saved.accent, saved.mode));
    let session: Shared = Default::default();
    let slot: Slot = Default::default();

    ui.on_apply_theme({
        let w = ui.as_weak();
        move |f, a, m| {
            if let Some(ui) = w.upgrade() { apply_theme(&ui, choice_from_indices(f, a, m)); }
            config::update(|c| { c.flavor = f; c.accent = a; c.mode = m; });
        }
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
                        let items = channel_items(&s, "");
                        let names: Vec<slint::SharedString> = s.channels.iter().map(|c| c.name.clone().into()).collect();
                        let tags: Vec<TagItem> = s.api.tags().await.unwrap_or_default().into_iter()
                            .map(|t| TagItem { uuid: t.uuid.into(), name: t.name.into(), selected: false }).collect();
                        let admin = s.permissions.admin;
                        let info = format!("{}{}", s.api.addr.base_url(), if s.has_htsp() { "" } else { " (HTSP not reachable: live signal only for admins)" });
                        let me = format!("Signed in as {}{}", if opts.user.is_empty() { "anonymous".to_string() } else { opts.user.clone() }, if admin { " · Administrator" } else { "" });
                        config::update(|c| {
                            c.address = opts.address.clone(); c.htsp_port = opts.htsp_port.map(|p| p.to_string()).unwrap_or_default();
                            c.user = opts.user.clone(); c.pass = opts.pass.clone(); c.insecure = opts.insecure_tls;
                        });
                        *session.lock().await = Some(s);
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(ui) = w.upgrade() else { return };
                            ui.set_busy(false);
                            ui.set_channels(ModelRc::new(VecModel::from(items)));
                            ui.set_channel_names(ModelRc::new(VecModel::from(names)));
                            ui.set_tags(ModelRc::new(VecModel::from(tags)));
                            ui.set_nav(ModelRc::new(VecModel::from(nav_items(admin))));
                            ui.set_is_admin(admin);
                            ui.set_server_info(info.into());
                            ui.set_me_text(me.into());
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

    ui.on_tab_changed({
        let (w, session) = (ui.as_weak(), session.clone());
        move |t| match t {
            1 => { if let Some(ui) = w.upgrade() { ui.invoke_load_guide(ui.get_guide_index()); } }
            2 => { if let Some(ui) = w.upgrade() { load_recordings(&w, &session, ui.get_rec_kind()); } }
            4 => { if let Some(ui) = w.upgrade() { ui.invoke_refresh_server(); } }
            _ => {}
        }
    });

    ui.on_select_tag({
        let (w, session) = (ui.as_weak(), session.clone());
        move |uuid| {
            let (w, session, uuid) = (w.clone(), session.clone(), uuid.to_string());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                ui_do(&w, move |ui| {
                    // tapping the active chip clears the filter
                    let cur: Vec<TagItem> = ui.get_tags().iter().collect();
                    let was = cur.iter().any(|t| t.uuid == uuid.as_str() && t.selected);
                    let sel = if was { String::new() } else { uuid };
                    let next: Vec<TagItem> = cur.into_iter().map(|mut t| { t.selected = t.uuid == sel.as_str(); t }).collect();
                    ui.set_tags(ModelRc::new(VecModel::from(next)));
                    ui.set_channels(ModelRc::new(VecModel::from(channel_items(&s, &sel))));
                });
            });
        }
    });

    ui.on_record_now({
        let (w, session) = (ui.as_weak(), session.clone());
        move |uuid, name| {
            let (w, session, uuid, name) = (w.clone(), session.clone(), uuid.to_string(), name.to_string());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let title = s.channels.iter().find(|c| c.uuid == uuid).and_then(|c| c.now.as_ref()).map(|n| n.title.clone()).unwrap_or_else(|| name.clone());
                let msg = match s.api.record_now(&uuid, &title, 60).await {
                    Ok(()) => format!("Recording {name} for 60 min"),
                    Err(e) => e.user_message(),
                };
                ui_do(&w, move |ui| ui.set_toast(msg.into()));
            });
        }
    });

    ui.on_load_guide({
        let (w, session) = (ui.as_weak(), session.clone());
        move |idx| {
            let (w, session) = (w.clone(), session.clone());
            ui_do(&w, |ui| ui.set_guide_busy(true));
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let Some(ch) = s.channels.get(idx.max(0) as usize) else { return };
                let res = s.api.epg_channel(&ch.uuid, 100).await;
                ui_do(&w, move |ui| {
                    ui.set_guide_busy(false);
                    match res {
                        Ok(ev) => ui.set_guide_events(ModelRc::new(VecModel::from(event_items(&ev)))),
                        Err(e) => { ui.set_guide_events(ModelRc::default()); ui.set_toast(e.user_message().into()); }
                    }
                });
            });
        }
    });

    ui.on_do_search({
        let (w, session) = (ui.as_weak(), session.clone());
        move |text, full| {
            let (w, session, text) = (w.clone(), session.clone(), text.to_string());
            if text.trim().is_empty() { return; }
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let res = s.api.epg_search(text.trim(), full, 100).await;
                ui_do(&w, move |ui| match res {
                    Ok(ev) => {
                        ui.set_search_note(if ev.is_empty() { "No matches".into() } else { format!("{} result(s)", ev.len()).into() });
                        ui.set_search_events(ModelRc::new(VecModel::from(event_items(&ev))));
                    }
                    Err(e) => { ui.set_search_events(ModelRc::default()); ui.set_search_note(e.user_message().into()); }
                });
            });
        }
    });

    ui.on_record_event({
        let (w, session) = (ui.as_weak(), session.clone());
        move |id| {
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let msg = match s.api.record_event(id as u32).await { Ok(()) => "Scheduled".to_string(), Err(e) => e.user_message() };
                ui_do(&w, move |ui| {
                    ui.set_toast(msg.into());
                    if ui.get_tab() == 1 { ui.invoke_load_guide(ui.get_guide_index()); }
                });
            });
        }
    });

    // stop / cancel / remove share one shape
    macro_rules! rec_action {
        ($setter:ident, $method:ident, $ok:expr) => {
            ui.$setter({
                let (w, session) = (ui.as_weak(), session.clone());
                move |uuid| {
                    let (w, session, uuid) = (w.clone(), session.clone(), uuid.to_string());
                    rt().spawn(async move {
                        let Some(s) = session.lock().await.clone() else { return };
                        let msg = match s.api.$method(&uuid).await { Ok(()) => $ok.to_string(), Err(e) => e.user_message() };
                        let w2 = w.clone();
                        ui_do(&w2, move |ui| ui.set_toast(msg.into()));
                        let kind = { let (tx, rx) = std::sync::mpsc::channel(); ui_do(&w, move |ui| { let _ = tx.send(ui.get_rec_kind()); }); rx.recv().unwrap_or(0) };
                        load_recordings(&w, &session, kind);
                    });
                }
            });
        };
    }
    rec_action!(on_stop_recording, stop_recording, "Recording stopped");
    rec_action!(on_cancel_recording, cancel_recording, "Recording cancelled");
    rec_action!(on_remove_recording, remove_recording, "Recording removed");

    ui.on_load_recordings({
        let (w, session) = (ui.as_weak(), session.clone());
        move |k| load_recordings(&w, &session, k)
    });

    ui.on_play({
        let (w, session, slot) = (ui.as_weak(), session.clone(), slot.clone());
        move |uuid| {
            let Some(ui) = w.upgrade() else { return };
            let title = ui.get_channels().iter().find(|c| c.uuid == uuid).map(|c| c.name.to_string()).unwrap_or_default();
            start_playback(w.clone(), session.clone(), slot.clone(), Target::Channel(uuid.to_string()), title, ui.get_player());
        }
    });

    ui.on_play_recording({
        let (w, session, slot) = (ui.as_weak(), session.clone(), slot.clone());
        move |uuid| {
            let Some(ui) = w.upgrade() else { return };
            let title = ui.get_recordings().iter().find(|r| r.uuid == uuid).map(|r| r.title.to_string()).unwrap_or_else(|| "Recording".into());
            start_playback(w.clone(), session.clone(), slot.clone(), Target::Recording(uuid.to_string()), title, ui.get_player());
        }
    });

    ui.on_player_changed(|i| config::update(|c| c.player = i));

    ui.on_stop_playback({
        let (w, slot) = (ui.as_weak(), slot.clone());
        move || {
            drop(slot.lock().unwrap().take()); // also closes the player we started
            if let Some(ui) = w.upgrade() { ui.set_now_playing("".into()); ui.set_has_signal(false); ui.set_play_note("".into()); }
        }
    });

    ui.on_sign_out({
        let (w, session, slot) = (ui.as_weak(), session.clone(), slot.clone());
        move || {
            drop(slot.lock().unwrap().take());
            config::update(|c| c.pass.clear());
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                *session.lock().await = None;
                ui_do(&w, |ui| {
                    ui.set_password("".into()); ui.set_now_playing("".into()); ui.set_has_signal(false);
                    ui.set_tab(0); ui.set_screen(0);
                });
            });
        }
    });

    ui.on_refresh_server({
        let (w, session) = (ui.as_weak(), session.clone());
        move || {
            let (w, session) = (w.clone(), session.clone());
            if let Some(ui) = w.upgrade() { ui.set_server_busy(true); ui.set_server_error("".into()); }
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let res = s.server_overview().await;
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = w.upgrade() else { return };
                    ui.set_server_busy(false);
                    match res {
                        Ok(o) => {
                            let tuners: Vec<InfoRow> = o.inputs.iter().map(|i| {
                                let st = SignalStatus::from_input(i);
                                let (_, snr_t, _, ss_t) = signal_texts(&st);
                                let mut parts = vec![if i.subs > 0 { format!("{} · in use", i.stream) } else { "idle".to_string() }];
                                if !snr_t.is_empty() { parts.push(format!("SNR {snr_t}")); }
                                if !ss_t.is_empty() { parts.push(format!("SS {ss_t}")); }
                                if let Some(u) = i.unc { parts.push(format!("UNC {u}")); }
                                InfoRow { title: i.input.clone().into(), detail: parts.join(" · ").into() }
                            }).collect();
                            let streams: Vec<InfoRow> = o.subscriptions.iter().map(|s| InfoRow {
                                title: (if s.service.is_empty() { s.title.clone() } else { s.service.clone() }).into(),
                                detail: format!("{} · {} · {} error(s)", s.title, s.state, s.errors).into(),
                            }).collect();
                            let clients: Vec<InfoRow> = o.connections.iter().map(|c| InfoRow {
                                title: (if c.user.is_empty() { "anonymous".to_string() } else { c.user.clone() }).into(),
                                detail: format!("{} · {}", c.kind, c.peer).into(),
                            }).collect();
                            let ver = format!("{} {} (API {})", o.info.name, o.info.sw_version, o.info.api_version);
                            ui.set_tuners(ModelRc::new(VecModel::from(tuners)));
                            ui.set_streams(ModelRc::new(VecModel::from(streams)));
                            ui.set_clients(ModelRc::new(VecModel::from(clients)));
                            let base = ui.get_server_info().to_string();
                            if !base.contains(&o.info.sw_version) { ui.set_server_info(format!("{base}\n{ver}").into()); }
                        }
                        Err(e) => ui.set_server_error(e.to_string().into()),
                    }
                });
            });
        }
    });

    // Developer aid: TVHME_AUTOCONNECT=address,htsp_port,user,pass[,play_channel_uuid]
    if let Ok(v) = std::env::var("TVHME_AUTOCONNECT") {
        let p: Vec<String> = v.split(',').map(str::to_string).collect();
        if p.len() >= 4 {
            ui.invoke_connect(p[0].clone().into(), p[1].clone().into(), p[2].clone().into(), p[3].clone().into(), false);
            if let Ok(scr) = std::env::var("TVHME_TAB") {
                if let Ok(n) = scr.parse::<i32>() {
                    let w = ui.as_weak();
                    rt().spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        ui_do(&w, move |ui| { ui.set_tab(n); ui.invoke_tab_changed(n); });
                    });
                }
            }
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
    if let Some(dir) = app.internal_data_path() { config::set_dir(dir); }
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
