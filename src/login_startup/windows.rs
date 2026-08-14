use std::io;

use winreg::{
    HKCU,
    enums::{KEY_READ, KEY_SET_VALUE},
};

use super::{
    LoginStartupBackend, LoginStartupMethod, LoginStartupSpec, LoginStartupState,
    LoginStartupStatus, quote_windows_argument,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "yttt Host (default)";

pub(super) struct WindowsLoginStartupBackend;

impl LoginStartupBackend for WindowsLoginStartupBackend {
    fn status(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        let expected = registration_command(spec)?;
        let status = match HKCU.open_subkey_with_flags(RUN_KEY, KEY_READ) {
            Ok(key) => match key.get_value::<String, _>(VALUE_NAME) {
                Ok(command) if command == expected => LoginStartupStatus::Enabled,
                Ok(_) | Err(_) => LoginStartupStatus::Disabled,
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => LoginStartupStatus::Disabled,
            Err(error) => return Err(error),
        };
        Ok(LoginStartupState {
            status,
            method: LoginStartupMethod::WindowsCurrentUserRun,
        })
    }

    fn enable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        let command = registration_command(spec)?;
        let (key, _) = HKCU.create_subkey(RUN_KEY)?;
        key.set_value(VALUE_NAME, &command)?;
        self.status(spec)
    }

    fn disable(&self, spec: &LoginStartupSpec) -> io::Result<LoginStartupState> {
        match HKCU.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            Ok(key) => match key.delete_value(VALUE_NAME) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.status(spec)
    }
}

fn registration_command(spec: &LoginStartupSpec) -> io::Result<String> {
    let executable = spec.executable().to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the yttt executable path is not valid Unicode",
        )
    })?;
    let mut arguments = Vec::with_capacity(4);
    arguments.push(quote_windows_argument(executable));
    for argument in spec.arguments() {
        let argument = argument.into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "a login startup argument is not valid Unicode",
            )
        })?;
        arguments.push(quote_windows_argument(&argument));
    }
    Ok(arguments.join(" "))
}
