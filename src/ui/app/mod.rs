pub mod assets;
pub(crate) mod existing_host;
pub mod platform;
mod remote_connect;
pub mod startup;

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use gpui::{
    App, AppContext, Bounds, Entity, Global, Pixels, QuitMode, Styled, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, px, size, transparent_black,
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
    desktop_shell::{DesktopShellCommand, DesktopShellRuntime},
    desktop_tray::{DesktopTrayAction, DesktopTrayAdapter, DesktopTrayStatus, create_desktop_tray},
    host_runtime::HostRuntimeGlobal,
    login_startup::LoginStartupManager,
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
use yttt_protocol::{LifecycleRequest, LifecycleResponse};

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

pub fn run(
    profile: AppProfile,
    desktop_shell: Arc<DesktopShellRuntime>,
    initial_command: DesktopShellCommand,
) {
    let config_paths = profile.config_paths();
    let startup_mode = startup_mode_from_fixture(std::env::var("YTTT_DEV_FIXTURE").ok().as_deref());
    let terminal_performance_mode =
        cfg!(feature = "perf-metrics") && std::env::var_os("YTTT_TERMINAL_PERF_OUTPUT").is_some();
    let host_runtime = if startup_mode == StartupMode::Normal {
        HostRuntimeGlobal::start(profile.clone())
    } else {
        HostRuntimeGlobal::disabled()
    };
    if let Some(runtime) = host_runtime.runtime() {
        if let Err(error) = crate::config::storage::bind_environment(runtime.environment_storage())
        {
            eprintln!("failed to bind Host environment storage: {error}");
            return;
        }
    } else if startup_mode != StartupMode::Normal {
        crate::config::storage::allow_test_root(&profile.paths().config);
        crate::config::storage::allow_test_root(&profile.paths().state);
    }
    let (app_settings, theme_runtime) = load_app_runtime(&config_paths);
    let mut application =
        gpui_platform::application().with_quit_mode(desktop_quit_mode(terminal_performance_mode));
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
        let appearance = AppearanceState::new(theme_runtime);
        Theme::global_mut(cx).apply_config(&Rc::new(
            appearance.runtime().to_gpui_component_theme_config(),
        ));
        cx.set_global(appearance.clone());
        let command_registry = bindable_registry();
        cx.bind_keys(load_app_keybindings(&config_paths, &command_registry));

        let login_startup = LoginStartupManager::for_current_platform(&profile).ok();
        if let Some(manager) = login_startup.clone() {
            cx.background_spawn(async move {
                if let Err(error) = manager.reconcile()
                    && !matches!(
                        error,
                        crate::login_startup::LoginStartupError::UnsupportedProfile
                    )
                {
                    eprintln!("failed to reconcile login startup registration: {error}");
                }
            })
            .detach();
        }
        let window_context = DesktopWindowContext {
            profile,
            config_paths,
            app_settings,
            login_startup,
            appearance,
            startup_mode,
            workbenches: Rc::new(RefCell::new(Vec::new())),
        };
        install_desktop_tray(window_context.clone(), cx);
        let restore_windows = matches!(&initial_command, DesktopShellCommand::Activate);
        if let Err(error) = handle_desktop_shell_command(initial_command, &window_context, cx) {
            eprintln!("failed to open initial yttt window: {error}");
        }
        if restore_windows {
            while cx
                .global::<HostRuntimeGlobal>()
                .runtime()
                .is_some_and(|runtime| runtime.pending_workspace_count() > 0)
            {
                if let Err(error) =
                    open_workbench_window(&window_context, WindowIntent::Restore, cx)
                {
                    eprintln!("failed to restore work window: {error}");
                    break;
                }
            }
        }
        start_desktop_shell_listener(desktop_shell, window_context, cx);
    });
}

