use parking_lot::Mutex;
use yttt_core::model::ids::ClientInstanceId;
use yttt_protocol::{
    FailureCode, ProtocolFailure,
    session::{
        ControlContext, ControlStatus, ControlTransfer, ProfileControlRequest, TransferPhase,
        WorkspaceRevision,
    },
};

pub struct ProfileControl {
    state: Mutex<State>,
}

struct State {
    epoch: u64,
    revision: u64,
    owner: Option<ClientInstanceId>,
    pending: Option<PendingTransfer>,
}

struct PendingTransfer {
    wire: ControlTransfer,
    target: Option<ClientInstanceId>,
    revisions: Option<Vec<WorkspaceRevision>>,
}

impl ProfileControl {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                epoch: 0,
                revision: 0,
                owner: None,
                pending: None,
            }),
        }
    }

    pub fn status(&self, host_epoch: u64) -> ControlStatus {
        let mut state = self.state.lock();
        expire_preparation(&mut state);
        snapshot(&state, host_epoch)
    }

    pub fn authorize(
        &self,
        client: &ClientInstanceId,
        context: ControlContext,
        host_epoch: u64,
    ) -> Result<(), ProtocolFailure> {
        let state = self.state.lock();
        if context.host_epoch != host_epoch || context.control_epoch != state.epoch {
            return Err(error(
                FailureCode::StaleEpoch,
                "profile control epoch is no longer current",
            ));
        }
        authorize_owner(&state, client)
    }

    pub fn authorize_owner(&self, client: &ClientInstanceId) -> Result<(), ProtocolFailure> {
        authorize_owner(&self.state.lock(), client)
    }

    /// Preparation does not hold the mutation fence: the old Client must still flush.
    /// The returned transfer ID requires the caller to drain the fence before finish().
    pub fn request(
        &self,
        client: &ClientInstanceId,
        request: ProfileControlRequest,
    ) -> Result<Option<String>, ProtocolFailure> {
        let mut state = self.state.lock();
        expire_preparation(&mut state);
        if !matches!(request, ProfileControlRequest::Status) {
            state.revision = state.revision.saturating_add(1);
        }
        match request {
            ProfileControlRequest::Status => Ok(None),
            ProfileControlRequest::RequestControl => {
                if state.owner.as_ref() == Some(client) {
                    return Ok(None);
                }
                if let Some(pending) = &state.pending {
                    return if &pending.wire.requester == client {
                        Ok(None)
                    } else {
                        Err(error(
                            FailureCode::Conflict,
                            "another profile transfer is in progress",
                        ))
                    };
                }
                let id = uuid::Uuid::new_v4().to_string();
                let phase = if state.owner.is_none() {
                    TransferPhase::Fencing
                } else {
                    TransferPhase::Preparing
                };
                state.pending = Some(PendingTransfer {
                    wire: ControlTransfer {
                        id: id.clone(),
                        requester: client.clone(),
                        previous_owner: state.owner.clone(),
                        phase,
                        deadline_millis: crate::now_millis().saturating_add(5_000),
                    },
                    target: Some(client.clone()),
                    revisions: None,
                });
                Ok((phase == TransferPhase::Fencing).then_some(id))
            }
            ProfileControlRequest::Ready {
                transfer_id,
                revisions,
            } => {
                let pending = matching(&mut state, &transfer_id)?;
                if pending.wire.previous_owner.as_ref() != Some(client) {
                    return Err(error(
                        FailureCode::PermissionDenied,
                        "only the previous controller can publish transfer readiness",
                    ));
                }
                if pending.wire.phase == TransferPhase::Fencing {
                    return Err(error(FailureCode::Conflict, "transfer is already fencing"));
                }
                if revisions.len() > 128 {
                    return Err(error(
                        FailureCode::ResourceLimit,
                        "too many transfer workspace revisions",
                    ));
                }
                pending.revisions = Some(revisions);
                pending.wire.phase = TransferPhase::Fencing;
                Ok(Some(transfer_id))
            }
            ProfileControlRequest::ConfirmForce { transfer_id } => {
                let pending = matching(&mut state, &transfer_id)?;
                if &pending.wire.requester != client {
                    return Err(error(
                        FailureCode::PermissionDenied,
                        "only the requesting Client can confirm a forced transfer",
                    ));
                }
                if pending.wire.phase != TransferPhase::ForceConfirmationRequired {
                    return Err(error(
                        FailureCode::Conflict,
                        "forced transfer needs a separate confirmation after preparation expires",
                    ));
                }
                pending.wire.phase = TransferPhase::Fencing;
                Ok(Some(transfer_id))
            }
            ProfileControlRequest::Cancel { transfer_id } => {
                let pending = matching(&mut state, &transfer_id)?;
                if &pending.wire.requester != client
                    && pending.wire.previous_owner.as_ref() != Some(client)
                {
                    return Err(error(
                        FailureCode::PermissionDenied,
                        "only transfer participants can cancel",
                    ));
                }
                state.pending = None;
                Ok(None)
            }
            ProfileControlRequest::Release => {
                authorize_owner(&state, client)?;
                if state.pending.is_some() {
                    return Err(error(
                        FailureCode::Conflict,
                        "cancel the pending transfer before releasing control",
                    ));
                }
                let id = uuid::Uuid::new_v4().to_string();
                state.pending = Some(PendingTransfer {
                    wire: ControlTransfer {
                        id: id.clone(),
                        requester: client.clone(),
                        previous_owner: Some(client.clone()),
                        phase: TransferPhase::Fencing,
                        deadline_millis: crate::now_millis(),
                    },
                    target: None,
                    revisions: None,
                });
                Ok(Some(id))
            }
        }
    }

    /// Called with the Host mutation write fence held. Revision verification happens
    /// after all admitted writes drain, not merely when the Client sends Ready.
    pub fn finish(
        &self,
        id: &str,
        verify: impl FnOnce(&[WorkspaceRevision]) -> bool,
    ) -> Result<Option<ClientInstanceId>, ProtocolFailure> {
        let mut state = self.state.lock();
        state.revision = state.revision.saturating_add(1);
        let pending = matching(&mut state, id)?;
        if pending.wire.phase != TransferPhase::Fencing {
            return Err(error(
                FailureCode::Conflict,
                "profile transfer has not reached its write fence",
            ));
        }
        if pending
            .revisions
            .as_ref()
            .is_some_and(|revisions| !verify(revisions))
        {
            state.pending = None;
            return Err(error(
                FailureCode::Conflict,
                "workspace publication did not match durable Host revisions",
            ));
        }
        let pending = state.pending.take().expect("matching transfer");
        let previous = state.owner.take();
        state.epoch = state
            .epoch
            .checked_add(1)
            .expect("profile control epoch exhausted");
        state.owner = pending.target;
        Ok(previous)
    }

    /// The caller drains admitted writes before disconnect/revocation is made visible.
    pub fn disconnect(&self, client: &ClientInstanceId) -> Option<ClientInstanceId> {
        let mut state = self.state.lock();
        state.revision = state.revision.saturating_add(1);
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| &pending.wire.requester == client)
        {
            state.pending = None;
        }
        if state.owner.as_ref() == Some(client) {
            if let Some(pending) = &mut state.pending {
                pending.wire.phase = TransferPhase::ForceConfirmationRequired;
            }
            state.epoch = state
                .epoch
                .checked_add(1)
                .expect("profile control epoch exhausted");
            state.owner.take()
        } else {
            None
        }
    }
}

