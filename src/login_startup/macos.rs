use std::{
    ffi::CStr,
    fs::{self, OpenOptions},
    io::{self, Write as _},
    sync::mpsc,
    time::Duration,
};

use block::ConcreteBlock;
use cocoa::{
    base::{BOOL, YES, id, nil},
    foundation::{NSAutoreleasePool, NSString},
};
use objc::{
    Message, MessageError,
    rc::autoreleasepool,
    runtime::{Class, Sel},
};
use sha2::{Digest as _, Sha256};

use super::{
    LoginStartupBackend, LoginStartupMethod, LoginStartupSpec, LoginStartupState,
    LoginStartupStatus,
};

const LAUNCH_AGENT_PLIST: &str = "com.yttt.host.plist";
const UPDATE_TIMEOUT: Duration = Duration::from_secs(10);
const REGISTRATION_BUILD_MARKER: &str = "login-startup-macos-build";

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

pub(super) struct MacOsLoginStartupBackend;

impl LoginStartupBackend for MacOsLoginStartupBackend {
    fn status(&self, _spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        if !is_bundled_app()? {
            return Ok(LoginStartupState::unavailable());
        }
        autoreleasepool(|| unsafe { service_state(login_startup_service()?) })
    }

    fn reconcile(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        ensure_bundled_app()?;
        let fingerprint = registration_fingerprint(spec)?;
        autoreleasepool(|| unsafe {
            let service = login_startup_service()?;
            let current = service_state(service)?;
            if !matches!(
                current.status,
                LoginStartupStatus::Enabled | LoginStartupStatus::RequiresApproval
            ) || registration_marker_matches(spec, &fingerprint)?
            {
                return Ok(current);
            }

            unregister_for_update(service)?;
            register_service(service)?;
            write_registration_marker(spec, &fingerprint)?;
            service_state(service)
        })
    }

    fn enable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        ensure_bundled_app()?;
        let fingerprint = registration_fingerprint(spec)?;
        autoreleasepool(|| unsafe {
            let service = login_startup_service()?;
            let current = service_state(service)?;
            if matches!(
                current.status,
                LoginStartupStatus::Enabled | LoginStartupStatus::RequiresApproval
            ) {
                if registration_marker_matches(spec, &fingerprint)? {
                    return Ok(current);
                }
                unregister_for_update(service)?;
            }

            register_service(service)?;
            write_registration_marker(spec, &fingerprint)?;
            service_state(service)
        })
    }

    fn disable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        if !is_bundled_app()? {
            return Ok(LoginStartupState::unavailable());
        }
        autoreleasepool(|| unsafe {
            let service = login_startup_service()?;
            let raw_status = service_status_code(service)?;
            if raw_status == 0 {
                remove_registration_marker(spec)?;
                return Ok(LoginStartupState {
                    status: LoginStartupStatus::Disabled,
                    method: LoginStartupMethod::MacOsServiceManagement,
                });
            }
            // `notFound` can indicate a stale registration after an update removed or damaged the
            // embedded plist. Still ask ServiceManagement to unregister it instead of fabricating a
            // disabled state that leaves the stale item behind.
            if !matches!(raw_status, 1..=3) {
                let _ = login_startup_status(raw_status)?;
            }

            let mut error: id = nil;
            let unregistered: BOOL = (&*service)
                .send_message(
                    Sel::register("unregisterAndReturnError:"),
                    (&mut error as *mut id,),
                )
                .map_err(objc_io_error)?;
            if unregistered != YES {
                return Err(nserror(
                    error,
                    "macOS rejected the login startup unregistration",
                ));
            }
            remove_registration_marker(spec)?;
            Ok(LoginStartupState {
                status: LoginStartupStatus::Disabled,
                method: LoginStartupMethod::MacOsServiceManagement,
            })
        })
    }
}

unsafe fn register_service(service: id) -> io::Result<()> {
    unsafe {
        let mut error: id = nil;
        let registered: BOOL = (&*service)
            .send_message(
                Sel::register("registerAndReturnError:"),
                (&mut error as *mut id,),
            )
            .map_err(objc_io_error)?;
        if registered == YES {
            Ok(())
        } else {
            Err(nserror(
                error,
                "macOS rejected the login startup registration",
            ))
        }
    }
}

unsafe fn unregister_for_update(service: id) -> io::Result<()> {
    unsafe {
        let (sender, receiver) = mpsc::sync_channel(1);
        let completion = ConcreteBlock::new(move |error: id| {
            let result = if error == nil {
                Ok(())
            } else {
                Err(nserror(
                    error,
                    "macOS rejected the login startup update unregistration",
                )
                .to_string())
            };
            let _ = sender.send(result);
        })
        .copy();
        let _: () = (&*service)
            .send_message(
                Sel::register("unregisterWithCompletionHandler:"),
                (&*completion,),
            )
            .map_err(objc_io_error)?;
        match receiver.recv_timeout(UPDATE_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(io::Error::other(error)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out while unregistering the previous macOS login startup service",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other(
                "macOS login startup update callback disconnected",
            )),
        }
    }
}

fn registration_fingerprint(spec: &LoginStartupSpec) -> io::Result<String> {
    let executable = fs::metadata(spec.executable())?;
    let launch_agent = bundled_launch_agent(spec)?;
    let mut digest = Sha256::new();
    digest.update(env!("CARGO_PKG_VERSION").as_bytes());
    digest.update(executable.len().to_le_bytes());
    if let Ok(modified) = executable.modified()
        && let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH)
    {
        digest.update(elapsed.as_nanos().to_le_bytes());
    }
    digest.update(fs::read(launch_agent)?);
    Ok(format!("{:x}", digest.finalize()))
}

