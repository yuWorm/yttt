#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerItem {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: Option<String>,
    pub keybinding: Option<String>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

impl PickerItem {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            status: None,
            keybinding: None,
            enabled: true,
            disabled_reason: None,
        }
    }
}
