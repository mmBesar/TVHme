use std::sync::Arc;
use std::time::Duration;
use tvhme_core::api::ApiError;
use tvhme_core::permissions::{ProbeError, Section};
use tvhme_core::session::{ConnectOptions, Session, SessionError, SignalSource};

fn opts(addr: String, user: &str, pass: &str, htsp: Option<u16>) -> ConnectOptions {
    ConnectOptions { address: addr, user: user.into(), pass: pass.into(), htsp_port: htsp, insecure_tls: false, auth_code: String::new() }
}
fn addr(m: &tvhme_mock::MockServer) -> String { format!("127.0.0.1:{}", m.http.port()) }

#[tokio::test]
async fn admin_gets_management_channels_and_now_playing() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "admin", "admin", Some(m.htsp.port()))).await.unwrap();
    assert!(s.permissions.admin);
    assert!(s.permissions.sections().contains(&Section::Settings));
    let names: Vec<_> = s.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Mock One", "Mock Two", "Mock Three"]);
    assert_eq!(s.channels[1].now.as_ref().unwrap().title, "Evening News");
    assert!(s.channels[0].now.is_none());
}

#[tokio::test]
async fn normal_user_has_no_management() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap();
    assert!(!s.permissions.admin);
    assert!(!s.permissions.sections().contains(&Section::Settings));
    assert_eq!(s.channels.len(), 3);
}

#[tokio::test]
async fn wrong_password_says_so() {
    let m = tvhme_mock::spawn().await;
    let e = Session::connect(opts(addr(&m), "user", "nope", None)).await.err().unwrap();
    assert!(matches!(e, SessionError::Probe(ProbeError::BadCredentials)), "{e:?}");
}

#[tokio::test]
async fn http_scheme_and_webroot_are_accepted() {
    let m = tvhme_mock::spawn_with("/tvh", false).await;
    let a = format!("http://127.0.0.1:{}/tvh/", m.http.port());
    let s = Session::connect(opts(a, "user", "user", None)).await.unwrap();
    assert_eq!(s.channels.len(), 3);
    assert!(s.stream_url("uuid-one").contains("/tvh/stream/channel/uuid-one"));
}

#[tokio::test]
async fn wrong_webroot_is_reported_as_not_tvheadend() {
    let m = tvhme_mock::spawn_with("/tvh", false).await;
    let e = Session::connect(opts(addr(&m), "user", "user", None)).await.err().unwrap();
    assert!(matches!(e, SessionError::Probe(ProbeError::NotTvheadend(404))), "{e:?}");
}

#[tokio::test]
async fn unknown_host_gives_a_dns_message_and_url_input_is_not_misread() {
    // The same text a user might paste from the browser bar must parse; a bad host must say "could not resolve".
    let e = Session::connect(opts("https://no-such-host.invalid".into(), "a", "b", None)).await.err().unwrap();
    let msg = e.to_string();
    assert!(msg.contains("could not resolve") || matches!(e, SessionError::Probe(ProbeError::Api(ApiError::Dns(_)))), "{msg}");
}

#[tokio::test]
async fn login_works_without_htsp_port_and_admin_gets_signal_from_status_inputs() {
    let m = tvhme_mock::spawn_with("", false).await; // HTSP closed, like behind a reverse proxy
    let s = Arc::new(Session::connect(opts(addr(&m), "admin", "admin", Some(m.htsp.port()))).await.unwrap());
    assert!(!s.has_htsp());
    let mut w = s.watch_signal("uuid-two", 9).await;
    assert_eq!(w.source, SignalSource::StatusInputs);
    tokio::time::timeout(Duration::from_secs(3), w.rx.changed()).await.unwrap().unwrap();
    let st = w.rx.borrow().clone().unwrap();
    assert!((st.signal_pct.unwrap() - 75.0).abs() < 0.1);
    assert!((st.snr_db.unwrap() - 18.5).abs() < 0.01);
}

#[tokio::test]
async fn normal_user_without_htsp_gets_no_signal_but_still_plays() {
    let m = tvhme_mock::spawn_with("", false).await;
    let s = Arc::new(Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap());
    assert_eq!(s.watch_signal("uuid-one", 1).await.source, SignalSource::Unavailable);
    assert!(s.stream_url("uuid-one").starts_with("http://user:user@127.0.0.1:"));
}