pub fn run_remote(launch: crate::remote_launch::RemoteLaunch) {
    let profile = launch.local_profile.clone();
    let assets = assets::app_assets(&profile.config_paths());
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .with_assets(assets)
        .run(move |cx| {
            #[cfg(target_os = "macos")]
            platform::macos::prepare_macos_app_runtime();
            gpui_component::init(cx);
            yttt_terminal::init(cx);
            crate::ui::editor::register_builtin_editor_languages();
            crate::ui::editor::init_vim_mode(cx);
            cx.set_global(HostRuntimeGlobal::disabled());
            if let Err(error) = remote_connect::open(
                launch,
                move |remote, cx| {
                    let workspace_count = remote.runtime.pending_workspace_count().max(1);
                    cx.set_global(HostRuntimeGlobal::ready(remote.runtime));
                    let (app_settings, theme_runtime) = load_app_runtime(&remote.config_paths);
                    let appearance = AppearanceState::new(theme_runtime);
                    Theme::global_mut(cx).apply_config(&Rc::new(
                        appearance.runtime().to_gpui_component_theme_config(),
                    ));
                    cx.set_global(appearance.clone());
                    rebind_application_keybindings(
                        cx,
                        &crate::config::keybindings::load_keybindings(
                            &remote.config_paths,
                            &bindable_registry(),
                        )
                        .map(|loaded| loaded.config)
                        .unwrap_or_default(),
                    );
                    let context = DesktopWindowContext {
                        profile,
                        config_paths: remote.config_paths,
                        app_settings,
                        login_startup: None,
                        appearance,
                        startup_mode: StartupMode::Normal,
                        workbenches: Rc::new(RefCell::new(Vec::new())),
                    };
                    for _ in 0..workspace_count {
                        if let Err(error) =
                            open_workbench_window(&context, WindowIntent::Restore, cx)
                        {
                            eprintln!("failed to open remote workspace: {error}");
                            break;
                        }
                    }
                    for window in cx.windows() {
                        let _ = window.update(cx, |_, window, _| {
                            window.set_window_title(&format!("yttt — {}", remote.label))
                        });
                    }
                },
                cx,
            ) {
                eprintln!("failed to open remote connection window: {error}");
                cx.quit();
            }
        });
}

#[derive(Clone)]
struct DesktopWindowContext {
    profile: AppProfile,

    config_paths: AppConfigPaths,
    app_settings: AppSettings,
    login_startup: Option<LoginStartupManager>,
    appearance: AppearanceState,
    startup_mode: StartupMode,
    workbenches: Rc<RefCell<Vec<WeakEntity<WorkbenchView>>>>,
}
fn desktop_quit_mode(terminal_performance_mode: bool) -> QuitMode {
    if terminal_performance_mode {
        QuitMode::LastWindowClosed
    } else {
        QuitMode::Explicit
    }
}

fn start_desktop_shell_listener(
    desktop_shell: Arc<DesktopShellRuntime>,
    window_context: DesktopWindowContext,
    cx: &mut App,
) {
    let commands = desktop_shell.commands();
    cx.spawn(async move |cx| {
        let _desktop_shell = desktop_shell;
        while let Ok(command) = commands.recv_async().await {
            let result = cx.update(|cx| handle_desktop_shell_command(command, &window_context, cx));
            if let Err(error) = result {
                eprintln!("failed to handle desktop shell command: {error}");
            }
        }
    })
    .detach();
}

fn handle_desktop_shell_command(
    command: DesktopShellCommand,
    window_context: &DesktopWindowContext,
    cx: &mut App,
) -> anyhow::Result<()> {
    match command {
        DesktopShellCommand::Activate => {
            if !activate_workbench_window(cx) {
                open_workbench_window(window_context, WindowIntent::Restore, cx)?;
            }
        }
        DesktopShellCommand::OpenWindow { project_paths } => {
            let intent = if project_paths.is_empty() {
                WindowIntent::Empty
            } else {
                WindowIntent::OpenProjects(project_paths)
            };
            open_workbench_window(window_context, intent, cx)?;
        }
    }
    Ok(())
}

fn activate_workbench_window(cx: &mut App) -> bool {
    let window = cx
        .window_stack()
        .and_then(|windows| windows.into_iter().next())
        .or_else(|| cx.windows().into_iter().next());
    let Some(window) = window else {
        return false;
    };
    cx.activate(true);
    window
        .update(cx, |_, window, _| window.activate_window())
        .is_ok()
}

enum WindowIntent {
    Restore,
    Empty,
    OpenProjects(Vec<PathBuf>),
}

