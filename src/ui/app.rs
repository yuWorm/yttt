use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, IntoElement, Render,
    Window, WindowBounds, WindowOptions,
};

pub fn run() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| AppRoot),
        )
        .expect("failed to open yttt window");
    });
}

struct AppRoot;

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size(px(500.0))
            .justify_center()
            .items_center()
            .bg(rgb(0x101010))
            .text_color(rgb(0xf5f5f5))
            .child("yttt")
            .child("Open a project to start.")
    }
}
