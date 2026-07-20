use crate::runtime::update::UpdateInfo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Failed(String),
}

#[derive(Debug)]
pub(in super::super) struct UpdateControllerState {
    pub(in super::super) status: UpdateStatus,
}

impl Default for UpdateControllerState {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
        }
    }
}
