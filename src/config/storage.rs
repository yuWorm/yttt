use parking_lot::RwLock;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

/// Shared configuration belongs to the bound Host, including for a local desktop.
pub trait ConfigStorage: Send + Sync {
    fn config_root(&self) -> &Path;
    fn is_remote(&self) -> bool;
    fn install_agent_hooks(&self) -> io::Result<()>;
    fn environment(&self) -> &yttt_protocol::workspace::WorkspaceEnvironment;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_directory(&self, path: &Path) -> io::Result<()>;
    fn rename_directory(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    fn exists(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
}

static ENVIRONMENT: LazyLock<RwLock<Option<Arc<dyn ConfigStorage>>>> =
    LazyLock::new(|| RwLock::new(None));
static TEST_ROOTS: LazyLock<RwLock<Vec<PathBuf>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Pure configuration tests and explicit UI fixtures may opt into their own local files.
/// This is never inferred from a failed connection or an unbound production profile.
pub(crate) fn allow_test_root(path: &Path) {
    let mut roots = TEST_ROOTS.write();
    for root in std::iter::once(path.to_path_buf()).chain(std::fs::canonicalize(path).ok()) {
        if !roots.iter().any(|existing| root.starts_with(existing)) {
            roots.push(root);
        }
    }
}

pub fn bind_environment(storage: Arc<dyn ConfigStorage>) -> io::Result<()> {
    let mut current = ENVIRONMENT.write();
    if current.as_ref().is_some_and(|current| {
        current.environment().environment_id != storage.environment().environment_id
    }) {
        return Err(io::Error::other(
            "a Client process cannot switch environments",
        ));
    }
    *current = Some(storage);
    Ok(())
}

pub fn environment_storage() -> Option<Arc<dyn ConfigStorage>> {
    ENVIRONMENT.read().clone()
}

pub(crate) fn storage_for(path: &Path) -> io::Result<Option<Arc<dyn ConfigStorage>>> {
    if let Some(storage) = environment_storage() {
        return Ok(Some(storage));
    }
    if TEST_ROOTS.read().iter().any(|root| path.starts_with(root)) {
        return Ok(None);
    }
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "environment storage is not bound to its Host",
    ))
}

pub fn is_remote() -> bool {
    environment_storage().is_some_and(|storage| storage.is_remote())
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    match storage_for(path.as_ref())? {
        Some(storage) => storage.read(path.as_ref()),
        None => std::fs::read(path),
    }
}
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    match storage_for(path.as_ref())? {
        Some(storage) => storage.write(path.as_ref(), bytes.as_ref()),
        None => std::fs::write(path, bytes),
    }
}
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    match storage_for(path.as_ref())? {
        Some(storage) => storage.create_dir_all(path.as_ref()),
        None => std::fs::create_dir_all(path),
    }
}
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    match storage_for(path.as_ref())? {
        Some(storage) => storage.remove_file(path.as_ref()),
        None => std::fs::remove_file(path),
    }
}
pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(storage) = storage_for(path)? {
        for entry in storage.read_dir(path)? {
            if storage.is_dir(&entry) {
                remove_dir_all(&entry)?;
            } else {
                storage.remove_file(&entry)?;
            }
        }
        storage.remove_directory(path)
    } else {
        std::fs::remove_dir_all(path)
    }
}
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    match storage_for(path.as_ref())? {
        Some(storage) => storage.canonicalize(path.as_ref()),
        None => dunce::canonicalize(path.as_ref()),
    }
}
pub fn exists(path: impl AsRef<Path>) -> bool {
    match storage_for(path.as_ref()) {
        Ok(Some(storage)) => storage.exists(path.as_ref()),
        Ok(None) => path.as_ref().exists(),
        Err(_) => false,
    }
}
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    match storage_for(path.as_ref()) {
        Ok(Some(storage)) => storage.is_dir(path.as_ref()),
        Ok(None) => path.as_ref().is_dir(),
        Err(_) => false,
    }
}
pub fn is_file(path: impl AsRef<Path>) -> bool {
    match storage_for(path.as_ref()) {
        Ok(Some(storage)) => storage.read(path.as_ref()).is_ok(),
        Ok(None) => path.as_ref().is_file(),
        Err(_) => false,
    }
}
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    if let Some(storage) = storage_for(from.as_ref())? {
        if storage.is_dir(from.as_ref()) {
            return storage.rename_directory(from.as_ref(), to.as_ref());
        }
        storage.write(to.as_ref(), &storage.read(from.as_ref())?)?;
        storage.remove_file(from.as_ref())
    } else {
        storage_for(to.as_ref())?;
        std::fs::rename(from, to)
    }
}
pub fn sync(path: impl AsRef<Path>) -> io::Result<()> {
    if storage_for(path.as_ref())?.is_some() {
        // Host write acknowledgments already include file and directory fsync.
        Ok(())
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .sync_all()
    }
}
pub struct DirectoryEntry(PathBuf);
impl DirectoryEntry {
    pub fn path(&self) -> PathBuf {
        self.0.clone()
    }
}
pub fn read_dir(
    path: impl AsRef<Path>,
) -> io::Result<std::vec::IntoIter<io::Result<DirectoryEntry>>> {
    let entries = if let Some(storage) = storage_for(path.as_ref())? {
        storage
            .read_dir(path.as_ref())?
            .into_iter()
            .map(|path| Ok(DirectoryEntry(path)))
            .collect()
    } else {
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| DirectoryEntry(entry.path())))
            .collect::<Vec<_>>()
    };
    Ok(entries.into_iter())
}
