//! Probes a remote Host session when the system wakes or the network changes.
//! Either can leave a connection that looks open but reaches nothing, and a
//! monotonic clock that stopped during sleep would otherwise make the Client
//! wait out its whole liveness timeout before it reconnects.

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use n0_watcher::Watcher as _;
use yttt_client_core::ClientCore;

/// How often the wall clock is compared with the timers.
const WAKE_TICK: Duration = Duration::from_secs(2);
/// How far past a tick the wall clock must be before it counts as a sleep;
/// a busy runtime delays a tick by far less.
const WAKE_SLACK: Duration = Duration::from_secs(8);

/// Probes `client` after each system wake and each major network change,
/// noting why in `client.log`, until the Client stops.
pub(crate) fn follow(client: Arc<ClientCore>, runtime: &tokio::runtime::Handle, label: String) {
    let weak = Arc::downgrade(&client);
    runtime.spawn({
        let (weak, label) = (weak.clone(), label.clone());
        async move {
            loop {
                let before = SystemTime::now();
                tokio::time::sleep(WAKE_TICK).await;
                let Some(client) = weak.upgrade() else {
                    return;
                };
                let wall = SystemTime::now().duration_since(before).unwrap_or_default();
                if slept(wall) {
                    probe(&client, &label, "the system woke");
                }
            }
        }
    });
    runtime.spawn(async move {
        let Ok(monitor) = netwatch::netmon::Monitor::new().await else {
            return;
        };
        let mut state = monitor.interface_state();
        let mut current = state.get();
        while let Ok(next) = state.updated().await {
            let major = next.is_major_change(&current);
            current = next;
            let Some(client) = weak.upgrade() else {
                return;
            };
            if major {
                probe(&client, &label, "the network changed");
            }
        }
    });
}

/// Whether a tick that `wall` took by the wall clock spanned a sleep:
/// timers stop while the system sleeps, the wall clock does not.
fn slept(wall: Duration) -> bool {
    wall > WAKE_TICK + WAKE_SLACK
}

fn probe(client: &ClientCore, label: &str, why: &str) {
    client.probe_now();
    crate::connection_log::note(label, &format!("{why}; checking the Host link"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_tick_far_longer_than_its_timer_counts_as_a_sleep() {
        assert!(!slept(WAKE_TICK));
        assert!(!slept(WAKE_TICK + WAKE_SLACK));
        assert!(slept(WAKE_TICK + WAKE_SLACK + Duration::from_millis(1)));
        assert!(slept(Duration::from_secs(3600)));
    }
}
