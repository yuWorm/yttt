use serde::Deserialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use yttt_protocol::remote_access::RemoteAccessSettings;

/// This directory is deliberately outside the Config RPC allowlist.
pub struct DeviceSettingsStore {
    root: PathBuf,
    settings: RemoteAccessSettings,
}

impl DeviceSettingsStore {
    pub fn load(state_root: &Path, config_root: &Path) -> io::Result<Self> {
        let root = state_root.join("remote-access");
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "device settings root is not a real directory",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt as _;
                    builder.mode(0o700);
                }
                builder.create(&root)?;
            }
            Err(error) => return Err(error),
        }
        crate::secure_runtime_root(&root).map_err(io::Error::other)?;
        let path = root.join("settings.json");
        let settings = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                crate::validate_user_only_file(&path).map_err(io::Error::other)?;
                serde_json::from_slice(&crate::workspace::read_bounded_file(&path, 64 * 1024)?)
                    .map_err(io::Error::other)?
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "device settings must be a regular private file",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let settings = RemoteAccessSettings {
                    login_startup_consent_granted: legacy_startup_consent(config_root)?,
                    ..RemoteAccessSettings::default()
                };
                persist(&path, &settings)?;
                settings
            }
            Err(error) => return Err(error),
        };
        Ok(Self { root, settings })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn settings(&self) -> &RemoteAccessSettings {
        &self.settings
    }

    /// Do not publish confirmed state until both the file and its directory are durable.
    pub fn save(&mut self, settings: RemoteAccessSettings) -> io::Result<()> {
        persist(&self.root.join("settings.json"), &settings)?;
        self.settings = settings;
        Ok(())
    }

    pub fn set_startup_consent(&mut self, granted: bool) -> io::Result<RemoteAccessSettings> {
        let mut settings = self.settings.clone();
        settings.login_startup_consent_granted = granted;
        self.save(settings)?;
        Ok(self.settings.clone())
    }
}

fn persist(path: &Path, settings: &RemoteAccessSettings) -> io::Result<()> {
    let bytes = serde_json::to_vec(settings).map_err(io::Error::other)?;
    crate::workspace::atomic_write(path, &bytes, uuid::Uuid::new_v4().as_u128() as u64)
}

fn legacy_startup_consent(config_root: &Path) -> io::Result<bool> {
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct LegacySettings {
        remote_access: LegacyAccess,
    }
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct LegacyAccess {
        login_startup_consent_granted: bool,
    }
    let bytes = match crate::workspace::read_bounded_file(
        &config_root.join("settings.toml"),
        1024 * 1024,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let source = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let legacy: LegacySettings = toml::from_str(source).map_err(io::Error::other)?;
    Ok(legacy.remote_access.login_startup_consent_granted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_consent_migrates_without_enabling_network_or_rewriting_shared_settings() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        let original = "[remote_access]\nlogin_startup_consent_granted = true\nenabled = true\n";
        fs::write(config.join("settings.toml"), original).unwrap();
        let store = DeviceSettingsStore::load(root.path(), &config).unwrap();
        assert!(store.settings().login_startup_consent_granted);
        assert!(!store.settings().enabled);
        assert_eq!(
            fs::read_to_string(config.join("settings.toml")).unwrap(),
            original
        );
        // Legacy content is no longer an authority after a successful migration.
        fs::write(config.join("settings.toml"), "").unwrap();
        let restored = DeviceSettingsStore::load(root.path(), &config).unwrap();
        assert!(restored.settings().login_startup_consent_granted);
        assert!(!restored.settings().enabled);
    }

    #[test]
    fn failed_preference_save_does_not_publish_an_unconfirmed_setting() {
        let root = tempfile::tempdir().unwrap();
        let mut store = DeviceSettingsStore::load(root.path(), root.path()).unwrap();
        let path = store.root().join("settings.json");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.set_startup_consent(true).is_err());
        assert!(!store.settings().login_startup_consent_granted);
    }
}
