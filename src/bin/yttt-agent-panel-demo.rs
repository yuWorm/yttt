//! The agent panel driven by a scripted turn instead of an agent.
//!
//! Interactive by default: type a prompt (`@` mentions a sample file, `/` lists
//! commands) and the script plays thinking, tool calls, a permission prompt, a
//! diff and a test run. With `--screenshot` it submits a prompt itself, plays
//! fast, captures the window at `--capture` and exits.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use gpui::{
    App, AppContext as _, AsyncApp, Bounds, Entity, Render, Window, WindowBounds, WindowHandle,
    WindowOptions, point, prelude::*, px, size,
};
use yttt::{
    config::{paths::AppConfigPaths, settings::AppSettings, theme::ThemeStore},
    ui::{
        app::assets,
        editor::register_builtin_editor_languages,
        theme::{AppearanceState, ThemeRuntime},
    },
};
use yttt_agent_panel::{
    AgentPanel,
    demo::{DemoCandidates, DriverEvent, ScriptedDriver, configure, demo_prompt},
    event::{Prompt, PromptSegment},
};
use yttt_ui::style::UiStyleId;

const WINDOW_WIDTH: f32 = 560.;
const WINDOW_HEIGHT: f32 = 900.;
/// Lets syntax highlighting, markdown parsing and fades finish before capture.
const SETTLE_DELAY: Duration = Duration::from_millis(1500);
/// Lets a scrolled page lay out, and lazily parsed markdown finish.
const PAGE_DELAY: Duration = Duration::from_millis(700);
/// A little less than the thread's height, so pages overlap.
const PAGE_STEP: f32 = 640.;
const MAX_PAGES: usize = 12;
const USAGE: &str = "usage: yttt-agent-panel-demo [--style zed|rounded] [--speed N] [--open-cards] \
                     [--screenshot PATH [--capture permission|end|pages|empty|models|title] [--prompt TEXT]]";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Capture {
    /// While the edit waits for approval, with its diff and buttons showing.
    Permission,
    /// After the turn ends.
    #[default]
    End,
    /// After the turn ends, the whole thread a screen at a time from the
    /// top, as `NAME-1.png`, `NAME-2.png`, ….
    Pages,
    /// Before anything is sent: the empty thread and its suggestions.
    Empty,
    /// The model picker, open over the empty thread.
    Models,
    /// The header title being renamed.
    Title,
}

#[derive(Default)]
struct Options {
    style: Option<UiStyleId>,
    speed: Option<f32>,
    screenshot: Option<PathBuf>,
    capture: Capture,
    open_cards: bool,
    prompt: Option<String>,
}