fn open_workbench_window(
    window_context: &DesktopWindowContext,
    intent: WindowIntent,
    cx: &mut App,
) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
    let config_paths = window_context.config_paths.clone();
    let appearance = window_context.appearance.clone();
    let startup_mode = window_context.startup_mode;
    let login_startup = window_context.login_startup.clone();
    let workbenches = window_context.workbenches.clone();
    let restore_existing = matches!(intent, WindowIntent::Restore);
    let has_host_snapshot = cx
        .global::<HostRuntimeGlobal>()
        .runtime()
        .is_some_and(|runtime| runtime.pending_workspace_count() > 0 || runtime.is_remote());
    let should_check_for_updates =
        startup_mode == StartupMode::Normal && !crate::config::storage::is_remote();
    cx.open_window(
        workbench_window_options(bounds, window_context.app_settings.window.effect),
        move |window, cx| {
            let view = cx.new(|_| {
                let force_onboarding =
                    force_onboarding_from_env(std::env::var(FORCE_ONBOARDING_ENV).ok().as_deref());
                let view = match startup_mode {
                    StartupMode::DevFixture => WorkbenchView::dev_fixture(),
                    StartupMode::AgentExitFixture => WorkbenchView::agent_exit_fixture(),
                    StartupMode::Normal => match intent {
                        WindowIntent::OpenProjects(paths) => WorkbenchView::from_project_paths(
                            config_paths.clone(),
                            force_onboarding,
                            paths,
                        ),
                        WindowIntent::Empty => WorkbenchView::from_project_paths(
                            config_paths.clone(),
                            force_onboarding,
                            Vec::new(),
                        ),
                        WindowIntent::Restore if has_host_snapshot => {
                            WorkbenchView::from_project_paths(
                                config_paths.clone(),
                                false,
                                Vec::new(),
                            )
                        }
                        WindowIntent::Restore => {
                            WorkbenchView::from_startup(config_paths.clone(), force_onboarding)
                        }
                    },
                };
                let view = match login_startup.clone() {
                    Some(manager) => view.with_login_startup(manager),
                    None => view,
                };
                view.with_appearance_state(appearance)
            });
            workbenches.borrow_mut().push(view.downgrade());
            let host_runtime = cx.global::<HostRuntimeGlobal>().clone();
            view.update(cx, |view, _cx| {
                view.set_host_runtime_status(&host_runtime);
            });
            view.update(cx, |view, cx| {
                view.start_workspace_persistence(restore_existing, window, cx)
            });
            view.update(cx, |view, cx| view.sync_performance_monitoring(cx));
            view.update(cx, |view, cx| view.start_ssh_event_listener(cx));
            if should_check_for_updates {
                view.update(cx, |view, cx| view.start_update_check(window, cx));
            }
            register_workbench_keybinding_interceptor(cx, &view);
            register_workbench_focus_restore(window, cx, &view);
            register_workbench_close_guard(window, cx, &view);
            cx.activate(true);
            window.activate_window();
            cx.new(|cx| ComponentRoot::new(view, window, cx).bg(transparent_black()))
        },
    )?;
    Ok(())
}

struct DesktopTrayGlobal {
    adapter: Box<dyn DesktopTrayAdapter>,
}

impl Global for DesktopTrayGlobal {}

fn install_desktop_tray(window_context: DesktopWindowContext, cx: &mut App) {
    let tray = match create_desktop_tray() {
        Ok(tray) => tray,
        Err(error) => {
            eprintln!("{error}; use the Host lifecycle CLI commands instead");
            return;
        }
    };
    if !tray.is_available() {
        return;
    }
    tray.update(&DesktopTrayStatus::unavailable("Connecting"));
    let actions = tray.actions();
    cx.set_global(DesktopTrayGlobal { adapter: tray });
    start_desktop_tray_actions(actions, window_context, cx);
    start_desktop_tray_status_monitor(cx);
}

fn start_desktop_tray_actions(
    actions: flume::Receiver<DesktopTrayAction>,
    window_context: DesktopWindowContext,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Ok(action) = actions.recv_async().await {
            let result = cx.update(|cx| handle_desktop_tray_action(action, &window_context, cx));
            if let Err(error) = result {
                eprintln!("desktop tray action failed: {error}");
            }
        }
    })
    .detach();
}

