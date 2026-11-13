//! The remote services window on fixture state, for `yttt-ui-gallery`
//! screenshots: no Host, control plane or network is touched.

use super::auxiliary_windows::RemoteServicesPage;
use super::*;
use yttt_protocol::{
    EndpointKey,
    p2p::{DevicePreset, PairedDevice},
    remote_access::{
        P2pSettings, P2pStatus, RelayAccountStatus, RelayNode, RemoteAccessSettings,
        RemoteAccessState, RemoteAccessStatus, RemoteClientSummary,
    },
};

/// Which page of the window, in which state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemotePreview {
    /// The saved connections list.
    Connections,
    /// This computer, never set up: P2P off, no account.
    ThisComputerFresh,
    /// This computer with P2P on, a relay account, a node and two devices.
    ThisComputerConnected,
    /// This computer whose node was disabled: it asks for another.
    ThisComputerNeedsNode,
}

/// Renders the remote services window of a workbench.
pub struct RemoteServicesPreview(Entity<WorkbenchView>);

impl Render for RemoteServicesPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.0
                .update(cx, |root, cx| remote_connections::render(root, window, cx)),
        )
    }
}

impl RemoteServicesPreview {
    pub fn new(mut root: WorkbenchView, preview: RemotePreview, cx: &mut App) -> Self {
        root.ssh.manager_open = true;
        root.ssh.preview = true;
        root.auxiliary_windows.remote_page = match preview {
            RemotePreview::Connections => RemoteServicesPage::Connections,
            _ => RemoteServicesPage::ThisComputer,
        };
        if preview != RemotePreview::Connections {
            fill_this_computer(&mut root, preview);
        }
        Self(cx.new(|_| root))
    }
}

fn key(byte: u8) -> EndpointKey {
    EndpointKey([byte; 32])
}

fn fill_this_computer(root: &mut WorkbenchView, preview: RemotePreview) {
    let fresh = preview == RemotePreview::ThisComputerFresh;
    let needs_node = preview == RemotePreview::ThisComputerNeedsNode;
    let relay_account = (!fresh).then(|| {
        Box::new(RelayAccountStatus {
            control_url: "https://control.example.com".into(),
            account_id: 7,
            relay_url: (!needs_node).then(|| "https://tokyo-1.relay.example.com".into()),
            ticket_expires_unix: None,
            error: None,
            node_id: (!needs_node).then(|| "tokyo-1".into()),
            placement_needed: needs_node.then(|| yttt_control_client::PlacementNeeded {
                reason: yttt_control_client::PlacementReason::NodeUnavailable,
                current: yttt_control_client::SignedPlacement::sign(
                    &[9; 32],
                    3,
                    "tokyo-1",
                    "https://tokyo-1.relay.example.com",
                )
                .ok(),
                suggested: None,
            }),
        })
    });
    root.ssh.remote_access = Some(RemoteAccessStatus {
        settings: RemoteAccessSettings {
            enabled: false,
            listen_address: "127.0.0.1:43123".parse().expect("constant address"),
            login_startup_consent_granted: false,
            p2p: P2pSettings { enabled: !fresh },
        },
        effective: RemoteAccessState::Disabled,
        bound_address: None,
        error: None,
        clients: if fresh {
            Vec::new()
        } else {
            vec![RemoteClientSummary {
                client_id: yttt_core::model::ids::ClientInstanceId::new(
                    "mobile-client-6f2d9a41c3b07e55",
                ),
                connected_millis: 0,
                streams: 3,
                device: Some(key(2)),
            }]
        },
        p2p: P2pStatus {
            effective: if fresh {
                RemoteAccessState::Disabled
            } else {
                RemoteAccessState::Listening
            },
            endpoint_id: (!fresh).then(|| key(1)),
            error: None,
            pairing: None,
            relay_account,
        },
    });
    root.ssh.remote_access_loaded = true;
    if !fresh {
        root.ssh.p2p.devices = vec![
            PairedDevice {
                endpoint_id: key(2),
                label: "iPhone 16 Pro".into(),
                preset: DevicePreset::Full,
                paired_at_millis: 1_762_000_000_000,
                last_seen_millis: Some(1_763_000_000_000),
            },
            PairedDevice {
                endpoint_id: key(3),
                label: "MacBook Air".into(),
                preset: DevicePreset::Observe,
                paired_at_millis: 1_761_000_000_000,
                last_seen_millis: None,
            },
        ];
        root.ssh.relay_account.display = Some(crate::relay_account::AccountDisplay {
            control_url: "https://control.example.com".into(),
            email: "me@example.com".into(),
            account_id: 7,
        });
        root.ssh.relay_account.view = Some(yttt_control_client::AccountView {
            account_id: 7,
            email: "me@example.com".into(),
            plan: "basic".into(),
            plan_name: "基础版".into(),
            node_id: "tokyo-1".into(),
            relay_url: "https://tokyo-1.relay.example.com".into(),
            month_upstream_bytes: 734_003_200,
            monthly_bytes: 50 << 30,
            devices: 3,
            max_devices: 10,
            ..Default::default()
        });
        root.ssh.relay_account.nodes = Some(vec![
            node("tokyo-1", "东京 1", "日本", 38, Some(42)),
            node("hk-1", "香港 1", "中国香港", 61, Some(18)),
            node("sg-1", "新加坡 1", "新加坡", 85, Some(100)),
        ]);
        root.ssh.relay_account.choosing_node = needs_node;
    }
    root.ssh.relay_account.loaded = true;
}

fn node(id: &str, name: &str, region: &str, latency: u32, load: Option<u32>) -> RelayNode {
    RelayNode {
        id: id.into(),
        name: name.into(),
        region: region.into(),
        relay_url: format!("https://{id}.relay.example.com"),
        load_percent: load,
        full: load == Some(100),
        latency_ms: Some(latency),
    }
}
