use tvhme_core::{http, permissions::Section, session::{Session, SessionError}, stream::Server};
use std::time::Duration;

fn server(m: &tvhme_mock::MockServer) -> Server {
    Server::new("127.0.0.1", m.htsp.port(), m.http.port())
}

#[tokio::test]
async fn admin_gets_management_and_channels() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(server(&m), "admin", "admin").await.unwrap();
    assert!(s.permissions.admin);
    assert!(s.permissions.sections().contains(&Section::Settings));
    let names: Vec<_> = s.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Mock One", "Mock Two", "Mock Three"]);
}

#[tokio::test]
async fn normal_user_has_no_management() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(server(&m), "user", "user").await.unwrap();
    assert!(!s.permissions.admin);
    assert!(!s.permissions.sections().contains(&Section::Settings));
    assert_eq!(s.channels.len(), 3);
}

#[tokio::test]
async fn wrong_password_is_rejected() {
    let m = tvhme_mock::spawn().await;
    let e = Session::connect(server(&m), "user", "nope").await.err().unwrap();
    assert!(matches!(e, SessionError::Probe(_)), "{e:?}");
}

#[tokio::test]
async fn signal_status_arrives_as_percent() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(server(&m), "user", "user").await.unwrap();
    let mut rx = s.watch_signal(2, 9).await.unwrap();
    let st = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    assert_eq!(st.subscription_id, 9);
    assert!((st.snr_pct.unwrap() - 75.0).abs() < 0.1);
    assert!((st.signal_pct.unwrap() - 50.0).abs() < 0.1);
    s.stop_signal(9).await.unwrap();
}

#[tokio::test]
async fn stream_url_is_playable_over_http() {
    let m = tvhme_mock::spawn().await;
    let s = Session::connect(server(&m), "user", "user").await.unwrap();
    let url = s.stream_url(1);
    assert!(url.contains("/stream/channelid/1"));
    let (code, body) = http::get("127.0.0.1", m.http.port(), "/stream/channelid/1", "user", "user").await.unwrap();
    assert_eq!((code, body.as_slice()), (200, &b"MOCK-TS-DATA"[..]));
    let (code, _) = http::get("127.0.0.1", m.http.port(), "/stream/channelid/1", "", "").await.unwrap();
    assert_eq!(code, 401);
}
