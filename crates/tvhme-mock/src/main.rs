//! Run the mock server standalone: prints its ports and serves until killed.
#[tokio::main]
async fn main() {
    let m = tvhme_mock::spawn().await;
    println!("htsp={} http={}", m.htsp.port(), m.http.port());
    println!("users: admin/admin, user/user");
    tokio::signal::ctrl_c().await.ok();
}