fn main() -> ExitCode {
    match parse_options(std::env::args().skip(1)) {
        Ok(options) => {
            run(options);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("yttt-agent-panel-demo: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Options> {
    let mut options = Options::default();
    while let Some(argument) = args.next() {
        let mut value = || args.next().context(USAGE);
        match argument.as_str() {
            "--style" => {
                options.style = Some(match value()?.as_str() {
                    "zed" => UiStyleId::Zed,
                    "rounded" => UiStyleId::Rounded,
                    _ => bail!(USAGE),
                })
            }
            "--open-cards" => options.open_cards = true,
            "--prompt" => options.prompt = Some(value()?),
            "--speed" => options.speed = Some(value()?.parse().context(USAGE)?),
            "--screenshot" => options.screenshot = Some(value()?.into()),
            "--capture" => {
                options.capture = match value()?.as_str() {
                    "permission" => Capture::Permission,
                    "end" => Capture::End,
                    "pages" => Capture::Pages,
                    "empty" => Capture::Empty,
                    "models" => Capture::Models,
                    "title" => Capture::Title,
                    _ => bail!(USAGE),
                }
            }
            _ => bail!(USAGE),
        }
    }
    Ok(options)
}

/// Owns the driver so it lives as long as the window.
struct DemoRoot {
    panel: Entity<AgentPanel>,
    _driver: Entity<ScriptedDriver>,
}

impl Render for DemoRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.panel.clone()
    }
}

fn run(options: Options) {
    let mut settings = AppSettings::default();
    if let Some(style) = options.style {
        settings.theme.ui_style = style;
    }
    let runtime = ThemeRuntime::resolve(&settings, &ThemeStore::builtin());
    let config_paths =
        AppConfigPaths::from_config_dir(std::env::temp_dir().join("yttt-agent-panel-demo"));
    let unattended = options.screenshot.is_some();
    let speed = options.speed.unwrap_or(if unattended { 8. } else { 1. });
    let auto_allow = unattended && options.capture != Capture::Permission;
    let open_cards = options.open_cards;

    gpui_platform::application()
        .with_assets(assets::app_assets(&config_paths))
        .run(move |cx: &mut App| {
            yttt_ui::init(cx);
            register_builtin_editor_languages();
            assets::load_ui_fonts(cx);
            let appearance = AppearanceState::new(runtime);
            appearance.runtime().apply_component_theme(cx);
            let theme = appearance.runtime().ui;
            let ui_style = appearance.runtime().style;
            cx.set_global(appearance);

            let driver_slot = Rc::new(RefCell::new(None));
            let window_driver = driver_slot.clone();
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(40.), px(40.)),
                    size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
                ))),
                ..Default::default()
            };
            let handle = cx
                .open_window(window_options, move |window, cx| {
                    let panel = cx.new(|cx| {
                        AgentPanel::new(Rc::new(DemoCandidates), theme, ui_style, window, cx)
                    });
                    panel.update(cx, |panel, cx| {
                        configure(panel, theme, ui_style, cx);
                        panel.focus(window, cx);
                        if open_cards {
                            panel.set_cards_open(Some(true), cx);
                        }
                    });
                    let driver =
                        cx.new(|cx| ScriptedDriver::new(panel.clone(), speed, auto_allow, cx));
                    *window_driver.borrow_mut() = Some((driver.clone(), panel.clone()));
                    let root = cx.new(|_| DemoRoot {
                        panel,
                        _driver: driver,
                    });
                    cx.new(|cx| yttt_ui::window::yttt_root(root, window, cx))
                })
                .expect("demo window opens");
            cx.activate(true);

            let Some(path) = options.screenshot.clone() else {
                return;
            };
            let (driver, panel) = driver_slot
                .borrow_mut()
                .take()
                .expect("window built the driver");
            let capture = options.capture;
            let path_for_empty = path.clone();
            let panel_for_picker = panel.clone();
            cx.subscribe(&driver, move |_, event, cx| {
                let ready = matches!(
                    (capture, event),
                    (Capture::Permission, DriverEvent::AwaitingPermission)
                        | (Capture::End | Capture::Pages, DriverEvent::TurnFinished)
                );
                if !ready {
                    return;
                }
                let path = path.clone();
                let panel = panel.clone();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(SETTLE_DELAY).await;
                    let captured = match capture {
                        Capture::Pages => capture_pages(handle, &panel, &path, cx).await,
                        Capture::Permission
                        | Capture::End
                        | Capture::Empty
                        | Capture::Models
                        | Capture::Title => capture_window(handle, &path, cx),
                    };
                    if let Err(error) = captured {
                        eprintln!("yttt-agent-panel-demo: {error:#}");
                        std::process::exit(1);
                    }
                    std::process::exit(0);
                })
                .detach();
            })
            .detach();
            if matches!(capture, Capture::Empty | Capture::Models | Capture::Title) {
                let panel = panel_for_picker.clone();
                handle
                    .update(cx, |_, window, cx| {
                        panel.update(cx, |panel, cx| match capture {
                            Capture::Models => panel.open_model_picker(window, cx),
                            Capture::Title => panel.start_title_edit(window, cx),
                            _ => {}
                        })
                    })
                    .ok();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(SETTLE_DELAY).await;
                    let code = match capture_window(handle, &path_for_empty, cx) {
                        Ok(()) => 0,
                        Err(error) => {
                            eprintln!("yttt-agent-panel-demo: {error:#}");
                            1
                        }
                    };
                    std::process::exit(code);
                })
                .detach();
                return;
            }
            let prompt = match options.prompt.clone() {
                Some(text) => Prompt {
                    segments: vec![PromptSegment::Text(text)],
                    attachments: Vec::new(),
                },
                None => demo_prompt(),
            };
            driver.update(cx, |driver, cx| driver.start(prompt, None, cx));
        });
}

fn capture_window(
    handle: WindowHandle<gpui_base::Root>,
    path: &Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    handle.update(cx, |_, window, _| {
        window
            .render_to_image()
            .context("render demo to image")?
            .save(path)
            .with_context(|| format!("write {}", path.display()))
    })?
}

async fn capture_pages(
    handle: WindowHandle<gpui_base::Root>,
    panel: &Entity<AgentPanel>,
    path: &Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    panel.update(cx, |panel, cx| panel.scroll_thread_to_top(cx));
    for page in 1..=MAX_PAGES {
        cx.background_executor().timer(PAGE_DELAY).await;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let extension = path.extension().unwrap_or_default().to_string_lossy();
        capture_window(
            handle,
            &path.with_file_name(format!("{stem}-{page}.{extension}")),
            cx,
        )?;
        if !panel.update(cx, |panel, cx| panel.scroll_thread_down(px(PAGE_STEP), cx)) {
            break;
        }
    }
    Ok(())
}
