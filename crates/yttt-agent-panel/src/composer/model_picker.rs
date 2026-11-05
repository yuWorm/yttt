//! The model picker, after Zed's: a search box, one compact row per model
//! under its provider, favorites first, and a Configure button. Rows show
//! the model's icon and name, with its context window and abilities quietly
//! on the right; models that cannot run say why. Picks, stars, setup and
//! refresh are requests the host answers; the list syncs from the agent.
use gpui::{
    AnyElement, Context, IntoElement, MouseButton, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::*, px, relative,
};
use gpui_component::{h_flex, v_flex};
use yttt_ui::{
    Icon, IconName,
    primitives::{input::yttt_borderless_input, spinner::yttt_spinner},
};

use super::{Composer, ComposerEvent, OpenMenu};
use crate::{
    composer::footer::format_tokens,
    event::{ModelInfo, ModelStatus},
};

const PICKER_WIDTH: f32 = 320.;
const PICKER_MAX_HEIGHT: f32 = 380.;
const ROW_HEIGHT: f32 = 30.;

impl Composer {
    /// The models the filter matches, in the order the picker lists them.
    pub(super) fn listed_models(&self, cx: &gpui::App) -> Vec<ModelInfo> {
        let query = self.model_filter.read(cx).value();
        self.catalog
            .sections(&query)
            .into_iter()
            .flat_map(|section| section.models.into_iter().cloned())
            .collect()
    }

