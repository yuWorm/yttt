//! The core link of the yttt mobile Clients: transport, pairing, the
//! `yttt-client-core` session and the VT repaint encoder. Everything else
//! (UI, storage, notifications, file editing) belongs to the native apps.

uniffi::setup_scaffolding!();

mod ffi;
pub mod session;
pub mod vt;

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