fn handle_desktop_tray_action(
    action: DesktopTrayAction,
    window_context: &DesktopWindowContext,
    cx: &mut App,
) -> anyhow::Result<()> {
    match action {
        DesktopTrayAction::Open => {
            handle_desktop_shell_command(DesktopShellCommand::Activate, window_context, cx)?;
        }
        DesktopTrayAction::NewWindow => {
            handle_desktop_shell_command(
                DesktopShellCommand::OpenWindow {
                    project_paths: Vec::new(),
                },
                window_context,
                cx,
            )?;
        }
        DesktopTrayAction::OpenLogs => {
            std::fs::create_dir_all(&window_context.profile.paths().logs)?;
            platform::reveal_path(&window_context.profile.paths().logs)?;
        }
        DesktopTrayAction::StartHost => {
            start_host_runtime(window_context.clone(), cx);
        }
        DesktopTrayAction::StopHost => {
            request_host_stop(false, window_context.clone(), cx);
        }
        DesktopTrayAction::RestartHost => {
            request_host_stop(true, window_context.clone(), cx);
        }
        DesktopTrayAction::QuitDesktop | DesktopTrayAction::QuitAll => {
            if cx.windows().is_empty() {
                open_workbench_window(window_context, WindowIntent::Restore, cx)?;
            }
            if let Some(window) = cx.windows().first().copied() {
                window.update(cx, |_, window, cx| {
                    confirm_desktop_quit(matches!(action, DesktopTrayAction::QuitAll), window, cx)
                })?;
            }
        }
    }
    Ok(())
}

fn start_host_runtime(window_context: DesktopWindowContext, cx: &mut App) {
    let profile = window_context.profile.clone();
    let start = cx
        .background_executor()
        .spawn(async move { HostRuntimeGlobal::start(profile) });
    cx.spawn(async move |cx| {
        let status = start.await;
        cx.update(|cx| replace_host_runtime(status, &window_context, cx));
    })
    .detach();
}

fn request_host_stop(restart: bool, window_context: DesktopWindowContext, cx: &mut App) {
    let Some(runtime) = cx.global::<HostRuntimeGlobal>().runtime().cloned() else {
        if restart {
            start_host_runtime(window_context, cx);
        }
        return;
    };
    let response = runtime.request_lifecycle(LifecycleRequest::StopIfIdle, false);
    cx.spawn(async move |cx| match response.recv_async().await {
        Ok(Ok(LifecycleResponse::Stopping)) => {
            runtime.shutdown_client();
            if restart {
                let profile = window_context.profile.clone();
                let start = cx
                    .background_executor()
                    .spawn(async move { HostRuntimeGlobal::start(profile) });
                let status = start.await;
                cx.update(|cx| replace_host_runtime(status, &window_context, cx));
            } else {
                cx.update(|cx| {
                    replace_host_runtime(
                        HostRuntimeGlobal::unavailable("Host stopped"),
                        &window_context,
                        cx,
                    )
                });
            }
        }
        Ok(Ok(LifecycleResponse::Busy { blockers })) => {
            eprintln!(
                "Host remains running because {} lifecycle blockers are active",
                blockers.len()
            );
        }
        Ok(Ok(other)) => eprintln!("unexpected Host lifecycle response: {other:?}"),
        Ok(Err(error)) => eprintln!("Host lifecycle request failed: {error}"),
        Err(error) => eprintln!("Host lifecycle response channel failed: {error}"),
    })
    .detach();
}

