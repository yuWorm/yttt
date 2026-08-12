use std::path::{Path, PathBuf};

use yttt_core::model::ids::ProfileId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointAddress {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    WindowsPipe(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEndpoint {
    profile_id: ProfileId,
    runtime_root: PathBuf,
    address: EndpointAddress,
}

impl LocalEndpoint {
    pub fn for_profile(profile_id: ProfileId, runtime_root: impl Into<PathBuf>) -> Self {
        let runtime_root = runtime_root.into();
        #[cfg(unix)]
        let address = EndpointAddress::Unix(runtime_root.join("host.sock"));
        #[cfg(windows)]
        let address = {
            let profile_hash = crc32fast::hash(profile_id.as_str().as_bytes());
            EndpointAddress::WindowsPipe(format!(r"\\.\pipe\yttt-{profile_hash:08x}-host"))
        };
        Self {
            profile_id,
            runtime_root,
            address,
        }
    }

    pub fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    pub fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    pub fn address(&self) -> &EndpointAddress {
        &self.address
    }

    #[cfg(unix)]
    pub fn unix_path(&self) -> &Path {
        let EndpointAddress::Unix(path) = &self.address;
        path
    }

    #[cfg(windows)]
    pub fn pipe_name(&self) -> &str {
        let EndpointAddress::WindowsPipe(name) = &self.address;
        name
    }
}
