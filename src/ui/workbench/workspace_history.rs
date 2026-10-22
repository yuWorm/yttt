use super::*;
use chrono::{DateTime, Datelike as _, Local, TimeZone};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use serde::Deserialize as _;
use yttt_protocol::workspace::{
    WorkspaceId, WorkspaceRequest, WorkspaceResponse, WorkspaceSnapshot, WorkspaceSummary,
};

use crate::ui::{
    components::workbench_palette_item,
    i18n::Locale,
    palette::picker::picker_header,
    primitives::panel::{YtttOverlayPlacement, YtttPanelKind, yttt_panel, yttt_panel_overlay},
    primitives::row::YtttRowKind,
    theme::current_workbench_theme,
};

#[derive(Default)]
pub(super) struct WorkspaceHistoryState {
    pub entries: Vec<WorkspaceHistoryEntry>,
    loading: bool,
    pub(super) error: Option<String>,
    load_task: Option<Task<()>>,
}

pub(super) struct WorkspaceHistoryEntry {
    pub summary: WorkspaceSummary,
    pub item: PaletteItem,
    details: SharedString,
}

impl WorkspaceHistoryEntry {
    fn new(
        summary: WorkspaceSummary,
        snapshot: Result<WorkspaceSnapshot, String>,
        text: UiText,
        now: DateTime<Local>,
    ) -> Self {
        let mut names = Vec::new();
        let mut paths = Vec::new();
        let mut metadata_error = None;
        match snapshot {
            Ok(snapshot) => {
                if let Some(workspace) = snapshot.as_value().get("workspace")
                    && let Some(projects) =
                        workspace.get("opened_projects").and_then(|v| v.as_array())
                {
                    let selected = workspace.get("selected_project_id");
                    // The selected project leads, but every project's name and path is searchable.
                    let selected_index = projects
                        .iter()
                        .position(|project| project.get("id") == selected);
                    let ordered = selected_index
                        .into_iter()
                        .chain((0..projects.len()).filter(|index| Some(*index) != selected_index));
                    for index in ordered {
                        let project = &projects[index];
                        let location = project
                            .get("location")
                            .and_then(|value| ProjectLocation::deserialize(value).ok());
                        let name = project
                            .pointer("/layout/project/name")
                            .and_then(|value| value.as_str())
                            .filter(|name| !name.trim().is_empty())
                            .map(str::to_owned)
                            .or_else(|| location.as_ref().map(ProjectLocation::fallback_title));
                        if let Some(name) = name {
                            names.push(name);
                        }
                        if let Some(location) = location {
                            paths.push(location.display_path());
                        }
                    }
                }
            }
            Err(error) => metadata_error = Some(error),
        }
        let id = summary.workspace_id.as_str();
        let title = if !summary.name.trim().is_empty() && summary.name != id {
            summary.name.clone()
        } else if !names.is_empty() {
            names.join(" · ")
        } else {
            format!(
                "{} · {}",
                text.get(UiTextKey::WorkspaceHistoryUnnamed),
                &id[..id.len().min(8)]
            )
        };
        let subtitle = if metadata_error.is_some() {
            text.get(UiTextKey::WorkspaceHistoryMetadataUnavailable)
                .to_string()
        } else if paths.is_empty() {
            text.get(UiTextKey::WorkspaceHistoryNoProjects).to_string()
        } else {
            paths.join(" · ")
        };
        let (date, full_date) = saved_date(summary.saved_millis, now, text);
        let mut details = format!(
            "{title}\n{subtitle}\n{}: {full_date}\nID: {id}",
            text.get(UiTextKey::WorkspaceHistorySaved)
        );
        if let Some(error) = metadata_error {
            details.push('\n');
            details.push_str(&error);
        }
        let item = PaletteItem {
            id: id.to_string(),
            title,
            subtitle: Some(subtitle),
            status: Some(date),
            keybinding: None,
            command: CommandId::ProjectOpenRecent,
            enabled: true,
            disabled_reason: None,
        };
        Self {
            summary,
            item,
            details: details.into(),
        }
    }
}

fn saved_date<T: TimeZone>(millis: u64, now: DateTime<T>, text: UiText) -> (String, String)
where
    T::Offset: std::fmt::Display,
{
    let saved = i64::try_from(millis)
        .ok()
        .filter(|millis| *millis > 0)
        .and_then(|millis| now.timezone().timestamp_millis_opt(millis).single());
    let Some(saved) = saved else {
        let unknown = text.get(UiTextKey::WorkspaceHistoryUnknownDate).to_string();
        return (unknown.clone(), unknown);
    };
    let day = saved.date_naive();
    let (time_format, same_year_format, prior_year_format, full_format) = match text.locale() {
        Locale::English => (
            "%-I:%M %p",
            "%b %-d, %-I:%M %p",
            "%b %-d, %Y",
            "%b %-d, %Y, %-I:%M:%S %p %:z",
        ),
        Locale::Chinese => (
            "%H:%M",
            "%-m月%-d日 %H:%M",
            "%Y年%-m月%-d日",
            "%Y年%-m月%-d日 %H:%M:%S %:z",
        ),
    };
    let label = if day == now.date_naive() {
        format!(
            "{} {}",
            text.get(UiTextKey::WorkspaceHistoryToday),
            saved.format(time_format)
        )
    } else if Some(day) == now.date_naive().pred_opt() {
        format!(
            "{} {}",
            text.get(UiTextKey::WorkspaceHistoryYesterday),
            saved.format(time_format)
        )
    } else if saved.year() == now.year() {
        saved.format(same_year_format).to_string()
    } else {
        saved.format(prior_year_format).to_string()
    };
    (label, saved.format(full_format).to_string())
}

