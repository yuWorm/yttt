#[cfg(target_os = "macos")]
mod permissions;
pub(super) use permissions::{
    current_bundle_identifier, detect_permission_status, request_native_permission,
};

pub fn prepare_macos_app_runtime() {
    use cocoa::{
        appkit::{NSApplication as _, NSApplicationActivationPolicyRegular, NSWindow},
        base::{NO, id, nil},
    };

    unsafe {
        <id as NSWindow>::setAllowsAutomaticWindowTabbing_(nil, NO);

        let app = cocoa::appkit::NSApp();
        if app == nil {
            return;
        }

        let _ = app.setActivationPolicy_(NSApplicationActivationPolicyRegular);
    }
    disable_app_nap();
}

/// Keeps App Nap from throttling the app for the life of the process: a
/// throttled UI process stops reading its Host connection in time, misses
/// heartbeats and reconnects. Idle system sleep stays allowed.
fn disable_app_nap() {
    use cocoa::{
        base::{id, nil},
        foundation::NSString,
    };
    use objc::{
        Message,
        runtime::{Class, Sel},
    };
    /// `NSActivityUserInitiatedAllowingIdleSystemSleep`.
    const USER_INITIATED_ALLOWING_IDLE_SYSTEM_SLEEP: u64 = 0x00FF_FFFF;
    static ACTIVITY: std::sync::Once = std::sync::Once::new();

    ACTIVITY.call_once(|| unsafe {
        let Some(process_info) = Class::get("NSProcessInfo") else {
            return;
        };
        let Ok(info) = process_info.send_message::<_, id>(Sel::register("processInfo"), ()) else {
            return;
        };
        if info == nil {
            return;
        }
        let reason = NSString::alloc(nil).init_str("Keeping Host connections responsive");
        let Ok(activity) = (&*info).send_message::<_, id>(
            Sel::register("beginActivityWithOptions:reason:"),
            (USER_INITIATED_ALLOWING_IDLE_SYSTEM_SLEEP, reason),
        ) else {
            return;
        };
        // The activity lasts while its token lives; keep it for the process.
        if activity != nil {
            let _ = (&*activity).send_message::<_, id>(Sel::register("retain"), ());
        }
    });
}