fn bundled_launch_agent(spec: &LoginStartupSpec) -> io::Result<std::path::PathBuf> {
    let bundle = spec
        .executable()
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "macOS login startup requires the packaged yttt.app bundle",
            )
        })?;
    Ok(bundle
        .join("Contents/Library/LaunchAgents")
        .join(LAUNCH_AGENT_PLIST))
}

fn registration_marker(spec: &LoginStartupSpec) -> std::path::PathBuf {
    spec.state_root().join(REGISTRATION_BUILD_MARKER)
}

fn registration_marker_matches(spec: &LoginStartupSpec, fingerprint: &str) -> io::Result<bool> {
    match fs::read_to_string(registration_marker(spec)) {
        Ok(saved) => Ok(saved == fingerprint),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn write_registration_marker(spec: &LoginStartupSpec, fingerprint: &str) -> io::Result<()> {
    let path = registration_marker(spec);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "marker path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{REGISTRATION_BUILD_MARKER}.{}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(fingerprint.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn remove_registration_marker(spec: &LoginStartupSpec) -> io::Result<()> {
    match fs::remove_file(registration_marker(spec)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

unsafe fn login_startup_service() -> io::Result<id> {
    unsafe {
        let class = Class::get("SMAppService")
            .ok_or_else(|| io::Error::other("SMAppService is unavailable"))?;
        let plist_name = NSString::alloc(nil)
            .init_str(LAUNCH_AGENT_PLIST)
            .autorelease();
        let service: id = class
            .send_message(Sel::register("agentServiceWithPlistName:"), (plist_name,))
            .map_err(objc_io_error)?;
        if service == nil {
            Err(io::Error::other(
                "SMAppService could not resolve the bundled yttt LaunchAgent",
            ))
        } else {
            Ok(service)
        }
    }
}

unsafe fn service_state(service: id) -> io::Result<LoginStartupState> {
    let status = login_startup_status(unsafe { service_status_code(service)? })?;
    Ok(LoginStartupState {
        status,
        method: LoginStartupMethod::MacOsServiceManagement,
    })
}

unsafe fn service_status_code(service: id) -> io::Result<isize> {
    unsafe {
        (&*service)
            .send_message(Sel::register("status"), ())
            .map_err(objc_io_error)
    }
}

fn login_startup_status(raw: isize) -> io::Result<LoginStartupStatus> {
    match raw {
        0 => Ok(LoginStartupStatus::Disabled),
        1 => Ok(LoginStartupStatus::Enabled),
        2 => Ok(LoginStartupStatus::RequiresApproval),
        3 => Ok(LoginStartupStatus::Unavailable),
        value => Err(io::Error::other(format!(
            "macOS returned unknown SMAppService status {value}"
        ))),
    }
}

fn ensure_bundled_app() -> io::Result<()> {
    if is_bundled_app()? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "macOS login startup requires the packaged yttt.app bundle",
        ))
    }
}

fn is_bundled_app() -> io::Result<bool> {
    Ok(std::env::current_exe()?
        .ancestors()
        .any(|path| path.extension().is_some_and(|extension| extension == "app")))
}

fn nserror(error: id, fallback: &str) -> io::Error {
    if error == nil {
        return io::Error::other(fallback);
    }
    let description = unsafe {
        let description: id =
            match (&*error).send_message(Sel::register("localizedDescription"), ()) {
                Ok(description) => description,
                Err(_) => return io::Error::other(fallback),
            };
        let utf8: *const std::ffi::c_char =
            match (&*description).send_message(Sel::register("UTF8String"), ()) {
                Ok(utf8) => utf8,
                Err(_) => return io::Error::other(fallback),
            };
        if utf8.is_null() {
            fallback.to_string()
        } else {
            CStr::from_ptr(utf8).to_string_lossy().into_owned()
        }
    };
    io::Error::other(description)
}

fn objc_io_error(error: MessageError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_not_found_is_not_reported_as_disabled() {
        assert_eq!(
            login_startup_status(3).unwrap(),
            LoginStartupStatus::Unavailable
        );
    }

    #[test]
    fn registration_marker_detects_an_updated_build() {
        let temp = tempfile::tempdir().unwrap();
        let profile = crate::config::profile::AppProfile::scoped(
            crate::model::ids::ProfileId::new("default"),
            crate::config::profile::EnvironmentKind::Production,
            crate::config::profile::ProfilePersistence::Persistent,
            temp.path(),
            crate::config::profile::ProjectConfigPolicy::Normal,
            crate::config::profile::HostConnectPolicy::ProfileDiscovery,
        );
        let spec = LoginStartupSpec::new(&profile, "/Applications/yttt.app/Contents/MacOS/yttt");

        assert!(!registration_marker_matches(&spec, "build-one").unwrap());
        write_registration_marker(&spec, "build-one").unwrap();
        assert!(registration_marker_matches(&spec, "build-one").unwrap());
        assert!(!registration_marker_matches(&spec, "build-two").unwrap());
        remove_registration_marker(&spec).unwrap();
        assert!(!registration_marker_matches(&spec, "build-one").unwrap());
    }
}
