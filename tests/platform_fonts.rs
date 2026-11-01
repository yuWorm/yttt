//! Runs without the test harness because the macOS platform must be created on
//! the main thread.

fn main() {
    #[cfg(target_os = "macos")]
    {
        let font_names = gpui_platform::current_platform(true)
            .text_system()
            .all_font_names();

        assert!(
            !font_names.is_empty(),
            "gpui_platform must enable font-kit; NoopTextSystem renders all text invisibly"
        );
    }
}
