use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};

use crate::ui::{
    actions::default_ui_keybindings,
    root::RootView,
    startup::{StartupMode, startup_mode_from_fixture},
};

pub fn run() {
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        cx.bind_keys(default_ui_keybindings());

        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| {
                    match startup_mode_from_fixture(
                        std::env::var("YTTT_DEV_FIXTURE").ok().as_deref(),
                    ) {
                        StartupMode::DevFixture => RootView::dev_fixture(),
                        StartupMode::AgentExitFixture => RootView::agent_exit_fixture(),
                        StartupMode::Normal => RootView::from_startup_env(),
                    }
                })
            },
        )
        .expect("failed to open yttt window");
    });
}
