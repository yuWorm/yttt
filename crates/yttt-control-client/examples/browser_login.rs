//! Signs in to a relay account the way the app does: prints the console's
//! authorize page to open, waits on a loopback port for the answer, and
//! shows the account. For testing the browser sign-in without the desktop.
//!
//!     cargo run -p yttt-control-client --example browser_login -- <control url>
//!
//! The session is printed only as its account; nothing is saved.

use yttt_control_client::ControlClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let control_url = std::env::args()
        .nth(1)
        .ok_or("usage: browser_login <control url>")?;
    let client = ControlClient::new(&control_url).map_err(|error| error.to_string())?;
    let login = client
        .begin_browser_login("browser_login example")
        .await
        .map_err(|error| error.to_string())?;
    println!("open: {}", login.authorize_url());
    let session = login.finish().await.map_err(|error| error.to_string())?;
    let account = session.account().await.map_err(|error| error.to_string())?;
    println!(
        "signed in as {} (account {}, plan {})",
        account.email, account.account_id, account.plan
    );
    Ok(())
}
