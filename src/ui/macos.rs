#[cfg(target_os = "macos")]
pub const FALLBACK_APP_ICON_PATH: &str =
    "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericApplicationIcon.icns";

#[cfg(target_os = "macos")]
pub fn fallback_app_icon_path() -> &'static str {
    FALLBACK_APP_ICON_PATH
}

#[cfg(target_os = "macos")]
pub fn prepare_macos_app_runtime() {
    use cocoa::{
        appkit::{NSApplication as _, NSApplicationActivationPolicyRegular, NSImage, NSWindow},
        base::{NO, id, nil},
        foundation::NSString,
    };

    unsafe {
        <id as NSWindow>::setAllowsAutomaticWindowTabbing_(nil, NO);

        let app = cocoa::appkit::NSApp();
        if app == nil {
            return;
        }

        let _ = app.setActivationPolicy_(NSApplicationActivationPolicyRegular);

        let path = NSString::alloc(nil).init_str(FALLBACK_APP_ICON_PATH);
        let image: id = NSImage::alloc(nil).initWithContentsOfFile_(path);
        if image == nil {
            return;
        }

        app.setApplicationIconImage_(image);
    }
}
