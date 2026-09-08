#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_types)]
#![allow(clippy::too_many_arguments)]

pub use yttt_core::commands;
pub mod config;
pub mod desktop_shell;
pub mod desktop_tray;
pub use yttt_core::model;
pub mod host_launcher;
pub mod host_runtime;
pub mod host_storage;
pub mod login_startup;
pub mod palette;
pub mod remote_host;
pub mod remote_launch;
pub mod runtime;
pub mod session_coordinator;
pub mod ui;
