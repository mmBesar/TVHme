//! Standalone mock: `tvhme-mock [webroot]`. Prints ports; users admin/admin and user/user.
#[tokio::main]
async fn main() {
    let root = std::env::args().nth(1).unwrap_or_default();
    let m = tvhme_mock::spawn_with(&root, true).await;
    println!("htsp={} http={}", m.htsp.port(), m.http.port());
    println!("users: admin/admin, user/user");
    tokio::signal::ctrl_c().await.ok();
}
