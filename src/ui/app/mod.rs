pub mod assets;
pub mod platform;
pub mod startup;

use std::{rc::Rc, sync::Arc};

use gpui::{
    App, AppContext, Bounds, Entity, Pixels, QuitMode, Styled, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, px, size, transparent_black,
};
use gpui_component::{Root as ComponentRoot, Theme, TitleBar};
use reqwest_client::ReqwestClient;

use crate::{
    config::{
        paths::AppConfigPaths,
        profile::AppProfile,
        settings::{AppSettings, WindowBackgroundEffect, load_or_create_settings},
        theme::{ThemeStore, load_theme_store},
    },
    host_runtime::{DesktopHostRuntime, HostRuntimeGlobal},
    ui::{
        app::startup::{
            FORCE_ONBOARDING_ENV, StartupMode, force_onboarding_from_env, startup_mode_from_fixture,
        },
        interaction::actions::{
            bindable_registry, compiled_app_keybindings, load_app_keybindings,
            ui_action_for_command,
        },
        theme::{AppearanceState, ThemeRuntime},
        workbench::WorkbenchView,
    },
};

pub(crate) fn rebind_application_keybindings(
    cx: &mut App,
    config: &crate::config::keybindings::KeybindingsConfig,
) {
    cx.clear_key_bindings();
    gpui_component::rebind_keybindings(cx);
    cx.bind_keys(gpui_markdown_editor::default_key_bindings());
    let registry = bindable_registry();
    cx.bind_keys(compiled_app_keybindings(config, &registry));
}

pub fn run(profile: AppProfile) {
    let config_paths = profile.config_paths();
    let startup_mode = startup_mode_from_fixture(std::env::var("YTTT_DEV_FIXTURE").ok().as_deref());
    let host_runtime = if startup_mode == StartupMode::Normal {
        HostRuntimeGlobal::ready(
            DesktopHostRuntime::start(profile.clone())
                .expect("failed to start the authoritative Host runtime"),
        )
    } else {
        HostRuntimeGlobal::unavailable("Host runtime is disabled for the selected UI fixture")
    };
    let http_client = ReqwestClient::user_agent(concat!("yttt/", env!("CARGO_PKG_VERSION")))
        .expect("failed to initialize HTTP client");
    gpui_platform::application()
        .with_http_client(Arc::new(http_client))
        .with_assets(assets::app_assets(&config_paths))
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            #[cfg(target_os = "macos")]
            platform::macos::prepare_macos_app_runtime();

            gpui_component::init(cx);
            cx.set_global(host_runtime.clone());
            cx.bind_keys(gpui_markdown_editor::default_key_bindings());
            yttt_terminal::init(cx);
            crate::ui::editor::register_builtin_editor_languages();
            crate::ui::editor::init_vim_mode(cx);
            let config_paths = config_paths.clone();
            let (app_settings, theme_runtime) = load_app_runtime(&config_paths);
            let appearance = AppearanceState::new(theme_runtime);
            Theme::global_mut(cx).apply_config(&Rc::new(
                appearance.runtime().to_gpui_component_theme_config(),
            ));
            cx.set_global(appearance.clone());
            let command_registry = bindable_registry();
            cx.bind_keys(load_app_keybindings(&config_paths, &command_registry));

            let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
            cx.open_window(
                workbench_window_options(bounds, app_settings.window.effect),
                move |window, cx| {
                    let appearance = appearance.clone();
                    let startup_mode = startup_mode;
                    let should_check_for_updates = startup_mode == StartupMode::Normal;
                    let view = cx.new(|_| {
                        let force_onboarding = force_onboarding_from_env(
                            std::env::var(FORCE_ONBOARDING_ENV).ok().as_deref(),
                        );
                        let view = match startup_mode {
                            StartupMode::DevFixture => WorkbenchView::dev_fixture(),
                            StartupMode::AgentExitFixture => WorkbenchView::agent_exit_fixture(),
                            StartupMode::Normal => {
                                WorkbenchView::from_startup(config_paths.clone(), force_onboarding)
                            }
                        };
                        view.with_appearance_state(appearance)
                    });
                    let desktop_host_runtime = cx.global::<HostRuntimeGlobal>().runtime().cloned();
                    view.update(cx, |view, _cx| {
                        view.set_host_runtime(desktop_host_runtime);
                    });
                    view.update(cx, |view, cx| view.sync_performance_monitoring(cx));
                    view.update(cx, |view, cx| view.start_ssh_event_listener(cx));
                    if should_check_for_updates {
                        view.update(cx, |view, cx| view.start_update_check(window, cx));
                    }
                    register_workbench_keybinding_interceptor(cx, &view);
                    register_workbench_focus_restore(window, cx, &view);
                    register_workbench_close_guard(window, cx, &view);
                    cx.new(|cx| ComponentRoot::new(view, window, cx).bg(transparent_black()))
                },
            )
            .expect("failed to open yttt window");
        });
}

pub fn register_workbench_keybinding_interceptor(cx: &mut App, view: &Entity<WorkbenchView>) {
    let runtime_keybinding_view = view.clone();
    let keybinding_subscription = cx.intercept_keystrokes(move |event, window, cx| {
        if window
            .pending_input_keystrokes()
            .is_some_and(|pending| !pending.is_empty())
        {
            return;
        }
        let command = runtime_keybinding_view
            .read(cx)
            .runtime_command_for_dispatch(&event.keystroke);
        if let Some(action) = command.and_then(ui_action_for_command) {
            window.dispatch_action(action, cx);
            cx.stop_propagation();
        }
    });
    view.update(cx, |root, _| {
        root.set_keybinding_interceptor_subscription(keybinding_subscription);
    });
}

pub fn register_workbench_focus_restore(
    window: &mut Window,
    cx: &mut App,
    view: &Entity<WorkbenchView>,
) {
    view.update(cx, |view, cx| {
        view.register_window_activation_observer(window, cx);
    });
}

pub fn register_workbench_close_guard(window: &Window, cx: &App, view: &Entity<WorkbenchView>) {
    let view = view.downgrade();
    window.on_window_should_close(cx, move |_window, cx| {
        view.update(cx, |root, cx| root.request_window_close(cx))
            .unwrap_or(true)
    });
}

fn load_app_runtime(config_paths: &AppConfigPaths) -> (AppSettings, ThemeRuntime) {
    let settings = load_or_create_settings(config_paths)
        .map(|loaded| loaded.settings)
        .unwrap_or_else(|_| AppSettings::default());
    let theme_store = load_theme_store(config_paths)
        .map(|loaded| loaded.store)
        .unwrap_or_else(|_| ThemeStore::builtin());
    let theme_runtime = ThemeRuntime::resolve(&settings, &theme_store);

    (settings, theme_runtime)
}

pub fn workbench_window_options(
    bounds: Bounds<Pixels>,
    effect: WindowBackgroundEffect,
) -> WindowOptions {
    let effect = platform::resolved_window_background_effect(effect);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(960.0), px(640.0))),
        window_background: window_background_appearance(effect),
        app_id: Some(platform::APP_ID.to_string()),
        titlebar: Some(TitleBar::title_bar_options()),
        ..Default::default()
    }
}

pub fn window_background_appearance(effect: WindowBackgroundEffect) -> WindowBackgroundAppearance {
    match effect {
        WindowBackgroundEffect::None => WindowBackgroundAppearance::Opaque,
        WindowBackgroundEffect::Transparent => WindowBackgroundAppearance::Transparent,
        WindowBackgroundEffect::Blurred => WindowBackgroundAppearance::Blurred,
    }
}
