use gpui::{Pixels, Rgba};
use gpui_component::{Sizable as _, spinner::Spinner};

/// A loading spinner `size` across, tinted `color`.
pub fn yttt_spinner(size: Pixels, color: Rgba) -> Spinner {
    Spinner::new().with_size(size).color(color.into())
}

/// A small loading spinner in the current text color.
pub fn yttt_small_spinner() -> Spinner {
    Spinner::new().small()
}
