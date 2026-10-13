//! Register the installed macOS bundle, never a development build or mounted DMG.
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const CLI_PATH: &str = "/usr/local/bin/yttt";

fn installed_executable(executable: &Path, home: Option<&Path>) -> bool {
    let Some(bundle) = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return false;
    };
    if bundle
        .extension()
        .is_none_or(|extension| extension != "app")
        || executable.strip_prefix(bundle).ok() != Some(Path::new("Contents/MacOS/yttt"))
    {
        return false;
    }
    bundle.parent() == Some(Path::new("/Applications"))
        || home.is_some_and(|home| bundle.parent() == Some(home.join("Applications").as_path()))
}

fn is_registered(executable: &Path, destination: &Path) -> bool {
    fs::read_link(destination).is_ok_and(|target| target == executable)
}

fn register_link(executable: &Path, destination: &Path) -> io::Result<()> {
    if is_registered(executable, destination) {
        return Ok(());
    }
    // symlink_metadata also sees broken links. Neither those nor regular files are ours to replace.
    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} already exists and points elsewhere. It was not changed.",
                    destination.display()
                ),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(
        destination
            .parent()
            .ok_or_else(|| io::Error::other("missing CLI directory"))?,
    )?;
    match std::os::unix::fs::symlink(executable, destination) {
        Err(_) if is_registered(executable, destination) => Ok(()),
        result => result,
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn applescript_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}

fn privileged_registration(executable: &Path) -> io::Result<()> {
    let executable = executable
        .to_str()
        .ok_or_else(|| io::Error::other("application path is not UTF-8"))?;
    // No force flag: a competing installation or unrelated command must never be overwritten.
    let script = format!(
        "/bin/mkdir -p /usr/local/bin && test ! -e {destination} && test ! -L {destination} && /bin/ln -s -h {source} {destination}",
        source = shell_quote(executable),
        destination = shell_quote(CLI_PATH)
    );
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(format!(
            "do shell script {} with administrator privileges with prompt {}",
            applescript_string(&script),
            applescript_string(
                "yttt needs permission to install its terminal command at /usr/local/bin/yttt."
            )
        ))
        .output()?;
    if output.status.success() || is_registered(Path::new(executable), Path::new(CLI_PATH)) {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

/// Runs on the background executor; authorization and failure dialogs are native macOS dialogs.
pub(crate) fn register_installed_app() {
    if cfg!(debug_assertions) || std::env::var_os("YTTT_PROFILE_ROOT").is_some() {
        return;
    }
    let result = (|| {
        let executable = std::env::current_exe()?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if !installed_executable(&executable, home.as_deref()) {
            return Ok(());
        }
        match register_link(&executable, Path::new(CLI_PATH)) {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                privileged_registration(&executable)
            }
            result => result,
        }
    })();
    if let Err(error) = result {
        let message = format!(
            "The yttt terminal command could not be installed. The application can still be used.\n\n{error}\n\nResolve the conflict or allow installation, then restart yttt to retry. You can also run the application's Contents/MacOS/yttt executable directly."
        );
        eprintln!("{message}");
        let _ = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(format!(
                "display alert \"yttt command-line installation\" message {} as warning",
                applescript_string(&message)
            ))
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_paths_cannot_inject_shell_commands() {
        let path = "/Applications/yttt's \"app\" $(printf injected)\n.app/Contents/MacOS/yttt";
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote(path)))
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, path.as_bytes());
    }

    #[test]
    fn only_installed_bundles_are_registered() {
        let home = Some(Path::new("/Users/test"));
        assert!(installed_executable(
            Path::new("/Applications/yttt.app/Contents/MacOS/yttt"),
            home
        ));
        assert!(installed_executable(
            Path::new("/Users/test/Applications/yttt.app/Contents/MacOS/yttt"),
            home
        ));
        for path in [
            "/Volumes/yttt/yttt.app/Contents/MacOS/yttt",
            "/tmp/yttt.app/Contents/MacOS/yttt",
            "/Applications/yttt",
            "/Applications/yttt.app/Contents/yttt",
            "/Applications/yttt.app/Contents/MacOS/other",
        ] {
            assert!(!installed_executable(Path::new(path), home), "{path}");
        }
    }

    #[test]
    fn registration_is_idempotent_and_preserves_foreign_entries() {
        let directory =
            std::env::temp_dir().join(format!("yttt-cli-registration-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let executable = directory.join("yttt app's binary");
        fs::write(&executable, "original").unwrap();
        let destination = directory.join("bin/yttt");
        register_link(&executable, &destination).unwrap();
        register_link(&executable, &destination).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
        fs::remove_file(&destination).unwrap();
        fs::write(&destination, "foreign command").unwrap();
        assert_eq!(
            register_link(&executable, &destination).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&destination).unwrap(), "foreign command");
        fs::remove_file(&destination).unwrap();
        std::os::unix::fs::symlink(directory.join("missing"), &destination).unwrap();
        assert_eq!(
            register_link(&executable, &destination).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read_link(&destination).unwrap(),
            directory.join("missing")
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