    /// Moves the highlight to the next model that can be picked.
    pub(super) fn move_model_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let models = self.listed_models(cx);
        let usable = models
            .iter()
            .enumerate()
            .filter(|(_, model)| model.status == ModelStatus::Available)
            .map(|(ix, _)| ix)
            .collect::<Vec<_>>();
        if usable.is_empty() {
            return;
        }
        let at = usable
            .iter()
            .position(|ix| *ix == self.model_highlight)
            .map_or(0, |at| {
                (at as isize + delta).rem_euclid(usable.len() as isize) as usize
            });
        self.model_highlight = usable[at];
        cx.notify();
    }

    /// Highlights the active model, or the first one that can be picked.
    pub(super) fn reset_model_highlight(&mut self, cx: &gpui::App) {
        let models = self.listed_models(cx);
        let active = self.catalog.active().map(|model| model.id.clone());
        self.model_highlight = models
            .iter()
            .position(|model| Some(&model.id) == active.as_ref())
            .filter(|ix| models[*ix].status == ModelStatus::Available)
            .or_else(|| {
                models
                    .iter()
                    .position(|model| model.status == ModelStatus::Available)
            })
            .unwrap_or(0);
    }

    pub(super) fn pick_model(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog.select(id) {
            cx.emit(ComposerEvent::ModelSelected { id: id.to_owned() });
        }
        self.close_menu(window, cx);
    }

    pub(super) fn pick_highlighted_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let models = self.listed_models(cx);
        if let Some(model) = models
            .get(self.model_highlight)
            .filter(|model| model.status == ModelStatus::Available)
        {
            let id = model.id.clone();
            self.pick_model(&id, window, cx);
        }
    }

    /// The footer button: the active model's icon and name.
    pub(super) fn render_model_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.catalog.models().is_empty() {
            return None;
        }
        let theme = self.theme;
        let open = self.open_menu == Some(OpenMenu::Models);
        let active = self.catalog.active();
        let label = active.map_or_else(|| "Choose a model".to_owned(), |model| model.name.clone());
        Some(
            h_flex()
                .id("composer-model")
                .min_w_0()
                .h(px(24.))
                .gap(px(5.))
                .px(px(6.))
                .rounded(px(5.))
                .text_size(self.ui_style.text.xsmall)
                .text_color(theme.text_muted)
                .cursor_pointer()
                .when(open, |this| this.bg(theme.element_active))
                .hover(move |this| this.bg(theme.ghost_element_hover).text_color(theme.text))
                .children(
                    active
                        .map(|model| model_icon(model).size(px(12.)).text_color(theme.icon_muted)),
                )
                .child(div().truncate().child(label))
                .child(
                    Icon::new(IconName::ChevronDown)
                        .size(px(11.))
                        .text_color(theme.icon_muted),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if open {
                        this.close_menu(window, cx);
                    } else {
                        this.open_model_picker(window, cx);
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_model_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_style = self.ui_style;
        let query = self.model_filter.read(cx).value();
        let sections = self
            .catalog
            .sections(&query)
            .into_iter()
            .map(|section| {
                (
                    section.title,
                    section.models.into_iter().cloned().collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let mut row_ix = 0;
        let syncing = self.catalog.is_syncing();
        v_flex()
            .absolute()
            .left_0()
            .bottom(relative(1.))
            .mb(ui_style.spacing.xs)
            .w(px(PICKER_WIDTH))
            .max_h(px(PICKER_MAX_HEIGHT))
            .rounded(ui_style.radius.surface)
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .shadow_md()
            .overflow_hidden()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(px(38.))
                    .gap(ui_style.spacing.sm)
                    .pl(ui_style.spacing.md)
                    .pr(ui_style.spacing.sm)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div().flex_1().min_w_0().child(
                            yttt_borderless_input(&self.model_filter)
                                .text_size(ui_style.text.small)
                                .p_0(),
                        ),
                    )
                    .child(if syncing {
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(22.))
                            .child(yttt_spinner(px(12.), theme.icon_muted))
                            .into_any_element()
                    } else {
                        div()
                            .id("composer-models-refresh")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(22.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(move |this| this.bg(theme.ghost_element_hover))
                            .child(
                                Icon::new(IconName::RefreshCw)
                                    .size(px(12.))
                                    .text_color(theme.icon_muted),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.catalog.set_syncing(true);
                                cx.emit(ComposerEvent::RefreshModels);
                                cx.notify();
                            }))
                            .into_any_element()
                    }),
            )
            .child(
                v_flex()
                    .id("composer-model-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(sections.is_empty(), |this| {
                        this.child(
                            div()
                                .px(ui_style.spacing.md)
                                .py(ui_style.spacing.md)
                                .text_size(ui_style.text.small)
                                .text_color(theme.text_subtle)
                                .child(if self.catalog.models().is_empty() {
                                    "No models yet"
                                } else {
                                    "No matching models"
                                }),
                        )
                    })
                    .children(sections.into_iter().enumerate().map(
                        |(section_ix, (title, models))| {
                            v_flex()
                                .px(px(4.))
                                .py(px(4.))
                                .when(section_ix > 0, |this| {
                                    this.border_t_1().border_color(theme.border)
                                })
                                .child(
                                    div()
                                        .px(ui_style.spacing.sm)
                                        .pt(px(4.))
                                        .pb(px(2.))
                                        .text_size(ui_style.text.xsmall)
                                        .text_color(theme.text_subtle)
                                        .child(title),
                                )
                                .children(models.into_iter().map(|model| {
                                    let ix = row_ix;
                                    row_ix += 1;
                                    self.render_model_row(ix, &model, cx)
                                }))
                        },
                    )),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .p(px(4.))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        h_flex()
                            .id("composer-models-configure")
                            .w_full()
                            .h(px(28.))
                            .justify_center()
                            .rounded(px(5.))
                            .bg(theme.element_hover)
                            .text_size(ui_style.text.small)
                            .text_color(theme.text)
                            .cursor_pointer()
                            .hover(move |this| this.bg(theme.element_active))
                            .child("Configure")
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.emit(ComposerEvent::ManageModels);
                                this.close_menu(window, cx);
                            })),
                    ),
            )
    }

    fn render_model_row(&self, ix: usize, model: &ModelInfo, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let ui_style = self.ui_style;
        let usable = model.status == ModelStatus::Available;
        let highlighted = usable && ix == self.model_highlight;
        let active = self
            .catalog
            .active()
            .is_some_and(|active| active.id == model.id);
        let favorite = self.catalog.is_favorite(&model.id);
        let group = SharedString::from(format!("model-row-{ix}"));
        let (id, star_id, setup_id) = (model.id.clone(), model.id.clone(), model.id.clone());
        let ability = |icon: IconName| Icon::new(icon).size(px(11.)).text_color(theme.text_subtle);
        let details = match &model.status {
            ModelStatus::NeedsSetup => self
                .picker_link(
                    format!("composer-model-setup-{ix}"),
                    "Set up",
                    cx.listener(move |this, _, window, cx| {
                        cx.emit(ComposerEvent::ModelSetup {
                            id: setup_id.clone(),
                        });
                        this.close_menu(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .into_any_element(),
            ModelStatus::Unavailable(reason) => div()
                .max_w(px(130.))
                .truncate()
                .text_size(ui_style.text.xsmall)
                .text_color(theme.text_subtle)
                .child(reason.clone())
                .into_any_element(),
            ModelStatus::Available => h_flex()
                .gap(px(6.))
                .text_size(ui_style.text.xsmall)
                .text_color(theme.text_subtle)
                .when(model.capabilities.thinking, |this| {
                    this.child(ability(IconName::Asterisk))
                })
                .when(model.capabilities.vision, |this| {
                    this.child(ability(IconName::Eye))
                })
                .children(model.context_window.map(format_tokens))
                .into_any_element(),
        };
        h_flex()
            .group(group.clone())
            .id(SharedString::from(format!("composer-model-{ix}")))
            .h(px(ROW_HEIGHT))
            .gap(ui_style.spacing.sm)
            .px(ui_style.spacing.sm)
            .rounded(px(5.))
            .when(highlighted, |this| this.bg(theme.element_hover))
            .when(usable, |this| {
                this.cursor_pointer()
                    .hover(move |this| this.bg(theme.element_hover))
            })
            .child(model_icon(model).size(px(14.)).text_color(if active {
                theme.accent
            } else if usable {
                theme.icon_muted
            } else {
                theme.icon_disabled
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_style.text.small)
                    .text_color(if usable {
                        theme.text
                    } else {
                        theme.text_disabled
                    })
                    .child(model.name.clone()),
            )
            .child(details)
            .child(
                div()
                    .id(SharedString::from(format!("composer-model-star-{ix}")))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .when(!favorite, |this| {
                        this.invisible().group_hover(group, |this| this.visible())
                    })
                    .child(
                        Icon::new(if favorite {
                            IconName::StarFill
                        } else {
                            IconName::Star
                        })
                        .size(px(13.))
                        .text_color(if favorite {
                            theme.warning
                        } else {
                            theme.icon_muted
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            let favorite = !this.catalog.is_favorite(&star_id);
                            this.catalog.set_favorite(&star_id, favorite);
                            cx.emit(ComposerEvent::FavoriteModel {
                                id: star_id.clone(),
                                favorite,
                            });
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(div().w(px(13.)).flex_shrink_0().when(active, |this| {
                this.child(
                    Icon::new(IconName::Check)
                        .size(px(13.))
                        .text_color(theme.accent),
                )
            }))
            .when(usable, |this| {
                this.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.pick_model(&id, window, cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .into_any_element()
    }

    fn picker_link(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        let theme = self.theme;
        div()
            .id(id.into())
            .flex_shrink_0()
            .px(px(6.))
            .py(px(1.))
            .rounded(px(4.))
            .text_size(self.ui_style.text.xsmall)
            .text_color(theme.text_muted)
            .cursor_pointer()
            .hover(move |this| this.bg(theme.ghost_element_hover).text_color(theme.text))
            .child(label)
            .on_click(on_click)
    }
}

/// The model's own icon, or a generic chip.
fn model_icon(model: &ModelInfo) -> Icon {
    match &model.icon {
        Some(path) => Icon::empty().path(path.clone()),
        None => Icon::new(IconName::Cpu),
    }
}