#[tokio::test]
async fn htsp_signal_for_normal_user() {
    let m = tvhme_mock::spawn().await;
    let s = Arc::new(Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap());
    assert!(s.has_htsp());
    let mut w = s.watch_signal("uuid-two", 9).await;
    assert_eq!(w.source, SignalSource::Htsp);
    tokio::time::timeout(Duration::from_secs(3), w.rx.changed()).await.unwrap().unwrap();
    let st = w.rx.borrow().clone().unwrap();
    assert!((st.snr_pct.unwrap() - 75.0).abs() < 0.1);
    assert!((st.signal_pct.unwrap() - 50.0).abs() < 0.1);
}

#[tokio::test]
async fn playback_url_uses_a_ticket_not_the_password_and_it_works() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap();
    let url = s.playback_url("uuid-two").await;
    assert!(url.contains("ticket=TICKET123") && url.contains("/stream/channelid/101"), "{url}");
    assert!(!url.contains("user:user@"), "{url}");
    // No credentials at all: the ticket alone must be accepted.
    let r = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(&r.bytes().await.unwrap()[..], b"MOCK-TS-DATA");
}

#[tokio::test]
async fn playback_url_falls_back_to_credentials_without_htsp() {
    let m = tvhme_mock::spawn_with("", false).await;
    let s = Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap();
    let url = s.playback_url("uuid-one").await;
    // no HTSP: a ticket over HTTP, as the web interface does
    assert!(url.contains("/stream/channel/uuid-one?ticket=TICKET123&profile=pass") && !url.contains('@'), "{url}");
    assert!(s.stream_url("uuid-one").starts_with("http://user:user@127.0.0.1:")); // the login-in-address fallback
}

#[tokio::test]
async fn admin_overview_has_tuners_streams_and_clients_but_normal_user_is_refused() {
    let m = tvhme_mock::spawn().await;
    let a = Session::connect(opts(addr(&m), "admin", "admin", None)).await.unwrap();
    let o = a.server_overview().await.unwrap();
    assert_eq!(o.info.sw_version, "4.3-mock");
    assert_eq!(o.inputs.len(), 1);
    assert_eq!(o.subscriptions[0].service, "Mock Two");
    assert_eq!(o.connections[0].peer, "10.0.0.7");
    let u = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    assert!(u.api.connections().await.is_err());
}

use tvhme_core::api::RecordingKind;

#[tokio::test]
async fn tags_come_ordered_and_channels_know_their_tags() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let tags = s.api.tags().await.unwrap();
    assert_eq!(tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["News", "Movies"]);
    let news: Vec<_> = s.channels.iter().filter(|c| c.tags.contains(&"tag-news".to_string())).map(|c| c.name.as_str()).collect();
    assert_eq!(news, ["Mock One", "Mock Two"]);
}

#[tokio::test]
async fn epg_per_channel_and_search() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let two = s.api.epg_channel("uuid-two", 20).await.unwrap();
    assert_eq!(two.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Evening News", "Late Movie"]);
    assert!(two[0].start < two[0].stop && two[0].channel_name == "Mock Two");
    assert_eq!(s.api.epg_search("movie", false, 20).await.unwrap().len(), 1);
    // "delta" only appears in a summary: found by full-text search, not by title search
    assert!(s.api.epg_search("delta", false, 20).await.unwrap().is_empty());
    assert_eq!(s.api.epg_search("delta", true, 20).await.unwrap()[0].title, "Nature Documentary");
}