fn authorize_owner(state: &State, client: &ClientInstanceId) -> Result<(), ProtocolFailure> {
    if state.owner.as_ref() != Some(client) {
        return Err(error(
            FailureCode::PermissionDenied,
            "this Client is an observer; profile control is required",
        ));
    }
    if state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.wire.phase == TransferPhase::Fencing)
    {
        return Err(error(
            FailureCode::Conflict,
            "profile transfer is draining admitted writes",
        ));
    }
    Ok(())
}

fn matching<'a>(
    state: &'a mut State,
    id: &str,
) -> Result<&'a mut PendingTransfer, ProtocolFailure> {
    state
        .pending
        .as_mut()
        .filter(|pending| pending.wire.id == id)
        .ok_or_else(|| {
            error(
                FailureCode::Conflict,
                "profile transfer is no longer current",
            )
        })
}

fn expire_preparation(state: &mut State) {
    if let Some(pending) = &mut state.pending
        && pending.wire.phase == TransferPhase::Preparing
        && crate::now_millis() >= pending.wire.deadline_millis
    {
        pending.wire.phase = TransferPhase::ForceConfirmationRequired;
        state.revision = state.revision.saturating_add(1);
    }
}

fn snapshot(state: &State, host_epoch: u64) -> ControlStatus {
    ControlStatus {
        revision: state.revision,
        context: ControlContext {
            host_epoch,
            control_epoch: state.epoch,
        },
        owner: state.owner.clone(),
        transfer: state.pending.as_ref().map(|pending| pending.wire.clone()),
    }
}

