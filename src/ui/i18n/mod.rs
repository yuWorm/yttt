mod en;
mod key;
mod zh_cn;

use en::text as english;
pub use key::UiTextKey;
use zh_cn::text as chinese;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    English,
    Chinese,
}

#[derive(Clone, Copy, Debug)]
pub struct UiText {
    locale: Locale,
}

impl UiText {
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub fn english() -> Self {
        Self::new(Locale::English)
    }

    pub fn get(&self, key: UiTextKey) -> &'static str {
        match self.locale {
            Locale::English => english(key),
            Locale::Chinese => chinese(key),
        }
    }
}