#[tokio::test]
async fn schedule_manage_and_remove_recordings() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    assert!(s.api.recordings(RecordingKind::Upcoming).await.unwrap().is_empty());
    s.api.record_event(2).await.unwrap();
    let up = s.api.recordings(RecordingKind::Upcoming).await.unwrap();
    assert_eq!((up.len(), up[0].title.as_str(), up[0].channel_name.as_str()), (1, "Late Movie", "Mock Two"));
    // the guide now shows the event as scheduled
    let ev = s.api.epg_channel("uuid-two", 20).await.unwrap();
    assert_eq!(ev[1].dvr_state.as_deref(), Some("scheduled"));
    assert!(ev[1].dvr_uuid.is_some() && ev[0].dvr_uuid.is_none());
    // one-off recording of a channel right now
    s.api.record_now("uuid-one", "Manual", 30).await.unwrap();
    assert_eq!(s.api.recordings(RecordingKind::Upcoming).await.unwrap().len(), 2);
    // stop moves one to finished, cancel deletes the other, remove deletes a finished one
    let stop_id = up[0].uuid.clone();
    s.api.stop_recording(&stop_id).await.unwrap();
    assert_eq!(s.api.recordings(RecordingKind::Finished).await.unwrap().len(), 1);
    s.api.remove_recording(&stop_id).await.unwrap();
    let rest = s.api.recordings(RecordingKind::Upcoming).await.unwrap();
    assert_eq!(rest.len(), 1);
    s.api.cancel_recording(&rest[0].uuid).await.unwrap();
    assert!(s.api.recordings(RecordingKind::Upcoming).await.unwrap().is_empty());
    // unknown event is refused by the server
    assert!(s.api.record_event(999).await.is_err());
}

#[tokio::test]
async fn account_without_recorder_rights_can_watch_and_search_but_the_server_refuses_recording() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "viewer", "viewer", None)).await.unwrap();
    assert_eq!(s.channels.len(), 3);
    assert_eq!(s.api.epg_search("news", false, 10).await.unwrap().len(), 1);
    assert_eq!(s.api.recordings(RecordingKind::Upcoming).await.unwrap_err(), ApiError::Status(403));
    assert_eq!(s.api.record_event(2).await.unwrap_err(), ApiError::Status(403));
}

#[tokio::test]
async fn recording_file_plays_for_recorders_and_is_refused_for_others() {
    let m = tvhme_mock::spawn().await;
    let ok = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let url = ok.recording_url("rec-1").await;
    assert!(url.contains("/dvrfile/rec-1?ticket=TICKET123") && !url.contains('@'), "{url}"); // ticket, no password
    let r = reqwest::Client::new().get(url).send().await.unwrap(); // no login at all
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(&r.bytes().await.unwrap()[..], b"MOCK-DVR-DATA");
    // viewer: the server still hands out a ticket in the mock, but tickets are only a way to carry the login
    let no = Session::connect(opts(addr(&m), "viewer", "viewer", None)).await.unwrap();
    let r = reqwest::Client::new().get(no.api.dvr_url("rec-1")).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 403);
}

#[tokio::test]
async fn authentication_code_replaces_the_password_for_streams_and_the_api() {
    let m = tvhme_mock::spawn().await;
    let mut o = opts(addr(&m), "", "", Some(m.htsp.port()));
    o.auth_code = tvhme_mock::AUTH_CODE.into();
    let s = Session::connect(o).await.unwrap();
    assert_eq!(s.channels.len(), 3);
    let url = s.playback_url("uuid-two").await;
    assert!(url.contains("/stream/channel/uuid-two?auth=MOCKCODE") && !url.contains('@'), "{url}");
    let r = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let mut bad = opts(addr(&m), "", "", None);
    bad.auth_code = "WRONG".into();
    assert!(Session::connect(bad).await.is_err());
}

#[tokio::test]
async fn channel_logos_are_listed_and_downloadable_with_the_account_login() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let with: Vec<_> = s.channels.iter().filter(|c| c.icon.is_some()).collect();
    assert_eq!(with.len(), 1);
    assert_eq!(with[0].uuid, "uuid-two");
    let png = s.api.image(with[0].icon.as_ref().unwrap()).await.unwrap();
    assert_eq!(png, tvhme_mock::LOGO_PNG);
    assert!(s.api.image("file:///etc/passwd").await.is_err());
    let bad = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    assert!(bad.api.image("imagecache/999").await.is_err());
}

