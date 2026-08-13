pub mod assets;
pub mod platform;
pub mod startup;

use std::{rc::Rc, sync::Arc};

use gpui::{
    App, AppContext, Bounds, Entity, Pixels, QuitMode, Styled, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, px, size, transparent_black,
};
#[cfg(feature = "perf-metrics")]
use gpui::{IntoElement, ParentElement, Render, WindowKind, div, point};
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
    let terminal_performance_mode =
        cfg!(feature = "perf-metrics") && std::env::var_os("YTTT_TERMINAL_PERF_OUTPUT").is_some();
    let host_runtime = if startup_mode == StartupMode::Normal {
        HostRuntimeGlobal::ready(
            DesktopHostRuntime::start(profile.clone())
                .expect("failed to start the authoritative Host runtime"),
        )
    } else {
        HostRuntimeGlobal::unavailable("Host runtime is disabled for the selected UI fixture")
    };
    let mut application = gpui_platform::application().with_quit_mode(QuitMode::LastWindowClosed);
    if !terminal_performance_mode {
        let http_client = ReqwestClient::user_agent(concat!("yttt/", env!("CARGO_PKG_VERSION")))
            .expect("failed to initialize HTTP client");
        application = application
            .with_http_client(Arc::new(http_client))
            .with_assets(assets::app_assets(&config_paths));
    }
    application.run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        platform::macos::prepare_macos_app_runtime();

        cx.set_global(host_runtime.clone());
        yttt_terminal::init(cx);
        #[cfg(feature = "perf-metrics")]
        if terminal_performance_mode {
            open_terminal_performance_window(cx);
            return;
        }

        gpui_component::init(cx);
        cx.bind_keys(gpui_markdown_editor::default_key_bindings());
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
                if std::env::var_os("YTTT_TERMINAL_PERF_OUTPUT").is_some() {
                    cx.activate(true);
                    window.activate_window();
                }
                cx.new(|cx| ComponentRoot::new(view, window, cx).bg(transparent_black()))
            },
        )
        .expect("failed to open yttt window");
    });
}

#[cfg(feature = "perf-metrics")]
fn open_terminal_performance_window(cx: &mut App) {
    let contexts = terminal_performance_contexts();
    let terminal_config = yttt_terminal::TerminalConfig {
        scrollback: 10_000,
        ..yttt_terminal::TerminalConfig::default()
    };
    let terminal_theme = crate::ui::theme::WorkbenchTheme::one_dark();
    let bounds = Bounds::new(point(px(100.0), px(100.0)), size(px(1024.0), px(768.0)));
    let mut window_options = workbench_window_options(bounds, WindowBackgroundEffect::None);
    window_options.kind = WindowKind::PopUp;
    cx.open_window(window_options, move |window, cx| {
        cx.activate(true);
        window.activate_window();
        let panes = contexts
            .into_iter()
            .map(|context| {
                cx.new(|cx| {
                    crate::ui::terminal::pane::TerminalPaneView::new(
                        context,
                        terminal_config.clone(),
                        terminal_theme,
                        window,
                        cx,
                    )
                })
            })
            .collect::<Vec<_>>();
        let primary = panes
            .first()
            .expect("terminal performance layout requires pane 'perf'");
        let terminal_view = primary
            .read(cx)
            .performance_terminal()
            .expect("Host performance terminal must be initialized");
        primary.update(cx, |terminal, cx| {
            terminal.focus_terminal(window, cx);
        });
        cx.new(|_| TerminalPerformanceRoot {
            _panes: panes,
            terminal: terminal_view,
        })
    })
    .expect("failed to open Host terminal performance window");
}

#[cfg(feature = "perf-metrics")]
struct TerminalPerformanceRoot {
    _panes: Vec<Entity<crate::ui::terminal::pane::TerminalPaneView>>,
    terminal: Entity<yttt_terminal::TerminalView>,
}

#[cfg(feature = "perf-metrics")]
impl Render for TerminalPerformanceRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().flex().size_full().child(self.terminal.clone())
    }
}

#[cfg(feature = "perf-metrics")]
fn terminal_performance_contexts() -> Vec<crate::ui::terminal::pane::TerminalPaneContext> {
    use crate::{
        model::{
            ids::ProjectId,
            layout::{LayoutNode, PaneConfig, ProjectLayout},
        },
        ui::interaction::input_owner::TerminalInputGate,
    };

    let project_path = startup::startup_project_paths()
        .into_iter()
        .next()
        .expect("terminal performance mode requires --project");
    let layout_path = project_path.join(".yttt/layout.toml");
    let source = std::fs::read_to_string(&layout_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", layout_path.display()));
    let layout: ProjectLayout = toml::from_str(&source)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", layout_path.display()));
    let tab = layout
        .project
        .default_tab
        .as_deref()
        .and_then(|tab_id| layout.tab(tab_id))
        .or_else(|| layout.tabs.first())
        .expect("terminal performance layout requires a tab");
    fn collect_panes(node: &LayoutNode, panes: &mut Vec<PaneConfig>) {
        match node {
            LayoutNode::Pane(pane) => panes.push(pane.clone()),
            LayoutNode::Split(split) => {
                collect_panes(&split.left, panes);
                collect_panes(&split.right, panes);
            }
        }
    }
    let mut panes = Vec::new();
    collect_panes(&tab.layout, &mut panes);
    panes.sort_by_key(|pane| pane.id != "perf");
    assert!(
        panes.first().is_some_and(|pane| pane.id == "perf"),
        "terminal performance layout requires pane 'perf'"
    );
    let project_id =
        ProjectId::from_legacy_location(&project_path.display().to_string()).to_string();

    let project_title = layout.project.name.clone();
    let tab_id = tab.id.clone();
    let tab_title = tab.title.clone();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let environment = Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
    panes
        .into_iter()
        .map(|pane| {
            let is_focused = pane.id == "perf";
            crate::ui::terminal::pane::TerminalPaneContext {
                project_id: project_id.clone(),
                project_path: project_path.clone(),
                project_title: project_title.clone(),
                tab_id: tab_id.clone(),
                tab_title: tab_title.clone(),
                pane,
                shell: shell.clone(),
                environment: environment.clone(),
                is_focused,
                terminal_input_gate: TerminalInputGate::default(),
                ssh: None,
                agent_launch: None,
            }
        })
        .collect()
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
