use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
};

use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use yttt_core::model::ids::{ConnectionId, CredentialId};

use crate::config::profile::AppProfile;

/// References only. Credentials must be resolved afresh from their existing secure stores.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SavedRemoteTarget {
    Ssh { connection_id: ConnectionId },
    Host { credential_id: CredentialId },
}

fn key(target: &SavedRemoteTarget) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(target).expect("serializable target"))
    )
}

fn directory(profile: &AppProfile) -> PathBuf {
    profile
        .paths()
        .state
        .join("client-connections/open-clients")
}

pub(crate) struct Registration {
    _lease: File,
    path: PathBuf,
    target: SavedRemoteTarget,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // A concurrent process spawn can briefly inherit the descriptor before exec.
        // Release the lease explicitly instead of waiting for every inherited fd to close.
        let _ = fs2::FileExt::unlock(&self._lease);
    }
}

impl Registration {
    /// A process lease prevents a launcher restart from duplicating a still-running Client.
    pub(crate) fn acquire(
        profile: &AppProfile,
        target: SavedRemoteTarget,
    ) -> io::Result<Option<Self>> {
        fs::create_dir_all(&profile.paths().runtime)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lease = options.open(
            profile
                .paths()
                .runtime
                .join(format!("remote-client-{}.lock", key(&target))),
        )?;
        match lease.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self {
                _lease: lease,
                path: directory(profile).join(format!("{}.json", key(&target))),
                target,
            })),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn connected(&self) -> io::Result<()> {
        let parent = self.path.parent().expect("registry parent");
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut temporary, &self.target)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path)?;
        sync_directory(parent)
    }

    fn closed(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => sync_directory(self.path.parent().expect("registry parent")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn forget(profile: &AppProfile, target: &SavedRemoteTarget) -> io::Result<()> {
    let path = directory(profile).join(format!("{}.json", key(target)));
    match fs::remove_file(path) {
        Ok(()) => sync_directory(&directory(profile)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn forget_if_idle(profile: &AppProfile, target: SavedRemoteTarget) -> io::Result<()> {
    if let Some(registration) = Registration::acquire(profile, target)? {
        registration.closed()?;
    }
    Ok(())
}

pub(crate) fn pending(profile: &AppProfile) -> io::Result<Vec<SavedRemoteTarget>> {
    let entries = match fs::read_dir(directory(profile)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut targets = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        if !entry.file_type()?.is_file() {
            return Err(io::Error::other(
                "remote restore record is not a regular file",
            ));
        }
        let mut bytes = Vec::new();
        File::open(entry.path())?
            .take(4097)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(io::Error::other("remote restore record exceeds 4 KiB"));
        }
        let target: SavedRemoteTarget = serde_json::from_slice(&bytes)?;
        if entry.file_name() != std::ffi::OsString::from(format!("{}.json", key(&target))) {
            return Err(io::Error::other("remote restore record identity mismatch"));
        }
        if Registration::acquire(profile, target.clone())?.is_some() {
            targets.push(target);
        }
    }
    Ok(targets)
}

pub(crate) struct RemoteRestoreGlobal {
    pub registration: Option<Registration>,
    windows: usize,
}
impl gpui::Global for RemoteRestoreGlobal {}
impl RemoteRestoreGlobal {
    pub(crate) fn new(registration: Option<Registration>) -> Self {
        Self {
            registration,
            windows: 0,
        }
    }
    pub(crate) fn opened_window(&mut self) {
        self.windows += 1;
    }
    pub(crate) fn close_window(&mut self) -> io::Result<()> {
        if self.windows <= 1 {
            if let Some(registration) = &self.registration {
                registration.closed()?;
            }
        }
        self.windows = self.windows.saturating_sub(1);
        Ok(())
    }
    pub(crate) fn cancel_connect(&self) -> io::Result<()> {
        if let Some(registration) = &self.registration {
            registration.closed()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_preserves_quit_but_forgets_explicit_last_window_close() {
        let root = tempfile::tempdir().unwrap();
        use crate::config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        let profile = AppProfile::scoped(
            yttt_core::model::ids::ProfileId::new("remote-restore"),
            EnvironmentKind::Test,
            ProfilePersistence::Persistent,
            root.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        let target = SavedRemoteTarget::Host {
            credential_id: CredentialId::random(),
        };
        let registration = Registration::acquire(&profile, target.clone())
            .unwrap()
            .unwrap();
        registration.connected().unwrap();
        assert!(
            pending(&profile).unwrap().is_empty(),
            "running Clients must not duplicate"
        );
        assert!(
            Registration::acquire(&profile, target.clone())
                .unwrap()
                .is_none()
        );
        drop(registration);
        assert_eq!(pending(&profile).unwrap(), vec![target.clone()]);
        let registration = Registration::acquire(&profile, target).unwrap().unwrap();
        let mut client = RemoteRestoreGlobal::new(Some(registration));
        client.opened_window();
        client.opened_window();
        client.close_window().unwrap();
        assert!(client.registration.as_ref().unwrap().path.exists());
        client.close_window().unwrap();
        drop(client);
        assert!(pending(&profile).unwrap().is_empty());
    }
}
