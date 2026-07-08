use gpui::{App, AppContext, Application, Bounds, Pixels, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root as ComponentRoot, TitleBar};

use crate::{
    commands::default_registry,
    config::paths::AppConfigPaths,
    ui::{
        actions::load_app_keybindings,
        root::RootView,
        startup::{StartupMode, startup_mode_from_fixture},
    },
};

pub fn run() {
    Application::new()
        .with_assets(crate::ui::assets::app_assets())
        .run(|cx: &mut App| {
            #[cfg(target_os = "macos")]
            crate::ui::macos::prepare_macos_app_runtime();

            gpui_component::init(cx);
            let config_paths = AppConfigPaths::for_app();
            let command_registry = default_registry();
            cx.bind_keys(load_app_keybindings(&config_paths, &command_registry));

            let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
            cx.open_window(workbench_window_options(bounds), |window, cx| {
                let view = cx.new(|_| {
                    match startup_mode_from_fixture(
                        std::env::var("YTTT_DEV_FIXTURE").ok().as_deref(),
                    ) {
                        StartupMode::DevFixture => RootView::dev_fixture(),
                        StartupMode::AgentExitFixture => RootView::agent_exit_fixture(),
                        StartupMode::Normal => RootView::from_startup_env(),
                    }
                });
                cx.new(|cx| ComponentRoot::new(view, window, cx))
            })
            .expect("failed to open yttt window");
        });
}

pub fn workbench_window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitleBar::title_bar_options()),
        ..Default::default()
    }
}
