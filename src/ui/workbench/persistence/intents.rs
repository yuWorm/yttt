//! Layout intents on their way to the Host. The window shows an intent as
//! soon as it is made and sends it without waiting: over a relay one answer
//! takes a round trip of hundreds of milliseconds, and a click sends up to
//! three intents. The Host applies one session's intents in the order sent,
//! so they are pipelined; their answers are read here, in the same order.
//!
//! While any intent is in flight the window holds a layout the Host has not
//! published yet, so Host layouts that arrive meanwhile wait until the last
//! answer: adopting one earlier would undo the intents still on their way.
//! An intent the Host refused leaves the window showing a layout the Host
//! never had; once the last answer is in, the Host layout replaces it.

use super::*;
use yttt_client_core::ClientCoreError;
use yttt_protocol::Response;

type Answer = flume::Receiver<Result<Response, ClientCoreError>>;

#[derive(Default)]
pub(super) struct PendingIntents {
    sender: Option<flume::Sender<(WorkspaceId, Answer)>>,
    worker: Option<Task<()>>,
    in_flight: usize,
    /// A Host layout arrived while intents were in flight.
    layout_deferred: bool,
    /// Why the Host refused an intent of the current batch.
    refused: Option<String>,
}

impl PendingIntents {
    /// Notes a Host layout that must wait for the intents in flight.
    /// Returns whether it waits.
    pub(super) fn defer_layout(&mut self) -> bool {
        if self.in_flight == 0 {
            return false;
        }
        self.layout_deferred = true;
        true
    }
}

/// What the window does once an intent's answer is read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum IntentSettled {
    /// More answers are due; nothing to do yet.
    Waiting,
    /// The last answer is in and the Host published nothing the window
    /// does not already show.
    Settled,
    /// The last answer is in; a Host layout that arrived meanwhile applies.
    AdoptLayout,
    /// The last answer is in and an intent was refused: the Host layout
    /// replaces the window's, and the reason is shown.
    Refused(String),
}

/// Records the answer to one intent of `workspace_id` and says what follows.
/// `revision` is the Host layout revision the window shows; it advances when
/// the Host applied the intent right after the layout the window shows, so
/// the broadcast of the window's own intent changes nothing.
pub(super) fn settle_intent(
    pending: &mut PendingIntents,
    revision: &mut Option<u64>,
    shown_workspace: Option<&WorkspaceId>,
    workspace_id: &WorkspaceId,
    answer: Result<Response, ClientCoreError>,
) -> IntentSettled {
    pending.in_flight = pending.in_flight.saturating_sub(1);
    // An answer for a workspace the window no longer shows changes nothing
    // the window shows, and is no reason to replace its layout.
    let shown = shown_workspace == Some(workspace_id);
    match answer {
        Ok(Response::Workspace(WorkspaceResponse::Applied {
            workspace_id: applied,
            revision: applied_revision,
        })) if &applied == workspace_id => {
            if shown && revision.map(|held| held + 1) == Some(applied_revision) {
                *revision = Some(applied_revision);
            }
        }
        Ok(response) if shown => {
            pending
                .refused
                .get_or_insert_with(|| format!("unexpected workspace response: {response:?}"));
        }
        Err(error) if shown => {
            pending.refused.get_or_insert_with(|| error.to_string());
        }
        Ok(_) | Err(_) => {}
    }
    if pending.in_flight > 0 {
        return IntentSettled::Waiting;
    }
    let deferred = std::mem::take(&mut pending.layout_deferred);
    match pending.refused.take() {
        Some(reason) => IntentSettled::Refused(reason),
        None if deferred => IntentSettled::AdoptLayout,
        None => IntentSettled::Settled,
    }
}

