use gpui::Keystroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeybindingDisplayPlatform {
    Mac,
    Other,
}

pub fn current_keybinding_display_platform() -> KeybindingDisplayPlatform {
    if cfg!(target_os = "macos") {
        KeybindingDisplayPlatform::Mac
    } else {
        KeybindingDisplayPlatform::Other
    }
}

pub fn display_keybindings_for_current_platform(keys: &[String]) -> Vec<String> {
    display_keybindings_for_platform(keys, current_keybinding_display_platform())
}

pub fn primary_display_keybinding_for_current_platform(keys: &[String]) -> Option<String> {
    display_keybindings_for_current_platform(keys)
        .into_iter()
        .next()
}

pub fn display_keybindings_for_platform(
    keys: &[String],
    platform: KeybindingDisplayPlatform,
) -> Vec<String> {
    let normalized = normalized_keybindings(keys);
    if normalized.is_empty() {
        return Vec::new();
    }

    let preferred = normalized
        .iter()
        .filter(|key| key_matches_platform(key, platform))
        .cloned()
        .collect::<Vec<_>>();
    if !preferred.is_empty() {
        return preferred;
    }

    let neutral = normalized
        .iter()
        .filter(|key| key_is_platform_neutral(key))
        .cloned()
        .collect::<Vec<_>>();
    if !neutral.is_empty() {
        return neutral;
    }

    normalized.into_iter().take(1).collect()
}

pub fn parse_keybinding_for_display(keys: &str) -> Option<Keystroke> {
    Keystroke::parse(keys).ok()
}

pub fn recorded_keybinding(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.trim();
    if key.is_empty()
        || is_modifier_key(key)
        || (keystroke.is_ime_in_progress() && key.chars().count() == 1)
    {
        return None;
    }

    Some(keystroke.unparse())
}

fn is_modifier_key(key: &str) -> bool {
    matches!(
        key,
        "shift"
            | "control"
            | "ctrl"
            | "alt"
            | "option"
            | "platform"
            | "command"
            | "cmd"
            | "super"
            | "win"
            | "windows"
            | "function"
            | "fn"
    )
}

fn normalized_keybindings(keys: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for key in keys {
        let key = key.trim().to_ascii_lowercase();
        if !key.is_empty() && !normalized.contains(&key) {
            normalized.push(key);
        }
    }
    normalized
}

fn key_matches_platform(keys: &str, platform: KeybindingDisplayPlatform) -> bool {
    match platform {
        KeybindingDisplayPlatform::Mac => {
            has_keybinding_token(keys, "cmd") || has_keybinding_token(keys, "secondary")
        }
        KeybindingDisplayPlatform::Other => {
            has_keybinding_token(keys, "ctrl")
                && !has_keybinding_token(keys, "cmd")
                && !has_keybinding_token(keys, "secondary")
        }
    }
}

fn key_is_platform_neutral(keys: &str) -> bool {
    !has_keybinding_token(keys, "cmd")
        && !has_keybinding_token(keys, "secondary")
        && !has_keybinding_token(keys, "ctrl")
}

fn has_keybinding_token(keys: &str, token: &str) -> bool {
    keys.split('-').any(|part| part == token)
}
