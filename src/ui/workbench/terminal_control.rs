//! Which Client writes the Host's terminals. One Client controls them all;
//! the others observe until they take control back.

use gpui::{Context, Task};
use yttt_client_core::{ClientCoreError, ClientEvent, ConnectionState};
use yttt_protocol::{FailureCode, Request, Response, ServerEvent, TerminalController};

use super::WorkbenchView;
use crate::ui::i18n::UiTextKey;

#[derive(Default)]
pub(super) struct TerminalControlState {
    /// `None` until the Host answers, and for a Host without a controller.
    controller: Option<TerminalController>,
    event_task: Option<Task<()>>,
    taking: bool,
}

impl TerminalControlState {
    /// Keeps the newer of two controller states; a Host restart starts over.
    fn apply(&mut self, state: TerminalController) {
        if self
            .controller
            .as_ref()
            .is_none_or(|current| current.epoch <= state.epoch)
        {
            self.controller = Some(state);
        }
    }
}

impl WorkbenchView {
    pub fn start_terminal_control_listener(&mut self, cx: &mut Context<Self>) {
        if self.terminal_control.event_task.is_some() {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let events = runtime.events();
        self.read_terminal_controller(cx);
        self.terminal_control.event_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv_async().await {
                let update = match event {
                    ClientEvent::Connection(ConnectionState::Ready { .. }) => None,
                    ClientEvent::Server(event) => match event.body {
                        ServerEvent::TerminalControllerChanged(state) => Some(state),
                        _ => continue,
                    },
                    _ => continue,
                };
                let updated = this.update(cx, |root, cx| match update {
                    Some(state) => {
                        root.terminal_control.apply(state);
                        cx.notify();
                    }
                    None => {
                        root.terminal_control.controller = None;
                        root.read_terminal_controller(cx);
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    pub(super) fn reset_terminal_control(&mut self) {
        self.terminal_control = TerminalControlState::default();
    }

    fn read_terminal_controller(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let response = runtime.request(Request::ReadTerminalController);
        cx.spawn(async move |this, cx| {
            // A Host that predates the controller answers `Unsupported`.
            if let Ok(Ok(Response::TerminalController(state))) = response.recv_async().await {
                let _ = this.update(cx, |root, cx| {
                    root.terminal_control.apply(state);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Whether another Client writes the terminals, so this one only observes.
    pub(super) fn terminals_controlled_elsewhere(&self) -> bool {
        let Some(runtime) = self.terminal.host_runtime.as_ref() else {
            return false;
        };
        runtime.is_connected()
            && self
                .terminal_control
                .controller
                .as_ref()
                .and_then(|state| state.controller.as_ref())
                .is_some_and(|controller| *controller != runtime.client_id())
    }

    pub(super) fn terminal_control_taking(&self) -> bool {
        self.terminal_control.taking
    }

    pub(super) fn take_terminal_control(&mut self, cx: &mut Context<Self>) {
        if self.terminal_control.taking {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        self.terminal_control.taking = true;
        cx.notify();
        let response = runtime.request(Request::TakeTerminalController);
        cx.spawn(async move |this, cx| {
            let result = response.recv_async().await;
            let _ = this.update(cx, |root, cx| {
                root.terminal_control.taking = false;
                match result {
                    Ok(Ok(Response::TerminalController(state))) => {
                        root.terminal_control.apply(state);
                    }
                    Ok(Err(ClientCoreError::Protocol(failure)))
                        if failure.code == FailureCode::Unsupported =>
                    {
                        root.terminal_control.controller = None;
                    }
                    result => {
                        let detail = match result {
                            Ok(Err(error)) => error.to_string(),
                            Ok(Ok(response)) => format!("{response:?}"),
                            Err(error) => error.to_string(),
                        };
                        root.load_error = Some(
                            root.ui_text
                                .get(UiTextKey::WorkspaceStatusTakeTerminalControlFailed)
                                .replace("{detail}", &detail),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
