use std::{fs, io::Write as _, path::Path};

pub(crate) fn atomic_write(path: &Path, source: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".yttt-config-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(source)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;

    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;

    Ok(())
}

pub mod bars;
pub mod default_layout;
pub mod keybindings;
pub mod layout_loader;
pub mod paths;
pub mod personal_layout;
pub mod profile;
pub mod settings;
pub mod ssh;
pub mod ssh_command;
pub mod terminal_placements;
pub mod theme;
