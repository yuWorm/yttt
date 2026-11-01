//! Deterministic gallery of `yttt-ui` primitives for visual baselines.
//!
//! Settings and themes come from built-in defaults, never the user's config.
//! Interactive states are forced from the public style structs, so the gallery
//! shows what a hovered or pressed primitive paints without synthesizing input.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::{
    cell::RefCell, collections::BTreeMap, path::PathBuf, process::ExitCode, rc::Rc, time::Duration,
};

use anyhow::{Context as _, Result, bail};
use gpui::{
    AnyElement, App, Bounds, Div, Entity, Pixels, Render, SharedString, Window, WindowBounds,
    WindowOptions, canvas, div, point, prelude::*, px, rems, size,
};
use gpui_component::{IndexPath, Theme, input::InputState, menu::PopupMenu, select::SelectState};
use yttt::{
    config::{paths::AppConfigPaths, settings::AppSettings, theme::ThemeStore},
    ui::{
        app::assets,
        theme::{AppearanceState, ThemeRuntime},
    },
};
use yttt_ui::IconName;
use yttt_ui::{
    SelectableState,
    menu::{YtttMenu, YtttMenuItem, yttt_menu_preview},
    primitives::{
        button::{YtttButtonVariant, yttt_button, yttt_button_style},
        dialog::{yttt_dialog_style, yttt_dialog_surface},
        icon_button::{YtttIconButtonKind, yttt_icon_button},
        input::{YtttInputKind, yttt_input},
        notification::{YtttNotificationTone, yttt_notification_surface},
        radio::yttt_radio,
        row::{YtttRowKind, yttt_row},
        select::yttt_select,
        switch::yttt_switch,
        tabs::yttt_tab,
        tooltip::yttt_tooltip,
    },
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

const WINDOW_WIDTH: f32 = 960.;
const WINDOW_HEIGHT: f32 = 800.;
const SETTLE_DELAY: Duration = Duration::from_millis(1500);
const USAGE: &str = "usage: yttt-ui-gallery [--style zed|rounded] \
                     [--screenshot PATH] [--dump-layout PATH]";

type LayoutMap = Rc<RefCell<BTreeMap<String, Bounds<Pixels>>>>;

#[derive(Default)]
struct Options {
    style: Option<UiStyleId>,
    screenshot: Option<PathBuf>,
    dump_layout: Option<PathBuf>,
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
            "--screenshot" => options.screenshot = Some(value()?.into()),
            "--dump-layout" => options.dump_layout = Some(value()?.into()),
            _ => bail!(USAGE),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    match parse_options(std::env::args().skip(1)) {
        Ok(options) => {
            run(options);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("yttt-ui-gallery: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) {
    let mut settings = AppSettings::default();
    if let Some(style) = options.style {
        settings.theme.ui_style = style;
    }
    let runtime = ThemeRuntime::resolve(&settings, &ThemeStore::builtin());
    let config_paths =
        AppConfigPaths::from_config_dir(std::env::temp_dir().join("yttt-ui-gallery"));

    gpui_platform::application()
        .with_assets(assets::app_assets(&config_paths))
        .run(move |cx: &mut App| {
            yttt_ui::init(cx);
            assets::load_ui_fonts(cx);
            let appearance = AppearanceState::new(runtime);
            Theme::global_mut(cx).apply_config(&Rc::new(
                appearance.runtime().to_gpui_component_theme_config(),
            ));
            let theme = appearance.runtime().ui;
            let ui_style = appearance.runtime().style;
            cx.set_global(appearance);

            let layout = LayoutMap::default();
            let view_layout = layout.clone();
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(40.), px(40.)),
                    size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
                ))),
                ..Default::default()
            };
            let handle = cx
                .open_window(window_options, move |window, cx| {
                    let view = cx.new(|cx| Gallery::new(theme, ui_style, view_layout, window, cx));
                    cx.new(|cx| yttt_ui::window::yttt_root(view, window, cx))
                })
                .expect("gallery window opens");
            cx.activate(true);

            if options.screenshot.is_none() && options.dump_layout.is_none() {
                return;
            }
            let Options {
                screenshot,
                dump_layout,
                ..
            } = options;
            cx.spawn(async move |cx| {
                cx.background_executor().timer(SETTLE_DELAY).await;
                let finished = handle.update(cx, |_, window, _| {
                    export(window, &layout, screenshot.as_ref(), dump_layout.as_ref())
                });
                if let Err(error) = finished.and_then(|result| result) {
                    eprintln!("yttt-ui-gallery: {error:#}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            })
            .detach();
        });
}

fn export(
    window: &mut Window,
    layout: &LayoutMap,
    screenshot: Option<&PathBuf>,
    dump_layout: Option<&PathBuf>,
) -> Result<()> {
    if let Some(path) = screenshot {
        window
            .render_to_image()
            .context("render gallery to image")?
            .save(path)
            .with_context(|| format!("write {}", path.display()))?;
    }
    if let Some(path) = dump_layout {
        let scale = window.scale_factor();
        let bounds: BTreeMap<_, _> = layout
            .borrow()
            .iter()
            .map(|(name, bounds)| {
                (
                    name.clone(),
                    serde_json::json!({
                        "x": f32::from(bounds.origin.x),
                        "y": f32::from(bounds.origin.y),
                        "width": f32::from(bounds.size.width),
                        "height": f32::from(bounds.size.height),
                    }),
                )
            })
            .collect();
        let document = serde_json::json!({ "scale_factor": scale, "components": bounds });
        std::fs::write(path, serde_json::to_string_pretty(&document)? + "\n")
            .with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

struct Gallery {
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    layout: LayoutMap,
    input: Entity<InputState>,
    filled_input: Entity<InputState>,
    select: Entity<SelectState<Vec<SharedString>>>,
    menu: Entity<PopupMenu>,
    tooltip: gpui::AnyView,
}

impl Gallery {
    fn new(
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        layout: LayoutMap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search files"));
        let filled_input = cx.new(|cx| InputState::new(window, cx).default_value("src/main.rs"));
        let select = cx.new(|cx| {
            SelectState::new(
                vec![
                    SharedString::from("One Dark"),
                    SharedString::from("One Light"),
                ],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let menu = yttt_menu_preview(
            YtttMenu::default()
                .item(YtttMenuItem::new("Rename"))
                .item(YtttMenuItem::new("Duplicate"))
                .item(YtttMenuItem::separator())
                .item(YtttMenuItem::new("Delete").disabled(true)),
            theme,
            ui_style,
            window,
            cx,
        );
        let tooltip = cx
            .new(|_| yttt_tooltip("Toggle Terminal", theme, ui_style))
            .into();
        Self {
            theme,
            ui_style,
            layout,
            input,
            filled_input,
            select,
            menu,
            tooltip,
        }
    }

    fn tracked(&self, name: &str, child: impl IntoElement) -> AnyElement {
        let layout = self.layout.clone();
        let key = name.to_string();
        div()
            .relative()
            .child(child)
            .child(
                canvas(
                    move |bounds, _, _| {
                        layout.borrow_mut().insert(key, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }

    fn section(&self, title: &str, children: Vec<AnyElement>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(self.theme.text_muted)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .children(children),
            )
    }

    fn buttons(&self) -> Vec<AnyElement> {
        let (theme, ui_style) = (self.theme, self.ui_style);
        let mut elements = Vec::new();
        for (name, variant) in [
            ("primary", YtttButtonVariant::Primary),
            ("secondary", YtttButtonVariant::Secondary),
            ("ghost", YtttButtonVariant::Ghost),
            ("danger", YtttButtonVariant::Danger),
        ] {
            let style = yttt_button_style(variant, theme, ui_style);
            let id = |state: &str| SharedString::from(format!("button-{name}-{state}"));
            let base =
                |state: &str| yttt_button(id(state), state.to_string(), variant, theme, ui_style);
            elements.push(self.tracked(&format!("button.{name}.default"), base("Default")));
            elements.push(self.tracked(
                &format!("button.{name}.hover"),
                base("Hover").bg(style.hover_background),
            ));
            elements.push(self.tracked(
                &format!("button.{name}.active"),
                base("Active").bg(style.active_background),
            ));
            elements.push(self.tracked(
                &format!("button.{name}.disabled"),
                base("Disabled").disabled(true),
            ));
        }
        elements
    }

    fn icon_buttons(&self) -> Vec<AnyElement> {
        [
            ("toolbar", YtttIconButtonKind::Toolbar),
            ("sidebar", YtttIconButtonKind::SidebarHeader),
            ("tab-close", YtttIconButtonKind::TabClose),
            ("overlay-close", YtttIconButtonKind::OverlayClose),
        ]
        .into_iter()
        .map(|(name, kind)| {
            self.tracked(
                &format!("icon_button.{name}"),
                yttt_icon_button(
                    SharedString::from(format!("icon-{name}")),
                    IconName::Close,
                    kind,
                    self.theme,
                    self.ui_style,
                    |_, _, _| {},
                ),
            )
        })
        .collect()
    }

    fn rows(&self) -> Vec<AnyElement> {
        [
            ("palette", YtttRowKind::Palette),
            ("palette_compact", YtttRowKind::PaletteCompact),
            ("settings", YtttRowKind::Settings),
            ("sidebar", YtttRowKind::Sidebar),
        ]
        .into_iter()
        .flat_map(|(name, kind)| {
            [
                ("active", SelectableState::Active),
                ("inactive", SelectableState::Inactive),
            ]
            .map(|(state_name, state)| {
                self.tracked(
                    &format!("row.{name}.{state_name}"),
                    yttt_row(kind, state, true, self.theme, self.ui_style)
                        .w(px(200.))
                        .child(format!("{name} {state_name}")),
                )
            })
        })
        .collect()
    }

    fn tree(&self) -> AnyElement {
        let rows = [("root", 0.), ("nested", 1.), ("leaf", 2.)]
            .into_iter()
            .map(|(name, depth)| {
                yttt_row(
                    YtttRowKind::Sidebar,
                    SelectableState::Inactive,
                    true,
                    self.theme,
                    self.ui_style,
                )
                .w(px(200.))
                .pl(rems(0.75 * depth))
                .child(name.to_string())
            });
        self.tracked("tree", div().flex().flex_col().children(rows))
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (theme, ui_style) = (self.theme, self.ui_style);
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(theme.surface)
            .text_color(theme.text)
            .text_sm()
            .child(self.section("Button", self.buttons()))
            .child(self.section("Icon button", self.icon_buttons()))
            .child(self.section(
                "Input",
                vec![
                    self.tracked(
                        "input.search.placeholder",
                        yttt_input(&self.input, YtttInputKind::Search, theme, ui_style).w(px(220.)),
                    ),
                    self.tracked(
                        "input.dialog.filled",
                        yttt_input(&self.filled_input, YtttInputKind::Dialog, theme, ui_style)
                            .w(px(220.)),
                    ),
                    self.tracked("select", yttt_select(&self.select, theme, ui_style)),
                ],
            ))
            .child(self.section(
                "Switch and radio",
                vec![
                    self.tracked(
                        "switch.on",
                        yttt_switch("switch-on", true, theme, ui_style, |_, _, _| {}),
                    ),
                    self.tracked(
                        "switch.off",
                        yttt_switch("switch-off", false, theme, ui_style, |_, _, _| {}),
                    ),
                    self.tracked("radio.on", yttt_radio("radio-on", "Selected", true)),
                    self.tracked("radio.off", yttt_radio("radio-off", "Unselected", false)),
                ],
            ))
            .child(self.section(
                "Tabs",
                vec![
                    self.tracked(
                        "tab.active",
                        yttt_tab(SelectableState::Active, theme, ui_style).child("main.rs"),
                    ),
                    self.tracked(
                        "tab.inactive",
                        yttt_tab(SelectableState::Inactive, theme, ui_style).child("lib.rs"),
                    ),
                ],
            ))
            .child(self.section("Rows", self.rows()))
            .child(self.section("Tree", vec![self.tree()]))
            .child(self.section(
                "Overlays",
                vec![
                    self.tracked("tooltip", self.tooltip.clone()),
                    self.tracked("menu", self.menu.clone()),
                    self.tracked(
                        "dialog",
                        yttt_dialog_surface(theme, ui_style)
                            .w(px(240.))
                            .child("Discard changes?")
                            .child(
                                div()
                                    .text_color(yttt_dialog_style(theme, ui_style).hint)
                                    .child("This cannot be undone."),
                            ),
                    ),
                    self.tracked(
                        "notification",
                        yttt_notification_surface(YtttNotificationTone::Info, theme, ui_style)
                            .child("Saved"),
                    ),
                ],
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options> {
        parse_options(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn options_parse_style_and_output_paths() {
        let options = parse(&[
            "--style",
            "rounded",
            "--screenshot",
            "a.png",
            "--dump-layout",
            "b.json",
        ])
        .unwrap();
        assert_eq!(options.style, Some(UiStyleId::Rounded));
        assert_eq!(options.screenshot, Some(PathBuf::from("a.png")));
        assert_eq!(options.dump_layout, Some(PathBuf::from("b.json")));
    }

    #[test]
    fn options_reject_unknown_values() {
        assert!(parse(&["--style", "plain"]).is_err());
        assert!(parse(&["--unknown"]).is_err());
        assert!(parse(&["--screenshot"]).is_err());
    }
}
