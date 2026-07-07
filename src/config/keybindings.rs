use std::collections::HashMap;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct Keybinding {
    pub keys: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct KeybindingsConfig {
    #[serde(default)]
    pub bindings: Vec<Keybinding>,
}

impl KeybindingsConfig {
    pub fn conflicts(&self) -> Vec<KeybindingConflict> {
        let mut by_keys: HashMap<String, Vec<String>> = HashMap::new();
        for binding in &self.bindings {
            by_keys
                .entry(normalize_keys(&binding.keys))
                .or_default()
                .push(binding.command.clone());
        }

        let mut conflicts: Vec<_> = by_keys
            .into_iter()
            .filter_map(|(keys, commands)| {
                if commands.len() > 1 {
                    Some(KeybindingConflict { keys, commands })
                } else {
                    None
                }
            })
            .collect();
        conflicts.sort_by(|left, right| left.keys.cmp(&right.keys));
        conflicts
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub keys: String,
    pub commands: Vec<String>,
}

pub fn default_keybindings() -> KeybindingsConfig {
    KeybindingsConfig {
        bindings: vec![
            binding("cmd-o", "project.open"),
            binding("ctrl-o", "project.open"),
            binding("cmd-p", "command_palette.open"),
            binding("ctrl-p", "command_palette.open"),
            binding("cmd-shift-o", "project.palette"),
            binding("ctrl-shift-o", "project.palette"),
            binding("cmd-j", "tab.palette"),
            binding("ctrl-j", "tab.palette"),
            binding("cmd-k", "pane.palette"),
            binding("ctrl-k", "pane.palette"),
            binding("cmd-t", "tab.new"),
            binding("ctrl-t", "tab.new"),
            binding("cmd-d", "pane.split_vertical"),
            binding("ctrl-d", "pane.split_vertical"),
            binding("cmd-shift-d", "pane.split_horizontal"),
            binding("ctrl-shift-d", "pane.split_horizontal"),
        ],
    }
}

fn binding(keys: &str, command: &str) -> Keybinding {
    Keybinding {
        keys: keys.to_string(),
        command: command.to_string(),
    }
}

fn normalize_keys(keys: &str) -> String {
    keys.trim().to_ascii_lowercase()
}
