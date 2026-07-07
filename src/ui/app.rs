use gpui::{px, size, App, AppContext, Application, Bounds, WindowBounds, WindowOptions};

use crate::ui::root::RootView;

pub fn run() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| {
                    if std::env::var("YTTT_DEV_FIXTURE").as_deref() == Ok("1") {
                        RootView::dev_fixture()
                    } else {
                        RootView::new()
                    }
                })
            },
        )
        .expect("failed to open yttt window");
    });
}
