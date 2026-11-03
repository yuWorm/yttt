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

        // CoreText does not fall back to fonts registered at runtime, so the
        // terminal names the bundled Nerd Font; a font without its icons
        // must then draw them from it.
        text_system
            .add_fonts(vec![std::borrow::Cow::Borrowed(
                include_bytes!("../assets/fonts/hack-nerd-font-mono/HackNerdFontMono-Regular.ttf")
                    .as_slice(),
            )])
            .expect("bundled Hack Nerd Font loads");
        let font = |fallbacks| gpui::Font {
            family: if fallbacks {
                "Lilex"
            } else {
                "Hack Nerd Font Mono"
            }
            .into(),
            features: gpui::FontFeatures::disable_ligatures(),
            fallbacks: fallbacks
                .then(|| gpui::FontFallbacks::from_fonts(vec!["Hack Nerd Font Mono".to_string()])),
            weight: gpui::FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        };
        let hack = text_system.font_id(&font(false)).unwrap();
        let lilex = text_system.font_id(&font(true)).unwrap();
        let folder = "\u{f07b}";
        let layout = text_system.layout_line(
            folder,
            gpui::px(15.),
            &[gpui::FontRun {
                len: folder.len(),
                font_id: lilex,
            }],
        );
        // CoreText may hand the fallback back as a separate font instance, so
        // compare the glyph, not the font id.
        let drawn = layout
            .runs
            .first()
            .and_then(|run| run.glyphs.first().map(|glyph| (run.font_id, glyph.id)));
        let expected = text_system.glyph_for_char(hack, '\u{f07b}');
        assert!(
            expected.is_some(),
            "Hack Nerd Font Mono has the folder icon"
        );
        assert_eq!(
            drawn.map(|(_, glyph)| glyph),
            expected,
            "Nerd Font icons must come from the bundled fallback; drew {drawn:?}, Hack is {hack:?}"
        );
    }
}
