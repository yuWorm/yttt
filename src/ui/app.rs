use std::rc::Rc;

use gpui::{App, AppContext, Application, Bounds, Pixels, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root as ComponentRoot, Theme, TitleBar};

use crate::{
    commands::default_registry,
    config::{
        paths::AppConfigPaths,
        settings::{AppSettings, load_or_create_settings},
        theme::{ThemeStore, load_theme_store},
    },
    ui::{
        actions::load_app_keybindings,
        root::RootView,
        startup::{StartupMode, startup_mode_from_fixture},
        theme::ThemeRuntime,
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
            let theme_runtime = load_app_theme_runtime(&config_paths);
            Theme::global_mut(cx)
                .apply_config(&Rc::new(theme_runtime.to_gpui_component_theme_config()));
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

fn load_app_theme_runtime(config_paths: &AppConfigPaths) -> ThemeRuntime {
    let settings = load_or_create_settings(config_paths)
        .map(|loaded| loaded.settings)
        .unwrap_or_else(|_| AppSettings::default());
    let theme_store = load_theme_store(config_paths)
        .map(|loaded| loaded.store)
        .unwrap_or_else(|_| ThemeStore::builtin());

    ThemeRuntime::resolve(&settings, &theme_store)
}

pub fn workbench_window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(720.0), px(520.0))),
        titlebar: Some(TitleBar::title_bar_options()),
        ..Default::default()
    }
}
