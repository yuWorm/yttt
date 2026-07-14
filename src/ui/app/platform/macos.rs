#[cfg(target_os = "macos")]
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
}
