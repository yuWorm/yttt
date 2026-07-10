pub const SYSTEM_FONT_FAMILY_LABEL: &str = "System default";

pub fn font_family_options_from_system(
    current: &str,
    system_fonts: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<String> {
    let current = current.trim();
    let mut values = vec![SYSTEM_FONT_FAMILY_LABEL.to_string()];
    let mut fonts: Vec<String> = system_fonts
        .into_iter()
        .map(Into::into)
        .map(|font| font.trim().to_string())
        .filter(|font| !font.is_empty())
        .collect();
    fonts.sort_by_key(|font| font.to_ascii_lowercase());
    fonts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    values.extend(fonts);

    if !current.is_empty() && values.iter().all(|font| font != current) {
        values.insert(1, current.to_string());
    }

    values
}

pub fn font_family_option_for_setting(font_family: &str) -> String {
    let font_family = font_family.trim();
    if font_family.is_empty() {
        SYSTEM_FONT_FAMILY_LABEL.to_string()
    } else {
        font_family.to_string()
    }
}

pub fn font_family_setting_from_option(option: &str) -> String {
    if option == SYSTEM_FONT_FAMILY_LABEL {
        String::new()
    } else {
        option.to_string()
    }
}

pub fn terminal_font_family_options_from_system(
    current: &str,
    system_fonts: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<String> {
    font_family_options_from_system(current, system_fonts)
}

pub fn terminal_font_family_option_for_setting(font_family: &str) -> String {
    font_family_option_for_setting(font_family)
}

pub fn terminal_font_family_setting_from_option(option: &str) -> String {
    font_family_setting_from_option(option)
}
