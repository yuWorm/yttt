mod en;
mod key;
mod zh_cn;

use en::text as english;
pub use key::UiTextKey;
use zh_cn::text as chinese;

use crate::config::settings::{LanguageSetting, detect_system_language_setting};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Locale {
    English,
    Chinese,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiText {
    locale: Locale,
}

impl gpui::Global for UiText {}

impl UiText {
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub fn from_language(language: LanguageSetting) -> Self {
        let language = match language {
            LanguageSetting::System => detect_system_language_setting(),
            language => language,
        };
        Self::new(match language {
            LanguageSetting::Chinese => Locale::Chinese,
            LanguageSetting::English | LanguageSetting::System => Locale::English,
        })
    }

    pub fn current(cx: &gpui::App) -> Self {
        cx.try_global::<Self>()
            .copied()
            .unwrap_or_else(|| Self::from_language(LanguageSetting::System))
    }

    pub fn activate(self, cx: &mut gpui::App) {
        if cx.try_global::<Self>() != Some(&self) {
            gpui_component::set_locale(match self.locale {
                Locale::English => "en",
                Locale::Chinese => "zh-CN",
            });
            cx.set_global(self);
        }
    }

    pub fn english() -> Self {
        Self::new(Locale::English)
    }
    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn get(&self, key: UiTextKey) -> &'static str {
        match self.locale {
            Locale::English => english(key),
            Locale::Chinese => chinese(key),
        }
    }
}
