//! The core link of the yttt mobile Clients: transport, pairing, the
//! `yttt-client-core` session and the VT repaint encoder. Everything else
//! (UI, storage, notifications, file editing) belongs to the native apps.

uniffi::setup_scaffolding!();

pub mod account;
pub mod appearance;
mod ffi;
pub mod files;
pub mod keys;
pub mod logging;
pub mod overview;
pub mod session;
pub mod vt;

/// Starts the connection log in `dir`, the app's own directory; call before
/// making the core. Once per process.
#[uniffi::export]
pub fn start_connection_log(dir: String) {
    logging::start(std::path::Path::new(&dir));
}

/// The end of the connection log in `dir`, for the settings page.
#[uniffi::export]
pub fn read_connection_log(dir: String) -> String {
    logging::read(std::path::Path::new(&dir))
}

/// The version of this core, so an app can show what it was built with.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn core_version_is_the_workspace_version() {
        assert_eq!(super::core_version(), env!("CARGO_PKG_VERSION"));
    }
}
