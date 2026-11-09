#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_types)]
#![allow(clippy::too_many_arguments)]

pub use yttt_core::commands;
pub mod agent_control_client;
pub mod agent_mcp;
pub mod cli;
#[cfg(target_os = "macos")]
mod cli_registration;
pub mod config;
mod connection_log;
pub mod desktop_shell;
pub mod desktop_tray;
mod link_watch;
pub use yttt_core::model;
pub mod host_launcher;
pub mod host_runtime;
pub mod host_storage;
pub mod login_startup;
mod paired_hosts;
pub mod palette;
mod relay_account;
pub mod remote_host;
pub mod remote_launch;
mod remote_restore;
pub mod runtime;
pub mod session_coordinator;
pub mod ui;