impl WorkbenchView {
    /// Starts reading intent answers for this window; the sender lives as
    /// long as the window does.
    pub(super) fn start_intent_worker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = &mut self.workspace_persistence.intents;
        if pending.worker.is_some() {
            return;
        }
        let (sender, answers) = flume::unbounded::<(WorkspaceId, Answer)>();
        pending.sender = Some(sender);
        pending.worker = Some(cx.spawn_in(window, async move |this, cx| {
            while let Ok((workspace_id, answer)) = answers.recv_async().await {
                // The Client fails a request at its own deadline, so every
                // answer comes.
                let answer = answer
                    .recv_async()
                    .await
                    .unwrap_or(Err(ClientCoreError::SupervisorStopped));
                if this
                    .update_in(cx, |root, window, cx| {
                        root.finish_workspace_intent(&workspace_id, answer, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// Sends `intent` without waiting for its answer. `false` when the
    /// window has no worker to read the answer, so the caller must not
    /// treat the intent as sent.
    pub(super) fn send_workspace_intent(
        &mut self,
        runtime: &crate::host_runtime::DesktopHostRuntime,
        workspace_id: WorkspaceId,
        intent: crate::model::host_workspace::WorkspaceIntent,
    ) -> bool {
        let pending = &mut self.workspace_persistence.intents;
        let Some(sender) = pending.sender.as_ref() else {
            return false;
        };
        let answer = runtime.request(yttt_protocol::Request::Workspace(WorkspaceRequest::Apply {
            workspace_id: workspace_id.clone(),
            intent,
            expected_revision: None,
        }));
        if sender.send((workspace_id, answer)).is_err() {
            return false;
        }
        pending.in_flight += 1;
        true
    }

    fn finish_workspace_intent(
        &mut self,
        workspace_id: &WorkspaceId,
        answer: Result<Response, ClientCoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone());
        let persistence = &mut self.workspace_persistence;
        match settle_intent(
            &mut persistence.intents,
            &mut persistence.revision,
            shown.as_ref(),
            workspace_id,
            answer,
        ) {
            IntentSettled::Waiting | IntentSettled::Settled => {}
            IntentSettled::AdoptLayout => self.adopt_host_workspace_layout(window, cx),
            IntentSettled::Refused(reason) => {
                tracing::warn!(%reason, "the Host refused a layout change; showing its layout");
                self.load_error = combine_load_messages(
                    self.load_error.take(),
                    Some(WorkbenchError::HostWorkspace(reason).to_string()),
                );
                self.restore_host_workspace_layout(window, cx);
                cx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: &str) -> WorkspaceId {
        WorkspaceId::new(id).unwrap()
    }

    fn applied(id: &WorkspaceId, revision: u64) -> Result<Response, ClientCoreError> {
        Ok(Response::Workspace(WorkspaceResponse::Applied {
            workspace_id: id.clone(),
            revision,
        }))
    }

    fn sent(count: usize) -> PendingIntents {
        PendingIntents {
            in_flight: count,
            ..PendingIntents::default()
        }
    }

    #[test]
    fn pipelined_intents_of_this_window_advance_the_revision_and_change_nothing_else() {
        let id = workspace("w1");
        let mut pending = sent(3);
        let mut revision = Some(10);
        // The broadcasts of the window's own intents arrive first.
        assert!(pending.defer_layout());
        for (answer, expected) in [(11, IntentSettled::Waiting), (12, IntentSettled::Waiting)] {
            let settled = settle_intent(
                &mut pending,
                &mut revision,
                Some(&id),
                &id,
                applied(&id, answer),
            );
            assert_eq!(settled, expected);
        }
        let settled = settle_intent(
            &mut pending,
            &mut revision,
            Some(&id),
            &id,
            applied(&id, 13),
        );
        // The deferred layout is read once more; it is revision 13, which the
        // window already shows.
        assert_eq!(settled, IntentSettled::AdoptLayout);
        assert_eq!(revision, Some(13));
        assert_eq!(pending.in_flight, 0);
        assert!(
            !pending.defer_layout(),
            "nothing in flight: layouts apply at once"
        );
    }

    #[test]
    fn another_clients_change_in_between_leaves_the_revision_for_its_layout() {
        let id = workspace("w1");
        let mut pending = sent(2);
        let mut revision = Some(10);
        assert_eq!(
            settle_intent(
                &mut pending,
                &mut revision,
                Some(&id),
                &id,
                applied(&id, 11)
            ),
            IntentSettled::Waiting
        );
        assert!(pending.defer_layout());
        // Revision 12 was another Client's.
        assert_eq!(
            settle_intent(
                &mut pending,
                &mut revision,
                Some(&id),
                &id,
                applied(&id, 13)
            ),
            IntentSettled::AdoptLayout
        );
        assert_eq!(
            revision,
            Some(11),
            "the Host layout at 13 replaces the window's"
        );
    }

    #[test]
    fn a_refusal_restores_the_host_layout_once_the_last_answer_is_in() {
        let id = workspace("w1");
        let mut pending = sent(2);
        let mut revision = Some(10);
        assert_eq!(
            settle_intent(
                &mut pending,
                &mut revision,
                Some(&id),
                &id,
                Err(ClientCoreError::NotConnected)
            ),
            IntentSettled::Waiting
        );
        let settled = settle_intent(
            &mut pending,
            &mut revision,
            Some(&id),
            &id,
            applied(&id, 11),
        );
        assert!(matches!(settled, IntentSettled::Refused(_)), "{settled:?}");
        // The next batch starts clean.
        let mut pending_next = pending;
        pending_next.in_flight = 1;
        assert_eq!(
            settle_intent(
                &mut pending_next,
                &mut revision,
                Some(&id),
                &id,
                applied(&id, 12)
            ),
            IntentSettled::Settled
        );
    }

    #[test]
    fn answers_for_a_workspace_the_window_left_change_nothing() {
        let left = workspace("w1");
        let shown = workspace("w2");
        let mut pending = sent(2);
        let mut revision = Some(4);
        assert_eq!(
            settle_intent(
                &mut pending,
                &mut revision,
                Some(&shown),
                &left,
                applied(&left, 5)
            ),
            IntentSettled::Waiting
        );
        assert_eq!(
            settle_intent(
                &mut pending,
                &mut revision,
                Some(&shown),
                &left,
                Err(ClientCoreError::RequestTimeout)
            ),
            IntentSettled::Settled
        );
        assert_eq!(revision, Some(4));
    }
}
