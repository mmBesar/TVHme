use std::sync::Arc;
use std::time::Duration;
use tvhme_core::api::ApiError;
use tvhme_core::permissions::{ProbeError, Section};
use tvhme_core::session::{ConnectOptions, Session, SessionError, SignalSource};

fn opts(addr: String, user: &str, pass: &str, htsp: Option<u16>) -> ConnectOptions {
    ConnectOptions { address: addr, user: user.into(), pass: pass.into(), htsp_port: htsp, insecure_tls: false }
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
    assert!(url.starts_with("http://user:user@127.0.0.1:") && url.contains("/stream/channel/uuid-one"), "{url}");
}
