#[test]
#[cfg(target_os = "macos")]
fn application_platform_exposes_system_fonts() {
    let font_names = gpui_platform::current_platform(true)
        .text_system()
        .all_font_names();

    assert!(
        !font_names.is_empty(),
        "gpui_platform must enable font-kit; NoopTextSystem renders all text invisibly"
    );
}

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
