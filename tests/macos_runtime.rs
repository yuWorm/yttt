#[test]
#[cfg(target_os = "macos")]
fn prepare_macos_app_runtime_disables_automatic_window_tabbing() {
    use cocoa::{
        appkit::NSWindow,
        base::{NO, id, nil},
    };

    unsafe {
        <id as NSWindow>::setAllowsAutomaticWindowTabbing_(nil, cocoa::base::YES);
    }

    yttt::ui::app::platform::macos::prepare_macos_app_runtime();

    let allows_tabbing = unsafe { <id as NSWindow>::allowsAutomaticWindowTabbing(nil) };
    assert_eq!(allows_tabbing, NO);
}