pub(crate) fn confirm_desktop_quit(stop_all: bool, window: &mut Window, cx: &mut App) {
    let Some(runtime) = cx.global::<HostRuntimeGlobal>().runtime().cloned() else {
        cx.quit();
        return;
    };
    if runtime.preparing_transfer().is_some() {
        return;
    }
    let stops_host = stop_all || runtime.actual_lifetime() == yttt_host::HostLifetime::DesktopOwned;
    let answer = window.prompt(gpui::PromptLevel::Warning, "Quit yttt?",
        Some(if stops_host { "This Host is owned by the desktop. Quitting stops its network listener, disconnects remote clients, and terminates running terminals and Agent tasks. Confirmed workspaces and drafts remain saved." }
            else { "This independently running Host will keep its network listener, remote clients, terminals and Agent tasks. Only this desktop client will disconnect." }),
        &["Cancel", if stops_host { "Stop Host and quit" } else { "Disconnect desktop" }], cx);
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| {
        if !matches!(answer.await, Ok(1)) { return; }
        let transfer = runtime.begin_exit_publication();
        cx.update(|cx| cx.refresh_windows());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let publication = loop {
            if !runtime.is_controller() { break Ok(()); }
            match runtime.exit_publication_ready(&transfer) {
                Ok(true) => break Ok(()),
                Err(error) => break Err(error),
                Ok(false) if std::time::Instant::now() >= deadline => break Err("Some work windows could not finish publishing. Keep them open and resolve their save errors before quitting.".to_string()),
                Ok(false) => cx.background_executor().timer(Duration::from_millis(50)).await,
            }
        };
        if let Err(error) = publication {
            runtime.cancel_exit_publication();
            cx.update(|cx| {
                let _ = window_handle.update(cx, |_, window, cx| {
                    let _ = window.prompt(gpui::PromptLevel::Warning, "Desktop remains open", Some(&error), &["OK"], cx);
                });
                cx.refresh_windows();
            });
            return;
        }
        if stops_host {
            let result = runtime.request_lifecycle(LifecycleRequest::ForceStop, true).recv_async().await;
            if !matches!(result, Ok(Ok(LifecycleResponse::Draining))) {
                runtime.cancel_exit_publication();
                cx.update(|cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = window.prompt(gpui::PromptLevel::Warning, "Host stop failed", Some("The desktop remains connected. Retry after checking Host status."), &["OK"], cx);
                    });
                });
                return;
            }
        }
        cx.background_executor().spawn(async move { runtime.shutdown_client(); }).await;
        cx.update(|cx| cx.quit());
    }).detach();
}

fn replace_host_runtime(
    status: HostRuntimeGlobal,
    window_context: &DesktopWindowContext,
    cx: &mut App,
) {
    if let Some(runtime) = status.runtime()
        && let Err(error) = crate::config::storage::bind_environment(runtime.environment_storage())
    {
        eprintln!("failed to bind replacement Host environment: {error}");
        return;
    }
    cx.set_global(status.clone());
    let workbenches = window_context
        .workbenches
        .borrow()
        .iter()
        .filter_map(WeakEntity::upgrade)
        .collect::<Vec<_>>();
    window_context
        .workbenches
        .borrow_mut()
        .retain(|workbench| workbench.upgrade().is_some());
    for workbench in workbenches {
        workbench.update(cx, |workbench, cx| {
            workbench.set_host_runtime_status(&status);
            workbench.start_ssh_event_listener(cx);
            workbench.sync_performance_monitoring(cx);
        });
    }
}

fn start_desktop_tray_status_monitor(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let response = cx.update(|cx| {
                cx.global::<HostRuntimeGlobal>()
                    .runtime()
                    .map(|runtime| runtime.request_lifecycle(LifecycleRequest::Status, false))
            });
            let status = match response {
                Some(response) => match response.recv_async().await {
                    Ok(Ok(LifecycleResponse::Status(status))) => {
                        DesktopTrayStatus::from_lifecycle(&status)
                    }
                    Ok(Ok(other)) => {
                        DesktopTrayStatus::unavailable(format!("Unexpected response: {other:?}"))
                    }
                    Ok(Err(error)) => DesktopTrayStatus::unavailable(error.to_string()),
                    Err(error) => DesktopTrayStatus::unavailable(error.to_string()),
                },
                None => DesktopTrayStatus::unavailable("Stopped"),
            };
            cx.update(|cx| {
                cx.global::<DesktopTrayGlobal>().adapter.update(&status);
            });
            cx.background_executor().timer(Duration::from_secs(2)).await;
        }
    })
    .detach();
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
    window.on_window_should_close(cx, move |window, cx| {
        view.update(cx, |root, cx| {
            if crate::config::storage::is_remote() {
                root.flush_workspace_persistence_on_close(window, cx)
            } else {
                root.request_window_close(cx)
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_desktop_remains_running_after_the_last_window_closes() {
        assert_eq!(desktop_quit_mode(false), QuitMode::Explicit);
        assert_eq!(desktop_quit_mode(true), QuitMode::LastWindowClosed);
    }
}