fn error(code: FailureCode, message: &str) -> ProtocolFailure {
    ProtocolFailure::new(code, message, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acquire(control: &ProfileControl, client: &ClientInstanceId) {
        let id = control
            .request(client, ProfileControlRequest::RequestControl)
            .unwrap()
            .unwrap();
        control.finish(&id, |_| true).unwrap();
    }

    #[test]
    fn preparation_allows_flush_but_fencing_and_old_epochs_deny_writes() {
        let control = ProfileControl::new();
        let a = ClientInstanceId::new("a");
        let b = ClientInstanceId::new("b");
        acquire(&control, &a);
        let old = control.status(7).context;
        control
            .request(&b, ProfileControlRequest::RequestControl)
            .unwrap();
        control.authorize(&a, old, 7).unwrap();
        assert!(control.authorize(&b, old, 7).is_err());
        let transfer = control.status(7).transfer.unwrap();
        assert!(
            control
                .request(
                    &b,
                    ProfileControlRequest::ConfirmForce {
                        transfer_id: transfer.id.clone()
                    }
                )
                .is_err()
        );
        control
            .request(
                &a,
                ProfileControlRequest::Ready {
                    transfer_id: transfer.id.clone(),
                    revisions: vec![],
                },
            )
            .unwrap();
        assert!(control.authorize(&a, old, 7).is_err());
        control.finish(&transfer.id, |_| true).unwrap();
        assert_eq!(
            control.authorize(&a, old, 7).unwrap_err().code,
            FailureCode::StaleEpoch
        );
        control.authorize(&b, control.status(7).context, 7).unwrap();
    }

    #[test]
    fn failed_publication_cancels_transfer_without_discarding_old_control() {
        let control = ProfileControl::new();
        let a = ClientInstanceId::new("a");
        let b = ClientInstanceId::new("b");
        acquire(&control, &a);
        let before = control.status(9);
        control
            .request(&b, ProfileControlRequest::RequestControl)
            .unwrap();
        let transfer = control.status(9).transfer.unwrap();
        control
            .request(
                &a,
                ProfileControlRequest::Ready {
                    transfer_id: transfer.id.clone(),
                    revisions: vec![],
                },
            )
            .unwrap();
        assert!(control.finish(&transfer.id, |_| false).is_err());
        let after = control.status(9);
        assert_eq!(after.context, before.context);
        assert_eq!(after.owner, before.owner);
        assert!(after.transfer.is_none());
        control.authorize(&a, before.context, 9).unwrap();
    }

    #[test]
    fn expired_preparation_never_automatically_grants_control() {
        let control = ProfileControl::new();
        let a = ClientInstanceId::new("a");
        let b = ClientInstanceId::new("b");
        acquire(&control, &a);
        control
            .request(&b, ProfileControlRequest::RequestControl)
            .unwrap();
        control
            .state
            .lock()
            .pending
            .as_mut()
            .unwrap()
            .wire
            .deadline_millis = 0;
        let status = control.status(1);
        assert_eq!(status.owner, Some(a));
        let transfer = status.transfer.unwrap();
        assert_eq!(transfer.phase, TransferPhase::ForceConfirmationRequired);
        assert!(control.authorize_owner(&b).is_err());
        let id = control
            .request(
                &b,
                ProfileControlRequest::ConfirmForce {
                    transfer_id: transfer.id,
                },
            )
            .unwrap()
            .unwrap();
        control.finish(&id, |_| false).unwrap(); // Forced transfer uses the last durable state.
        assert_eq!(control.status(1).owner, Some(b));
    }
}
