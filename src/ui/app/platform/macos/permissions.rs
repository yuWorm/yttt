use std::{
    ffi::c_void,
    io,
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration,
};

use block::ConcreteBlock;
use cocoa::base::{BOOL, YES, id, nil};
use objc::{
    Message, MessageError,
    rc::autoreleasepool,
    runtime::{Class, Sel},
};

use super::super::{PermissionKind, PermissionStatus};

const NOTIFICATION_STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const NOTIFICATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const NOTIFICATION_OPTION_BADGE: usize = 1 << 0;
const NOTIFICATION_OPTION_SOUND: usize = 1 << 1;
const NOTIFICATION_OPTION_ALERT: usize = 1 << 2;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

pub(in crate::ui::app::platform) fn detect_permission_status(
    kind: PermissionKind,
) -> io::Result<PermissionStatus> {
    match kind {
        PermissionKind::Notifications => notification_status(),
        PermissionKind::Accessibility => Ok(accessibility_status()),
        PermissionKind::ScreenCapture => Ok(screen_capture_status()),
        PermissionKind::FileSystem | PermissionKind::DeveloperTools => {
            Ok(PermissionStatus::Unavailable)
        }
    }
}

pub(in crate::ui::app::platform) fn request_native_permission(
    kind: PermissionKind,
) -> io::Result<PermissionStatus> {
    match kind {
        PermissionKind::Notifications => request_notifications(),
        PermissionKind::Accessibility => Ok(request_accessibility()),
        PermissionKind::ScreenCapture => Ok(request_screen_capture()),
        PermissionKind::FileSystem | PermissionKind::DeveloperTools => {
            Ok(PermissionStatus::Unavailable)
        }
    }
}

fn notification_status() -> io::Result<PermissionStatus> {
    ensure_bundled_app()?;
    autoreleasepool(|| unsafe {
        let center_class = Class::get("UNUserNotificationCenter")
            .ok_or_else(|| io::Error::other("UNUserNotificationCenter class is unavailable"))?;
        let center: id = center_class
            .send_message(Sel::register("currentNotificationCenter"), ())
            .map_err(objc_io_error)?;
        if center == nil {
            return Err(io::Error::other("UNUserNotificationCenter is unavailable"));
        }

        let (sender, receiver) = mpsc::sync_channel(1);
        let completion = ConcreteBlock::new(move |settings: id| {
            let result = (&*settings)
                .send_message(Sel::register("authorizationStatus"), ())
                .map_err(|error| error.to_string())
                .and_then(notification_status_from_raw);
            let _ = sender.send(result);
        })
        .copy();
        let _: () = (&*center)
            .send_message(
                Sel::register("getNotificationSettingsWithCompletionHandler:"),
                (&*completion,),
            )
            .map_err(objc_io_error)?;

        receive_notification_result(receiver.recv_timeout(NOTIFICATION_STATUS_TIMEOUT))
    })
}

fn request_notifications() -> io::Result<PermissionStatus> {
    ensure_bundled_app()?;
    autoreleasepool(|| unsafe {
        let center_class = Class::get("UNUserNotificationCenter")
            .ok_or_else(|| io::Error::other("UNUserNotificationCenter class is unavailable"))?;
        let center: id = center_class
            .send_message(Sel::register("currentNotificationCenter"), ())
            .map_err(objc_io_error)?;
        if center == nil {
            return Err(io::Error::other("UNUserNotificationCenter is unavailable"));
        }

        let options =
            NOTIFICATION_OPTION_BADGE | NOTIFICATION_OPTION_SOUND | NOTIFICATION_OPTION_ALERT;
        let (sender, receiver) = mpsc::sync_channel(1);
        let completion = ConcreteBlock::new(move |granted: BOOL, error: id| {
            let result = if error == nil {
                Ok(if granted == YES {
                    PermissionStatus::Granted
                } else {
                    PermissionStatus::Denied
                })
            } else {
                Err("macOS rejected the notification authorization request".to_string())
            };
            let _ = sender.send(result);
        })
        .copy();
        let _: () = (&*center)
            .send_message(
                Sel::register("requestAuthorizationWithOptions:completionHandler:"),
                (options, &*completion),
            )
            .map_err(objc_io_error)?;

        receive_notification_result(receiver.recv_timeout(NOTIFICATION_REQUEST_TIMEOUT))
    })
}

fn receive_notification_result(
    result: Result<Result<PermissionStatus, String>, RecvTimeoutError>,
) -> io::Result<PermissionStatus> {
    match result {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(message)) => Err(io::Error::other(message)),
        Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "macOS permission request timed out",
        )),
        Err(RecvTimeoutError::Disconnected) => {
            Err(io::Error::other("macOS permission callback disconnected"))
        }
    }
}

fn objc_io_error(error: MessageError) -> io::Error {
    io::Error::other(error.to_string())
}

fn ensure_bundled_app() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    if executable
        .ancestors()
        .any(|path| path.extension().is_some_and(|extension| extension == "app"))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "notification authorization requires the macOS app bundle",
        ))
    }
}

fn notification_status_from_raw(status: isize) -> Result<PermissionStatus, String> {
    match status {
        0 => Ok(PermissionStatus::NotDetermined),
        1 => Ok(PermissionStatus::Denied),
        2..=4 => Ok(PermissionStatus::Granted),
        value => Err(format!(
            "macOS returned an unknown notification authorization status: {value}"
        )),
    }
}

fn accessibility_status() -> PermissionStatus {
    let trusted = unsafe { AXIsProcessTrusted() != 0 };
    if trusted {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    }
}

fn request_accessibility() -> PermissionStatus {
    autoreleasepool(|| unsafe {
        let number_class = Class::get("NSNumber").expect("NSNumber must be available on macOS");
        let prompt: id = number_class
            .send_message(Sel::register("numberWithBool:"), (YES,))
            .expect("NSNumber numberWithBool: must be available on macOS");
        let dictionary_class =
            Class::get("NSDictionary").expect("NSDictionary must be available on macOS");
        let options: id = dictionary_class
            .send_message(
                Sel::register("dictionaryWithObject:forKey:"),
                (prompt, kAXTrustedCheckOptionPrompt as id),
            )
            .expect("NSDictionary dictionaryWithObject:forKey: must be available on macOS");
        if AXIsProcessTrustedWithOptions(options.cast()) != 0 {
            PermissionStatus::Granted
        } else {
            PermissionStatus::Denied
        }
    })
}

fn screen_capture_status() -> PermissionStatus {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    }
}

fn request_screen_capture() -> PermissionStatus {
    if unsafe { CGRequestScreenCaptureAccess() } {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_authorization_values_map_to_user_facing_statuses() {
        assert_eq!(
            notification_status_from_raw(0),
            Ok(PermissionStatus::NotDetermined)
        );
        assert_eq!(
            notification_status_from_raw(1),
            Ok(PermissionStatus::Denied)
        );
        assert_eq!(
            notification_status_from_raw(2),
            Ok(PermissionStatus::Granted)
        );
        assert_eq!(
            notification_status_from_raw(3),
            Ok(PermissionStatus::Granted)
        );
        assert!(notification_status_from_raw(99).is_err());
    }
}