impl WorkbenchView {
    pub(super) fn open_workspace_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        self.open_palette(PaletteKind::WorkspaceHistory);
        self.palette.workspace_history = WorkspaceHistoryState {
            loading: true,
            ..Default::default()
        };
        let text = self.ui_text;
        let load_runtime = runtime.clone();
        let task = cx.background_spawn(async move {
            let mut entries = match load_runtime.workspace_request(WorkspaceRequest::List)? {
                WorkspaceResponse::Workspaces(entries) => entries,
                response => {
                    return Err(format!(
                        "Unexpected workspace history response: {response:?}"
                    ));
                }
            };
            entries.retain(|entry| !load_runtime.workspace_has_view(&entry.workspace_id));
            entries.sort_by(|a, b| {
                b.saved_millis
                    .cmp(&a.saved_millis)
                    .then_with(|| a.workspace_id.as_str().cmp(b.workspace_id.as_str()))
            });
            let now = Local::now();
            // Read one snapshot at a time; retain only display metadata, never draft bodies.
            let entries = entries
                .into_iter()
                .map(|summary| {
                    let snapshot =
                        match load_runtime.workspace_request(WorkspaceRequest::OpenExisting {
                            workspace_id: summary.workspace_id.clone(),
                        }) {
                            Ok(WorkspaceResponse::Opened { snapshot, .. }) => Ok(snapshot),
                            Ok(response) => {
                                Err(format!("Unexpected workspace response: {response:?}"))
                            }
                            Err(error) => Err(error),
                        };
                    WorkspaceHistoryEntry::new(summary, snapshot, text, now)
                })
                .collect::<Vec<_>>();
            Ok::<_, String>(entries)
        });
        self.palette.workspace_history.load_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ =
                    this.update(cx, |root, cx| {
                        if !root
                            .terminal
                            .host_runtime
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &runtime))
                            || !root.palette.active_palette.as_ref().is_some_and(|palette| {
                                palette.kind == PaletteKind::WorkspaceHistory
                            })
                        {
                            return;
                        }
                        let state = &mut root.palette.workspace_history;
                        state.loading = false;
                        match result {
                            Ok(mut entries) => {
                                entries.retain(|entry| {
                                    !runtime.workspace_has_view(&entry.summary.workspace_id)
                                });
                                state.entries = entries;
                            }
                            Err(error) => {
                                state.error = Some(format!(
                                    "{}: {error}",
                                    root.ui_text
                                        .get(UiTextKey::WorkbenchI18nWorkspaceHistoryLoadError)
                                ))
                            }
                        }
                        cx.notify();
                    });
            }));
        cx.notify();
    }

    pub(super) fn confirm_workspace_history(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(entry) = self
            .palette
            .workspace_history
            .entries
            .iter()
            .find(|entry| entry.item.id == id)
        else {
            return;
        };
        match crate::ui::app::open_history_workspace(entry.summary.workspace_id.clone(), cx) {
            Ok(()) => self.close_palette(),
            Err(error) => {
                self.palette.workspace_history.error = Some(format!(
                    "{}: {error}",
                    self.ui_text
                        .get(UiTextKey::WorkbenchI18nWorkspaceHistoryOpenError)
                ))
            }
        }
        cx.notify();
    }

    pub(super) fn remove_workspace_history_entry(&mut self, id: &WorkspaceId) {
        self.palette
            .workspace_history
            .entries
            .retain(|entry| &entry.summary.workspace_id != id);
        let items = self.palette_items(PaletteKind::WorkspaceHistory);
        if let Some(active) = &mut self.palette.active_palette
            && active.kind == PaletteKind::WorkspaceHistory
        {
            active.selected_index = active
                .selected_index
                .min(active.filtered_items(&items).len().saturating_sub(1));
            self.palette.input_needs_focus = true;
        }
    }

    pub(super) fn workspace_history_overlay(
        &self,
        active: &ActivePalette,
        items: &[PaletteItem],
        input: &Entity<InputState>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let available_height = (window.viewport_size().height
            - style.spacing.overlay_top.to_pixels(window.rem_size())
            - px(12.0))
        .max(px(0.0));
        let available_width = (window.viewport_size().width - px(24.0)).max(px(0.0));
        let text = self.ui_text;
        let state = &self.palette.workspace_history;
        let visible = active.filtered_items(items);
        let selected = active.selected_index.min(visible.len().saturating_sub(1));
        self.palette.scroll_handle.scroll_to_item(selected);
        let mut list = div()
            .id("workspace-history-list")
            .debug_selector(|| "workspace-history-list".into())
            .flex()
            .flex_col()
            .min_h_0()
            .px(style.palette.list_padding_x)
            .py(style.palette.list_padding_y)
            .max_h(style.palette.body_max_height)
            .overflow_y_scroll()
            .track_scroll(&self.palette.scroll_handle);
        if state.loading || visible.is_empty() {
            let message = if state.loading {
                UiTextKey::WorkspaceHistoryLoading
            } else if state.error.is_some() {
                UiTextKey::WorkspaceHistoryLoadFailed
            } else if state.entries.is_empty() {
                UiTextKey::WorkspaceHistoryEmpty
            } else {
                UiTextKey::NoResults
            };
            list = list.child(
                div()
                    .p(style.spacing.xl)
                    .text_sm()
                    .text_color(theme.text_subtle)
                    .child(text.get(message)),
            );
        }
        let mut entries = state.entries.iter();
        for (index, item) in visible.into_iter().enumerate() {
            let Some(entry) = entries.find(|entry| entry.item.id == item.id) else {
                continue;
            };
            let id = item.id.clone();
            let details = entry.details.clone();
            let row_state = if selected == index {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            };
            let mut row = div().flex().items_center().min_w_0().child(
                workbench_palette_item(
                    ("workspace-history-item", index),
                    item.title.clone(),
                    item.subtitle.clone().unwrap_or_default(),
                    item.status.clone().unwrap_or_default(),
                    None,
                    Some(IconName::FolderClosed),
                    YtttRowKind::Palette,
                    row_state,
                    true,
                    theme,
                    style,
                    cx.listener(move |root, _, _, cx| root.confirm_workspace_history(&id, cx)),
                )
                .flex_1()
                .min_w_0()
                .debug_selector(move || format!("workspace-history-item-{index}"))
                .tooltip(move |window, cx| {
                    yttt_ui::primitives::tooltip::yttt_tooltip(
                        details.clone(),
                        current_workbench_theme(cx),
                        current_ui_style(cx),
                    )
                    .build(window, cx)
                }),
            );
            if !entry.summary.restore_on_startup
                && self
                    .terminal
                    .host_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.is_controller())
            {
                let summary = entry.summary.clone();
                let owner = cx.weak_entity();
                row = row.child(
                    yttt_button(
                        ("workspace-history-more", index),
                        "⋯",
                        YtttButtonVariant::Ghost,
                        theme,
                        style,
                        cx,
                    )
                    .w(px(28.0))
                    .flex_none()
                    .tooltip(text.get(UiTextKey::WorkspaceHistoryMore))
                    .dropdown_menu(move |menu, _, cx| {
                        let summary = summary.clone();
                        let owner = owner.clone();
                        yttt_ui::primitives::menu::yttt_popup_menu(
                            menu,
                            current_workbench_theme(cx),
                            current_ui_style(cx),
                        )
                        .item(
                            PopupMenuItem::new(text.get(UiTextKey::WorkspaceHistoryRemove))
                                .on_click(move |_, window, cx| {
                                    let _ = owner.update(cx, |root, cx| {
                                        root.forget_workspace_history(summary.clone(), window, cx)
                                    });
                                }),
                        )
                    }),
                );
            } else {
                row = row.child(div().w(px(28.0)).flex_none());
            }
            list = list.child(row);
        }
        yttt_panel_overlay(
            yttt_panel(YtttPanelKind::Palette, theme, style)
                .p_0()
                .overflow_hidden()
                .max_h(style.palette.panel_max_height.min(available_height))
                .max_w(style.palette.panel_max_width.min(available_width))
                .debug_selector(|| "workspace-history-picker".into())
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(style.spacing.lg)
                        .py(style.spacing.sm)
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(text.get(UiTextKey::WorkspaceHistoryTitle)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(text.get(UiTextKey::WorkspaceHistorySaved)),
                        ),
                )
                .child(picker_header(input, theme, style).flex_none())
                .children(state.error.as_ref().map(|error| {
                    div()
                        .px(style.spacing.lg)
                        .py(style.spacing.sm)
                        .text_sm()
                        .child(error.clone())
                }))
                .child(list)
                .child(
                    div()
                        .border_t(style.border.hairline)
                        .border_color(theme.border)
                        .px(style.spacing.lg)
                        .h(style.controls.palette_footer_height)
                        .flex_none()
                        .flex()
                        .items_center()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::WorkspaceHistoryNavigation)),
                ),
            YtttPanelKind::Palette,
            YtttOverlayPlacement::Top,
            theme,
            style,
        )
    }
}
