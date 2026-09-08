use crate::config::{
    paths::{AppConfigPaths, native_config_dir},
    profile::EnvironmentKind,
};
use std::{io, path::PathBuf};

pub fn install_managed_hooks(config_paths: &AppConfigPaths) -> io::Result<()> {
    if let Some(storage) = crate::config::storage::environment_storage() {
        return storage.install_agent_hooks();
    }
    if cfg!(test) || config_paths.environment() == EnvironmentKind::Test {
        return Ok(());
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "user home is unavailable"))?;
    let config = match config_paths.environment() {
        EnvironmentKind::Production => config_paths.config_dir().to_path_buf(),
        EnvironmentKind::Development => native_config_dir(),
        EnvironmentKind::Test => unreachable!(),
    };
    yttt_agent_providers::installer::install_managed_hooks_at(&config, &home)
}
