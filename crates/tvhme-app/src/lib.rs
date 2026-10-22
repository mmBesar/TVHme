slint::include_modules!();

mod config;
mod fmt;
mod player;

use slint::{Color, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::cell::Cell;
use std::sync::{Arc, OnceLock};
use tvhme_core::api::{Event, RecordingKind};
use tvhme_core::session::{ConnectOptions, Session, SignalSource, SignalWatch};
use tvhme_core::signal::SignalStatus;
use tvhme_core::theme::{Accent, Flavor, ThemeChoice};

thread_local! {
    /// Channel logos by channel uuid. Slint images belong to the UI thread, hence thread-local.
    static LOGOS: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
}

/// "BBC One" -> "BO", "Mock Two" -> "MT", "Nile" -> "NI": shown when a channel has no logo.
fn initials(name: &str) -> String {
    let words: Vec<char> = name.split_whitespace().filter_map(|w| w.chars().next()).collect();
    let s: String = if words.len() >= 2 { words.iter().take(2).collect() } else { name.chars().take(2).collect() };
    s.to_uppercase()
}

thread_local! {
    /// Events seen recently, by id, so a tap on any list or timeline cell can open its details.
    static EVENTS: RefCell<HashMap<i32, Event>> = RefCell::new(HashMap::new());
    /// Start time of the timeline currently on screen.
    static TL_T0: Cell<i64> = const { Cell::new(0) };
}

thread_local! {
    /// Upcoming programmes per channel uuid (a few per channel is enough for Now / Next).
    static CHAN_EVENTS: RefCell<HashMap<String, Vec<Event>>> = RefCell::new(HashMap::new());
    /// Text typed in the channel filter of the Live page.
    static LIVE_FILTER: RefCell<String> = const { RefCell::new(String::new()) };
}

fn store_channel_events(uuid: &str, evs: &[Event]) {
    CHAN_EVENTS.with(|m| { m.borrow_mut().insert(uuid.to_string(), evs.to_vec()); });
}

/// The programme on air at `t` and the one after it, from a channel's events (sorted by start).
fn now_next(evs: &[Event], t: i64) -> (Option<&Event>, Option<&Event>) {
    let now = evs.iter().find(|e| e.start <= t && t < e.stop);
    let next = evs.iter().filter(|e| e.start > t).min_by_key(|e| e.start);
    (now, next)
}

fn apply_now_next(row: &mut ChannelItem, evs: &[Event], t: i64, off: chrono::FixedOffset) {
    let (now, next) = now_next(evs, t);
    row.now_title = now.map(|e| e.title.clone()).unwrap_or_default().into();
    row.now_progress = now.map(|e| ((t - e.start) as f32 / (e.stop - e.start).max(1) as f32).clamp(0.0, 1.0)).unwrap_or(0.0);
    row.next_title = next.map(|e| e.title.clone()).unwrap_or_default().into();
    row.next_time = next.map(|e| fmt::hm(e.start, off)).unwrap_or_default().into();
}

fn register(evs: &[Event]) {
    EVENTS.with(|m| { let mut m = m.borrow_mut(); if m.len() > 20_000 { m.clear(); } for e in evs { m.insert(e.id as i32, e.clone()); } });
}

fn logo_for(uuid: &str) -> (Image, bool) {
    LOGOS.with(|l| match l.borrow().get(uuid) { Some(i) => (i.clone(), true), None => (Image::default(), false) })
}

/// Decode a downloaded picture off the UI thread; the pixels are sent over, the Image is made on the UI thread.
fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > 256 || img.height() > 256 { img.thumbnail(256, 256) } else { img };
    let rgba = img.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// Fetch every channel logo in the background (a few at a time) and put each on screen as it arrives.
fn load_logos(w: slint::Weak<MainWindow>, s: Arc<Session>) {
    rt().spawn(async move {
        let gate = Arc::new(tokio::sync::Semaphore::new(6));
        for c in s.channels.iter().filter(|c| c.icon.is_some()) {
            let (uuid, src, s, w, gate) = (c.uuid.clone(), c.icon.clone().unwrap_or_default(), s.clone(), w.clone(), gate.clone());
            tokio::spawn(async move {
                let Ok(_p) = gate.acquire().await else { return };
                let Ok(bytes) = s.api.image(&src).await else { return };
                let Some((wd, ht, px)) = tokio::task::spawn_blocking(move || decode_rgba(&bytes)).await.ok().flatten() else { return };
                ui_do(&w, move |ui| {
                    let img = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&px, wd, ht));
                    LOGOS.with(|l| l.borrow_mut().insert(uuid.clone(), img.clone()));
                    let m = ui.get_channels();
                    for i in 0..m.row_count() {
                        if let Some(mut row) = m.row_data(i) {
                            if row.uuid == uuid.as_str() { row.logo = img.clone(); row.has_logo = true; m.set_row_data(i, row); }
                        }
                    }
                });
            });
        }
    });
}

fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap())
}

fn col(c: u32) -> Color { Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8) }

pub fn apply_theme(ui: &MainWindow, choice: ThemeChoice) {
    let t = ui.global::<Theme>();
    match choice.colors() {
        None => {
            t.set_use_system(true);
            if let ThemeChoice::System { accent } = choice {
                let (d, l) = accent.pair();
                t.set_accent_dark(col(d));
                t.set_accent_light(col(l));
            }
        }
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
    let accent = Accent::ALL[(accent.clamp(0, 13)) as usize];
    if mode == 2 { return ThemeChoice::System { accent }; }
    ThemeChoice::Catppuccin {
        flavor: Flavor::ALL[(flavor.clamp(0, 3)) as usize],
        accent,
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
    register(evs);
    let (now, off) = (fmt::now_ts(), fmt::local_offset());
    evs.iter().map(|e| EventItem {
        id: e.id as i32,
        channel: e.channel_name.clone().into(),
        title: e.title.clone().into(),
        when: fmt::range(e.start, e.stop, now, off).into(),
        summary: e.summary.clone().into(),
        state: e.dvr_state.clone().unwrap_or_default().into(),
        dvr_uuid: e.dvr_uuid.clone().unwrap_or_default().into(),
        logo: logo_for(&e.channel_uuid).0, has_logo: logo_for(&e.channel_uuid).1, initials: initials(&e.channel_name).into(),
        description: e.description.clone().into(), channel_uuid: e.channel_uuid.clone().into(),
        poster: Image::default(), has_poster: false,
        subtitle: e.subtitle.clone().into(), episode: e.episode.clone().into(),
        facts: e.facts.clone().into(), credits: e.credits.clone().into(),
        rtl: is_rtl(&e.title) || is_rtl(&e.summary),
    }).collect()
}

/// True when the text's first strong letter is from a right-to-left script (Arabic, Hebrew, Syriac, Thaana, N'Ko...).
pub fn is_rtl(s: &str) -> bool {
    for c in s.chars() {
        let u = c as u32;
        if matches!(u, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF) { return true; }
        if c.is_alphabetic() { return false; }
    }
    false
}

fn matches_filter(c: &tvhme_core::api::Channel, filter: &str) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || c.name.to_lowercase().contains(&f) || c.number.to_string().starts_with(&f)
}

fn channel_items(s: &Session, tag: &str, filter: &str) -> Vec<ChannelItem> {
    let (t, off) = (fmt::now_ts(), fmt::local_offset());
    s.channels.iter().filter(|c| tag.is_empty() || c.tags.iter().any(|x| x == tag)).filter(|c| matches_filter(c, filter)).map(|c| {
        let (logo, has_logo) = logo_for(&c.uuid);
        let mut row = ChannelItem {
            uuid: c.uuid.clone().into(), number: c.number as i32, name: c.name.clone().into(), logo, has_logo,
            initials: initials(&c.name).into(), now_title: "".into(), now_progress: 0.0, next_title: "".into(), next_time: "".into(),
        };
        CHAN_EVENTS.with(|m| if let Some(e) = m.borrow().get(&c.uuid) { apply_now_next(&mut row, e, t, off); });
        row
    }).collect()
}

/// Re-show the Live list for the selected group and the typed filter.
fn rebuild_live(w: &slint::Weak<MainWindow>, session: &Shared) {
    let (w, session) = (w.clone(), session.clone());
    rt().spawn(async move {
        let Some(s) = session.lock().await.clone() else { return };
        ui_do(&w, move |ui| {
            let filter = LIVE_FILTER.with(|f| f.borrow().clone());
            ui.set_channels(ModelRc::new(VecModel::from(channel_items(&s, &selected_tag(&ui.get_tags()), &filter))));
        });
    });
}

/// Update the Now / Next lines and progress bars of the rows on screen (no network).
fn refresh_now_next(ui: &MainWindow) {
    let (t, off) = (fmt::now_ts(), fmt::local_offset());
    let m = ui.get_channels();
    CHAN_EVENTS.with(|ev| {
        let ev = ev.borrow();
        for i in 0..m.row_count() {
            if let Some(mut row) = m.row_data(i) {
                if let Some(e) = ev.get(row.uuid.as_str()) { apply_now_next(&mut row, e, t, off); m.set_row_data(i, row); }
            }
        }
    });
}

/// Read the next few programmes of every channel in the background, a few at a time.
fn load_now_next(w: slint::Weak<MainWindow>, s: Arc<Session>) {
    rt().spawn(async move {
        let gate = Arc::new(tokio::sync::Semaphore::new(6));
        for c in s.channels.iter() {
            let (uuid, s, w, gate) = (c.uuid.clone(), s.clone(), w.clone(), gate.clone());
            tokio::spawn(async move {
                let Ok(_p) = gate.acquire().await else { return };
                let Ok(ev) = s.api.epg_channel(&uuid, 4).await else { return };
                ui_do(&w, move |ui| {
                    store_channel_events(&uuid, &ev);
                    let (t, off) = (fmt::now_ts(), fmt::local_offset());
                    let m = ui.get_channels();
                    for i in 0..m.row_count() {
                        if let Some(mut row) = m.row_data(i) {
                            if row.uuid == uuid.as_str() { apply_now_next(&mut row, &ev, t, off); m.set_row_data(i, row); break; }
                        }
                    }
                });
            });
        }
    });
}

/// Open the full guide page of one channel (empty uuid = the first channel).
fn show_channel(w: &slint::Weak<MainWindow>, session: &Shared, uuid: String) {
    let (w, session) = (w.clone(), session.clone());
    ui_do(&w, |ui| ui.set_guide_busy(true));
    rt().spawn(async move {
        let Some(s) = session.lock().await.clone() else { return };
        let Some(ch) = (if uuid.is_empty() { s.channels.first() } else { s.channels.iter().find(|c| c.uuid == uuid) }) else { return };
        let (cu, cn) = (ch.uuid.clone(), ch.name.clone());
        let res = s.api.epg_channel(&cu, 100).await;
        ui_do(&w, move |ui| {
            let (logo, has_logo) = logo_for(&cu);
            ui.set_chan_name(cn.as_str().into());
            ui.set_chan_initials(initials(&cn).into());
            ui.set_chan_logo(logo); ui.set_chan_has_logo(has_logo);
            ui.set_chan_uuid(cu.as_str().into());
            ui.set_guide_busy(false);
            match res {
                Ok(ev) => { store_channel_events(&cu, &ev); ui.set_guide_events(ModelRc::new(VecModel::from(event_items(&ev)))); }
                Err(e) => { ui.set_guide_events(ModelRc::default()); ui.set_toast(e.user_message().into()); }
            }
        });
    });
}

fn nav_items(admin: bool) -> Vec<NavItem> {
    let mut v = vec![(0, "Live"), (1, "Guide"), (2, "Recordings")];
    if admin { v.push((4, "Server")); }
    v.push((5, "Settings"));
    v.into_iter().map(|(id, l)| NavItem { id, label: l.into() }).collect()
}


// ---- Guide timeline -------------------------------------------------------------------------
const PX_PER_MIN: f32 = 3.0;      // 1 hour = 180 px
const WINDOW_MIN: i64 = 12 * 60;  // how far ahead the timeline reaches
static TL_GEN: AtomicU64 = AtomicU64::new(1);

fn tl_events(evs: &[Event], t0: i64, now: i64) -> Vec<TlEvent> {
    let end_w = t0 + WINDOW_MIN * 60;
    evs.iter().filter(|e| e.stop > t0 && e.start < end_w).map(|e| {
        let (s, en) = (e.start.max(t0), e.stop.min(end_w));
        TlEvent {
            id: e.id as i32, title: e.title.clone().into(),
            x: ((s - t0) as f32 / 60.0) * PX_PER_MIN,
            w: (((en - s) as f32 / 60.0) * PX_PER_MIN - 2.0).max(6.0),
            live: e.start <= now && now < e.stop, rec: e.dvr_state.is_some(), rtl: is_rtl(&e.title),
        }
    }).collect()
}

fn set_tl_row(ui: &MainWindow, uuid: &str, evs: &[Event], now: i64) {
    let t0 = TL_T0.with(|c| c.get());
    let m = ui.get_tl_rows();
    for i in 0..m.row_count() {
        if let Some(mut r) = m.row_data(i) {
            if r.uuid == uuid { r.events = ModelRc::new(VecModel::from(tl_events(evs, t0, now))); m.set_row_data(i, r); break; }
        }
    }
}

fn selected_tag(m: &ModelRc<TagItem>) -> String {
    m.iter().find(|t| t.selected).map(|t| t.uuid.to_string()).unwrap_or_default()
}

/// Build the timeline for the channels of one group (empty = all) and fill it channel by channel.
fn load_timeline(w: slint::Weak<MainWindow>, session: Shared, tag: String) {
    rt().spawn(async move {
        let Some(s) = session.lock().await.clone() else { return };
        let gen = TL_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let chans: Vec<String> = s.channels.iter().filter(|c| tag.is_empty() || c.tags.contains(&tag)).map(|c| c.uuid.clone()).collect();
        let now = fmt::now_ts();
        let t0 = ((now - 1800) / 1800) * 1800;
        {
            let s = s.clone(); let chans = chans.clone();
            ui_do(&w, move |ui| {
                TL_T0.with(|c| c.set(t0));
                let off = fmt::local_offset();
                let old: Vec<TlRow> = ui.get_tl_rows().iter().collect(); // keep what is shown until the new data arrives
                let rows: Vec<TlRow> = chans.iter().filter_map(|u| s.channels.iter().find(|c| c.uuid == *u)).map(|c| {
                    let (logo, has_logo) = logo_for(&c.uuid);
                    TlRow { uuid: c.uuid.clone().into(), name: c.name.clone().into(), initials: initials(&c.name).into(), logo, has_logo, events: old.iter().find(|r| r.uuid == c.uuid.as_str()).map(|r| r.events.clone()).unwrap_or_default() }
                }).collect();
                let ticks: Vec<Tick> = (0..=(WINDOW_MIN / 30)).map(|i| {
                    let ts = t0 + i * 1800;
                    Tick { x: (i * 30) as f32 * PX_PER_MIN, label: fmt::hm(ts, off).into() }
                }).collect();
                ui.set_tl_rows(ModelRc::new(VecModel::from(rows)));
                ui.set_tl_ticks(ModelRc::new(VecModel::from(ticks)));
                ui.set_tl_width(WINDOW_MIN as f32 * PX_PER_MIN);
                ui.set_tl_now_x(((now - t0) as f32 / 60.0) * PX_PER_MIN);
                ui.set_tl_busy(!chans.is_empty());
            });
        }
        let remaining = Arc::new(AtomicUsize::new(chans.len()));
        let gate = Arc::new(tokio::sync::Semaphore::new(6));
        for uuid in chans {
            let (s, w, gate, remaining) = (s.clone(), w.clone(), gate.clone(), remaining.clone());
            tokio::spawn(async move {
                let _p = gate.acquire().await;
                let res = s.api.epg_channel(&uuid, 40).await;
                ui_do(&w, move |ui| {
                    if TL_GEN.load(Ordering::SeqCst) != gen { return; } // a newer timeline replaced this one
                    if let Ok(ev) = res { register(&ev); store_channel_events(&uuid, &ev); set_tl_row(ui, &uuid, &ev, now); }
                    if remaining.fetch_sub(1, Ordering::SeqCst) == 1 { ui.set_tl_busy(false); }
                });
            });
        }
    });
}

/// Re-read one channel's programmes (after recording or cancelling) and redraw its timeline row.
fn refresh_channel(w: &slint::Weak<MainWindow>, s: Arc<Session>, uuid: String) {
    let w = w.clone();
    rt().spawn(async move {
        if let Ok(ev) = s.api.epg_channel(&uuid, 40).await {
            let now = fmt::now_ts();
            ui_do(&w, move |ui| { register(&ev); set_tl_row(ui, &uuid, &ev, now); });
        }
    });
}

/// What the Guide tab shows when it is opened or its mode changes.
fn guide_enter(ui: &MainWindow, w: &slint::Weak<MainWindow>, session: &Shared) {
    match ui.get_guide_mode() {
        0 => { if ui.get_tl_rows().row_count() == 0 { load_timeline(w.clone(), session.clone(), selected_tag(&ui.get_tags())); } }
        1 => show_channel(w, session, ui.get_chan_uuid().to_string()),
        _ => {}
    }
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
        let url = match &target { Target::Channel(u) => s.playback_url(u).await, Target::Recording(u) => s.recording_url(u).await };
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();
        #[cfg(target_os = "android")]
        { let d = format!("Sent to the player: {}", Session::describe_url(&url)); ui_do(&w, move |ui| ui.set_play_note(d.into())); }

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
            if g.as_ref().is_none_or(|p| p.gen < gen) { *g = Some(pb); } else { drop(pb); }
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
    ui.set_auth_code(saved.auth_code.clone().into());
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
        move |address, htsp, user, pass, insecure, code| {
            if let Some(ui) = w.upgrade() { ui.set_busy(true); ui.set_error("".into()); }
            let opts = ConnectOptions {
                address: address.to_string(), user: user.to_string(), pass: pass.to_string(),
                htsp_port: htsp.parse::<u16>().ok(), insecure_tls: insecure, auth_code: code.to_string(),
            };
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                let res = Session::connect(opts.clone()).await;
                match res {
                    Ok(s) => {
                        let s = Arc::new(s);
                        let saved = config::load();
                        let all_tags = s.api.tags().await.unwrap_or_default();
                        let known = |u: &str| all_tags.iter().any(|t| t.uuid == u);
                        let live_tag = if known(&saved.last_tag) { saved.last_tag.clone() } else { String::new() };
                        let mk = |sel: &str| -> Vec<TagItem> { all_tags.iter().map(|t| TagItem { uuid: t.uuid.clone().into(), name: t.name.clone().into(), selected: t.uuid == sel }).collect() };
                        let tags = mk(&live_tag);
                        let admin = s.permissions.admin;
                        let info = format!("{}{}", s.api.addr.base_url(), if s.has_htsp() { "" } else { " (HTSP not reachable: live signal only for admins)" });
                        let me = format!("Signed in as {}{}", if opts.user.is_empty() { "anonymous".to_string() } else { opts.user.clone() }, if admin { " · Administrator" } else { "" });
                        config::update(|c| {
                            c.address = opts.address.clone(); c.htsp_port = opts.htsp_port.map(|p| p.to_string()).unwrap_or_default();
                            c.user = opts.user.clone(); c.pass = opts.pass.clone(); c.insecure = opts.insecure_tls;
                            c.auth_code = opts.auth_code.clone(); c.auto_login = true;
                        });
                        let s2 = s.clone();
                        *session.lock().await = Some(s);
                        let _ = slint::invoke_from_event_loop(move || {
                            let Some(ui) = w.upgrade() else { return };
                            LOGOS.with(|l| l.borrow_mut().clear());
                            LIVE_FILTER.with(|f| f.borrow_mut().clear());
                            CHAN_EVENTS.with(|m| m.borrow_mut().clear());
                            let items = channel_items(&s2, &live_tag, "");
                            load_now_next(w.clone(), s2.clone());
                            load_logos(w.clone(), s2);
                            ui.set_busy(false);
                            ui.set_channels(ModelRc::new(VecModel::from(items)));
                            ui.set_tags(ModelRc::new(VecModel::from(tags)));
                            ui.set_tl_rows(ModelRc::default());
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
                            if let Some(ui) = w.upgrade() { ui.set_busy(false); ui.set_error(msg.into()); ui.set_screen(0); }
                        });
                    }
                }
            });
        }
    });

    ui.on_tab_changed({
        let (w, session) = (ui.as_weak(), session.clone());
        move |t| match t {
            1 => { if let Some(ui) = w.upgrade() { guide_enter(&ui, &w, &session); } }
            2 => { if let Some(ui) = w.upgrade() { load_recordings(&w, &session, ui.get_rec_kind()); } }
            4 => { if let Some(ui) = w.upgrade() { ui.invoke_refresh_server(); } }
            _ => {}
        }
    });

    ui.on_select_tag({
        let (w, session) = (ui.as_weak(), session.clone());
        move |uuid| {
            let Some(ui) = w.upgrade() else { return };
            // tapping the active chip clears the filter; Live and Guide share this one choice
            let cur: Vec<TagItem> = ui.get_tags().iter().collect();
            let was = cur.iter().any(|t| t.uuid == uuid.as_str() && t.selected);
            let sel = if was { String::new() } else { uuid.to_string() };
            let next: Vec<TagItem> = cur.into_iter().map(|mut t| { t.selected = t.uuid == sel.as_str(); t }).collect();
            ui.set_tags(ModelRc::new(VecModel::from(next)));
            { let sel = sel.clone(); config::update(|c| c.last_tag = sel); }
            rebuild_live(&w, &session);
            ui.set_tl_rows(ModelRc::default());
            if ui.get_tab() == 1 && ui.get_guide_mode() == 0 { load_timeline(w.clone(), session.clone(), sel); }
        }
    });

    ui.on_filter_channels({
        let (w, session) = (ui.as_weak(), session.clone());
        move |text| { LIVE_FILTER.with(|f| *f.borrow_mut() = text.to_string()); rebuild_live(&w, &session); }
    });

    ui.on_picker_filter_changed({
        let (w, session) = (ui.as_weak(), session.clone());
        move |text| {
            let (w, session, text) = (w.clone(), session.clone(), text.to_string());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                ui_do(&w, move |ui| ui.set_picker(ModelRc::new(VecModel::from(channel_items(&s, "", &text)))));
            });
        }
    });

    ui.on_open_channel({
        let (w, session) = (ui.as_weak(), session.clone());
        move |uuid| {
            if let Some(ui) = w.upgrade() { ui.set_tab(1); ui.set_guide_mode(1); ui.set_detail_open(false); }
            show_channel(&w, &session, uuid.to_string());
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
                    if ui.get_tab() == 1 && ui.get_guide_mode() == 1 { ui.invoke_open_channel(ui.get_chan_uuid()); }
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

    ui.on_guide_mode_changed({
        let (w, session) = (ui.as_weak(), session.clone());
        move |_| { if let Some(ui) = w.upgrade() { guide_enter(&ui, &w, &session); } }
    });

    ui.on_event_clicked({
        let (w, session) = (ui.as_weak(), session.clone());
        move |id| {
            let Some(ev) = EVENTS.with(|m| m.borrow().get(&id).cloned()) else { return };
            let Some(ui) = w.upgrade() else { return };
            ui.set_detail(event_items(std::slice::from_ref(&ev)).remove(0));
            ui.set_detail_open(true);
            if let Some(src) = ev.image.clone() {
                let (w, session) = (w.clone(), session.clone());
                rt().spawn(async move {
                    let Some(s) = session.lock().await.clone() else { return };
                    let Ok(bytes) = s.api.image(&src).await else { return };
                    let Some((wd, ht, px)) = tokio::task::spawn_blocking(move || decode_rgba(&bytes)).await.ok().flatten() else { return };
                    ui_do(&w, move |ui| {
                        if ui.get_detail().id != id { return; } // the person already opened something else
                        let mut d = ui.get_detail();
                        d.poster = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&px, wd, ht));
                        d.has_poster = true;
                        ui.set_detail(d);
                    });
                });
            }
        }
    });

    ui.on_detail_record({
        let (w, session) = (ui.as_weak(), session.clone());
        move |id, channel| {
            let (w, session, channel) = (w.clone(), session.clone(), channel.to_string());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let msg = match s.api.record_event(id as u32).await { Ok(()) => "Scheduled".to_string(), Err(e) => e.user_message() };
                refresh_channel(&w, s, channel);
                ui_do(&w, move |ui| { ui.set_toast(msg.into()); ui.set_detail_open(false); if ui.get_tab() == 1 && ui.get_guide_mode() == 1 { ui.invoke_open_channel(ui.get_chan_uuid()); } });
            });
        }
    });

    ui.on_detail_cancel({
        let (w, session) = (ui.as_weak(), session.clone());
        move |dvr, channel| {
            let (w, session, dvr, channel) = (w.clone(), session.clone(), dvr.to_string(), channel.to_string());
            rt().spawn(async move {
                let Some(s) = session.lock().await.clone() else { return };
                let msg = match s.api.cancel_recording(&dvr).await { Ok(()) => "Recording cancelled".to_string(), Err(e) => e.user_message() };
                refresh_channel(&w, s, channel);
                ui_do(&w, move |ui| { ui.set_toast(msg.into()); ui.set_detail_open(false); if ui.get_tab() == 1 && ui.get_guide_mode() == 1 { ui.invoke_open_channel(ui.get_chan_uuid()); } });
            });
        }
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

    ui.on_edit_connection({
        let (w, session, slot) = (ui.as_weak(), session.clone(), slot.clone());
        move || {
            drop(slot.lock().unwrap().take());
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                *session.lock().await = None;
                ui_do(&w, |ui| { ui.set_now_playing("".into()); ui.set_has_signal(false); ui.set_tab(0); ui.set_screen(0); });
            });
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
            config::update(|c| { c.pass.clear(); c.auth_code.clear(); c.auto_login = false; });
            let (w, session) = (w.clone(), session.clone());
            rt().spawn(async move {
                *session.lock().await = None;
                ui_do(&w, |ui| {
                    ui.set_password("".into()); ui.set_auth_code("".into()); ui.set_now_playing("".into()); ui.set_has_signal(false);
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

    // Returning user: go straight in. If it fails the connect screen shows the reason.
    if saved.auto_login && !saved.address.is_empty() && std::env::var("TVHME_AUTOCONNECT").is_err() {
        ui.set_screen(3);
        ui.invoke_connect(saved.address.clone().into(), saved.htsp_port.clone().into(), saved.user.clone().into(),
                          saved.pass.clone().into(), saved.insecure, saved.auth_code.clone().into());
    }

    // Developer aid: TVHME_TAB=n opens a tab a moment after start
    if let Ok(scr) = std::env::var("TVHME_TAB") {
        if let Ok(n) = scr.parse::<i32>() {
            let w = ui.as_weak();
            rt().spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                ui_do(&w, move |ui| { ui.set_tab(n); ui.invoke_tab_changed(n); });
            });
        }
    }

    // Developer aid: TVHME_AUTOCONNECT=address,htsp_port,user,pass[,play_channel_uuid]
    if let Ok(v) = std::env::var("TVHME_AUTOCONNECT") {
        let p: Vec<String> = v.split(',').map(str::to_string).collect();
        if p.len() >= 4 {
            ui.invoke_connect(p[0].clone().into(), p[1].clone().into(), p[2].clone().into(), p[3].clone().into(), false, "".into());
            if let Some(ch) = p.get(4).cloned() {
                let w = ui.as_weak();
                rt().spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    let _ = slint::invoke_from_event_loop(move || { if let Some(ui) = w.upgrade() { ui.invoke_play(ch.into()); } });
                });
            }
        }
    }
    // Keep the guide alive: move the "now" line every 30 s and re-read the programmes every 3 minutes.
    let ticker = slint::Timer::default();
    ticker.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(30), {
        let (w, session) = (ui.as_weak(), session.clone());
        let mut count = 0u32;
        move || {
            count += 1;
            let Some(ui) = w.upgrade() else { return };
            if ui.get_screen() != 1 { return; }
            let t0 = TL_T0.with(|c| c.get());
            refresh_now_next(&ui);
            if t0 > 0 { ui.set_tl_now_x(((fmt::now_ts() - t0) as f32 / 60.0) * PX_PER_MIN); }
            if count.is_multiple_of(6) {
                if let Some(s) = session.try_lock().ok().and_then(|g| g.clone()) { load_now_next(w.clone(), s); }
            }
            if count.is_multiple_of(6) && ui.get_tl_rows().row_count() > 0 {
                load_timeline(w.clone(), session.clone(), selected_tag(&ui.get_tags()));
            }
        }
    });
    let result = ui.run();
    drop(ticker);
    result
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
    fn ev(id: u32, start: i64, stop: i64, title: &str) -> Event {
        Event { id, channel_uuid: "c".into(), channel_name: "C".into(), start, stop, title: title.into(), ..Default::default() }
    }
    #[test]
    fn rtl_detection_uses_first_letter() {
        assert!(is_rtl("نشرة الأخبار"));
        assert!(is_rtl("12:00 نشرة"));
        assert!(is_rtl("שלום"));
        assert!(!is_rtl("News"));
        assert!(!is_rtl("News نشرة"));
        assert!(!is_rtl(""));
    }
    #[test]
    fn now_and_next_from_a_channels_events() {
        let e = vec![ev(1, 100, 200, "A"), ev(2, 200, 300, "B"), ev(3, 300, 400, "C")];
        let (n, x) = now_next(&e, 250);
        assert_eq!((n.unwrap().title.as_str(), x.unwrap().title.as_str()), ("B", "C"));
        let (n, x) = now_next(&e, 50); // nothing on air yet
        assert!(n.is_none() && x.unwrap().title == "A");
        let (n, x) = now_next(&e, 399);
        assert!(n.is_some() && x.is_none());
    }
    #[test]
    fn initials_for_logoless_channels() {
        assert_eq!(initials("BBC One"), "BO");
        assert_eq!(initials("nile"), "NI");
        assert_eq!(initials(""), "");
    }
    #[test]
    fn theme_indices_map_correctly() {
        assert_eq!(choice_from_indices(2, 3, 0), ThemeChoice::default());
        assert_eq!(choice_from_indices(3, 12, 1), ThemeChoice::Catppuccin { flavor: Flavor::Mocha, accent: Accent::Blue, amoled: true });
        assert_eq!(choice_from_indices(0, 0, 2), ThemeChoice::System { accent: Accent::Rosewater });
        assert_eq!(choice_from_indices(99, -4, 0), ThemeChoice::Catppuccin { flavor: Flavor::Mocha, accent: Accent::Rosewater, amoled: false });
    }
}
