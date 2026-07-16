use super::*;

impl WorkbenchView {
    pub(super) fn split_resize_handle(
        &self,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = Self::visible_split_handle_style(direction);
        let theme = self.theme_runtime().ui;
        let mut handle = div()
            .id(match direction {
                SplitDirection::Horizontal => "horizontal-split-resize-handle",
                SplitDirection::Vertical => "vertical-split-resize-handle",
            })
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .bg(rgba(0x00000000))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_split_resize_drag(direction, event.position);
                    cx.stop_propagation();
                }),
            );

        handle = match direction {
            SplitDirection::Horizontal => handle.w(style.hit_area_width).cursor_ew_resize().child(
                div()
                    .w(style.visible_line_width)
                    .h_full()
                    .bg(theme.split_line),
            ),
            SplitDirection::Vertical => handle.h(style.hit_area_width).cursor_ns_resize().child(
                div()
                    .h(style.visible_line_width)
                    .w_full()
                    .bg(theme.split_line),
            ),
        };

        handle.into_any_element()
    }

    pub(super) fn sidebar_resize_handle(
        &self,
        side: SidebarSide,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, offset) = match side {
            SidebarSide::Left => ("project-sidebar-resize-handle", px(0.0)),
            SidebarSide::Right => ("project-file-panel-resize-handle", px(0.0)),
        };
        let line_color = if self
            .active_sidebar_resize_drag
            .is_some_and(|drag| drag.side == side)
        {
            self.theme_runtime().ui.split_line_active
        } else {
            self.theme_runtime().ui.split_line
        };
        let handle = div()
            .id(id)
            .debug_selector(move || id.to_string())
            .absolute()
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .w(px(SIDEBAR_RESIZE_HIT_AREA_WIDTH))
            .cursor_ew_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_sidebar_resize_drag(side, event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(div().h_full().w(px(1.0)).bg(line_color));

        match side {
            SidebarSide::Left => handle.right(offset),
            SidebarSide::Right => handle.left(offset),
        }
        .into_any_element()
    }

    pub fn visible_split_handle_style(_direction: SplitDirection) -> SplitHandleStyle {
        let theme = WorkbenchTheme::one_dark();
        SplitHandleStyle {
            visible_line_width: theme.split_line_width,
            hit_area_width: theme.split_hit_area_width,
        }
    }

    pub(super) fn begin_split_resize_drag(
        &mut self,
        direction: SplitDirection,
        position: Point<Pixels>,
    ) {
        self.active_sidebar_resize_drag = None;
        self.active_split_resize_drag = Some(ActiveSplitResizeDrag {
            direction,
            last_position: position,
        });
    }

    pub(super) fn begin_sidebar_resize_drag(&mut self, side: SidebarSide, position: Point<Pixels>) {
        self.active_split_resize_drag = None;
        self.active_sidebar_resize_drag = Some(ActiveSidebarResizeDrag {
            side,
            last_position: position,
        });
    }

    pub(super) fn resize_from_sidebar_drag(&mut self, side: SidebarSide, position: Point<Pixels>) {
        let Some(active_drag) = self.active_sidebar_resize_drag else {
            self.begin_sidebar_resize_drag(side, position);
            return;
        };
        if active_drag.side != side {
            self.begin_sidebar_resize_drag(side, position);
            return;
        }

        let delta_x = f32::from(position.x - active_drag.last_position.x);
        self.resize_sidebar_from_pointer_delta(side, delta_x);
        self.begin_sidebar_resize_drag(side, position);
    }

    pub(super) fn resize_from_split_drag(
        &mut self,
        direction: SplitDirection,
        position: Point<Pixels>,
    ) {
        let Some(active_drag) = self.active_split_resize_drag else {
            self.begin_split_resize_drag(direction, position);
            return;
        };
        if active_drag.direction != direction {
            self.begin_split_resize_drag(direction, position);
            return;
        }

        let delta_x = f32::from(position.x - active_drag.last_position.x);
        let delta_y = f32::from(position.y - active_drag.last_position.y);
        match self.resize_focused_split_from_pointer_delta(direction, delta_x, delta_y) {
            Ok(Some(_)) => self.begin_split_resize_drag(direction, position),
            Ok(None) => {}
            Err(error) => {
                self.load_error = Some(error.to_string());
                self.begin_split_resize_drag(direction, position);
            }
        }
    }

    pub(super) fn on_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_drag) = self.active_sidebar_resize_drag {
            if !event.dragging() {
                self.active_sidebar_resize_drag = None;
                cx.notify();
                return;
            }

            self.resize_from_sidebar_drag(active_drag.side, event.position);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let Some(active_drag) = self.active_split_resize_drag else {
            cx.propagate();
            return;
        };

        if !event.dragging() {
            self.active_split_resize_drag = None;
            cx.notify();
            return;
        }

        self.resize_from_split_drag(active_drag.direction, event.position);
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn on_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_drag) = self.active_sidebar_resize_drag.take() {
            if let Err(error) = self.persist_sidebar_width(active_drag.side) {
                self.load_error = Some(error.to_string());
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.active_split_resize_drag.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        }
    }
}
