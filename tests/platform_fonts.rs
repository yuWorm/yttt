//! Runs without the test harness because the macOS platform must be created on
//! the main thread.

fn main() {
    #[cfg(target_os = "macos")]
    {
        let text_system = gpui_platform::current_platform(true).text_system();
        let font_names = text_system.all_font_names();

        assert!(
            !font_names.is_empty(),
            "gpui_platform must enable font-kit; NoopTextSystem renders all text invisibly"
        );

        text_system
            .add_fonts(vec![std::borrow::Cow::Borrowed(
                include_bytes!("../assets/fonts/lilex/Lilex-Regular.ttf").as_slice(),
            )])
            .expect("bundled Lilex loads");
        assert!(
            text_system
                .all_font_names()
                .iter()
                .any(|name| name == "Lilex"),
            "bundled Lilex must register under the family name settings use"
        );
    }
}