#[tokio::test]
async fn events_carry_description_and_picture() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let ev = s.api.epg_channel("uuid-two", 10).await.unwrap();
    let news = ev.iter().find(|e| e.title == "Evening News").unwrap();
    assert_eq!(news.description, "Longer text about Evening News");
    assert_eq!(news.image.as_deref(), Some("imagecache/1"));
    assert!(ev.iter().find(|e| e.title == "Late Movie").unwrap().image.is_none());
    assert_eq!(s.api.image(news.image.as_ref().unwrap()).await.unwrap(), tvhme_mock::LOGO_PNG);
}

#[tokio::test]
async fn events_carry_episode_facts_and_credits() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let news = s.api.epg_search("news", false, 10).await.unwrap().into_iter().find(|e| e.title == "Evening News").unwrap();
    assert_eq!(news.subtitle, "Pilot");
    assert_eq!(news.episode, "S02E05");
    assert_eq!(news.facts, "2019 · News, Weather · ★ 7.5 · 12+");
    assert_eq!(news.credits, "Director: Jane Roe  ·  Cast: A B");
    let plain = s.api.epg_search("movie", false, 10).await.unwrap();
    assert!(plain[0].episode.is_empty() && plain[0].facts.is_empty() && plain[0].credits.is_empty());
}

#[tokio::test]
async fn admin_can_switch_channels_off_and_on() {
    let m = tvhme_mock::spawn().await;
    let a = Session::connect(opts(addr(&m), "admin", "admin", None)).await.unwrap();
    let all = a.api.managed_channels().await.unwrap();
    assert_eq!(all.len(), 4);
    assert!(!all.iter().find(|c| c.name == "Mock Hidden").unwrap().enabled);
    a.api.set_channel_enabled("uuid-hidden", true).await.unwrap();
    a.api.set_channel_enabled("uuid-one", false).await.unwrap();
    let u = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    let names: Vec<_> = u.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Mock Two", "Mock Three", "Mock Hidden"]);
    // a normal account may not change channels
    assert!(matches!(u.api.set_channel_enabled("uuid-two", false).await, Err(ApiError::Status(403))));
}

#[tokio::test]
async fn dropping_the_signal_watch_off_the_runtime_does_not_panic() {
    // the app drops it on the UI thread (Stop button), which has no Tokio runtime
    let m = tvhme_mock::spawn().await;
    let s = Arc::new(Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap());
    let sw = s.watch_signal("uuid-two", 1).await;
    assert_eq!(sw.source, SignalSource::Htsp);
    std::thread::spawn(move || drop(sw)).join().expect("drop must not panic");
}

#[tokio::test]
async fn a_server_that_redirects_to_another_port_is_followed_before_login() {
    // stands in for "http://tv.example.com" answering with a redirect to https: same host, other port
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let m = tvhme_mock::spawn().await;
    let real = m.http.port();
    let front = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let front_port = front.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = front.accept().await else { return };
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let r = format!("HTTP/1.1 308 Permanent Redirect\r\nLocation: http://127.0.0.1:{real}/api/serverinfo\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = s.write_all(r.as_bytes()).await;
        }
    });
    let s = Session::connect(opts(format!("127.0.0.1:{front_port}"), "user", "user", None)).await.unwrap();
    assert_eq!(s.api.addr.port, real);
    assert_eq!(s.channels.len(), 3);
}

#[tokio::test]
async fn stream_profile_is_listed_and_goes_into_the_play_address() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(opts(addr(&m), "user", "user", Some(m.htsp.port()))).await.unwrap();
    assert_eq!(s.api.stream_profiles().await, ["pass", "matroska", "webtv-h264-aac-matroska"]);
    // nothing chosen: what we always did
    assert!(s.playback_url("uuid-two").await.contains("profile=pass"));
    s.api.set_profile("webtv-h264-aac-matroska");
    assert!(s.playback_url("uuid-two").await.contains("profile=webtv-h264-aac-matroska"));
    s.api.set_profile("");
    assert!(s.playback_url("uuid-two").await.contains("profile=pass"));
    // without HTSP the profile also reaches the ticket address
    let h = Session::connect(opts(addr(&m), "user", "user", None)).await.unwrap();
    h.api.set_profile("matroska");
    assert!(h.playback_url("uuid-two").await.contains("profile=matroska"));
}
