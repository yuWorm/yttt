//! Why a connection runs where it does, for the log when it stays on the relay.

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

/// What a connection's endpoint advertises and which paths the connection holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathReport {
    /// Addresses this endpoint gives peers to dial: its interfaces and any
    /// public address QUIC address discovery found through the relay.
    pub advertised: Vec<SocketAddr>,
    pub paths: Vec<PathSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathSummary {
    /// The peer's address on this path, or the relay's URL.
    pub remote: String,
    pub relay: bool,
    pub selected: bool,
    pub rtt: Duration,
}

/// What an address says about reaching this endpoint directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressKind {
    Public,
    /// A home, office or carrier network; a peer elsewhere needs hole punching.
    Private,
    /// A proxy's TUN mode (such as 198.18.0.0/15 fake-ip): UDP goes through
    /// the proxy, so address discovery and hole punching fail.
    ProxyTun,
    Local,
}

pub fn address_kind(address: IpAddr) -> AddressKind {
    match address {
        IpAddr::V4(address) => {
            let [first, second, ..] = address.octets();
            if address.is_loopback() || address.is_link_local() || address.is_unspecified() {
                AddressKind::Local
            } else if first == 198 && (second & 0xfe) == 18 {
                AddressKind::ProxyTun
            } else if address.is_private() || (first == 100 && (second & 0xc0) == 64) {
                AddressKind::Private
            } else {
                AddressKind::Public
            }
        }
        IpAddr::V6(address) => {
            let first = address.segments()[0];
            if address.is_loopback() || address.is_unspecified() || (first & 0xffc0) == 0xfe80 {
                AddressKind::Local
            } else if (first & 0xfe00) == 0xfc00 {
                AddressKind::Private
            } else {
                AddressKind::Public
            }
        }
    }
}

impl PathReport {
    /// The likeliest reason the connection has no direct path, if one shows.
    pub fn hint(&self) -> Option<&'static str> {
        let kinds = self
            .advertised
            .iter()
            .map(|address| address_kind(address.ip()))
            .collect::<Vec<_>>();
        if self.paths.iter().any(|path| !path.relay && path.selected) {
            return None;
        }
        if kinds.contains(&AddressKind::ProxyTun) {
            return Some(
                "a proxy's TUN mode (198.18.x.x fake-ip) carries UDP; exempt the relay and the peer, or turn TUN off",
            );
        }
        if !kinds.contains(&AddressKind::Public) {
            return Some(
                "no public address was discovered: UDP to the relay's QUIC port is blocked, or the relay has no domain",
            );
        }
        if self.paths.iter().all(|path| path.relay) {
            return Some(
                "no direct path was tried: the peer gave no address this device could reach",
            );
        }
        Some(
            "direct paths were tried but none carries data: a NAT on either side may block hole punching",
        )
    }
}

impl fmt::Display for PathReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("advertises ")?;
        if self.advertised.is_empty() {
            formatter.write_str("nothing")?;
        }
        for (index, address) in self.advertised.iter().enumerate() {
            let kind = match address_kind(address.ip()) {
                AddressKind::Public => "public",
                AddressKind::Private => "private",
                AddressKind::ProxyTun => "proxy TUN",
                AddressKind::Local => "local",
            };
            let separator = if index == 0 { "" } else { ", " };
            write!(formatter, "{separator}{address} ({kind})")?;
        }
        formatter.write_str("; paths ")?;
        for (index, path) in self.paths.iter().enumerate() {
            let separator = if index == 0 { "" } else { ", " };
            let kind = if path.relay { "relay" } else { "direct" };
            let selected = if path.selected { ", selected" } else { "" };
            write!(
                formatter,
                "{separator}{kind} {} ({} ms{selected})",
                path.remote,
                path.rtt.as_millis()
            )?;
        }
        if let Some(hint) = self.hint() {
            write!(formatter, "; {hint}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(selected: bool) -> PathSummary {
        PathSummary {
            remote: "https://relay.example/".into(),
            relay: true,
            selected,
            rtt: Duration::from_millis(80),
        }
    }

    fn report(advertised: &[&str], paths: Vec<PathSummary>) -> PathReport {
        PathReport {
            advertised: advertised
                .iter()
                .map(|address| address.parse().unwrap())
                .collect(),
            paths,
        }
    }

    #[test]
    fn addresses_are_classified_by_what_they_say_about_reachability() {
        for (address, kind) in [
            ("8.8.8.8", AddressKind::Public),
            ("192.168.1.5", AddressKind::Private),
            ("10.0.0.2", AddressKind::Private),
            ("100.64.3.1", AddressKind::Private),
            ("198.18.0.7", AddressKind::ProxyTun),
            ("198.19.255.1", AddressKind::ProxyTun),
            ("198.20.0.1", AddressKind::Public),
            ("127.0.0.1", AddressKind::Local),
            ("fd00::1", AddressKind::Private),
            ("fe80::1", AddressKind::Local),
            ("2001:db8::1", AddressKind::Public),
        ] {
            assert_eq!(address_kind(address.parse().unwrap()), kind, "{address}");
        }
    }

    #[test]
    fn the_hint_names_why_a_connection_stays_on_the_relay() {
        let tun = report(&["198.18.0.7:4000"], vec![relay(true)]);
        assert!(tun.hint().unwrap().contains("TUN"));
        let private = report(&["192.168.1.5:4000"], vec![relay(true)]);
        assert!(private.hint().unwrap().contains("no public address"));
        let untried = report(&["203.0.113.9:4000"], vec![relay(true)]);
        assert!(untried.hint().unwrap().contains("no direct path was tried"));
        let mut tried = untried.clone();
        tried.paths.push(PathSummary {
            remote: "198.51.100.2:5000".into(),
            relay: false,
            selected: false,
            rtt: Duration::from_millis(40),
        });
        assert!(tried.hint().unwrap().contains("hole punching"));
        tried.paths[1].selected = true;
        tried.paths[0].selected = false;
        assert_eq!(tried.hint(), None);
        assert_eq!(
            private.to_string(),
            "advertises 192.168.1.5:4000 (private); paths relay https://relay.example/ (80 ms, selected); \
             no public address was discovered: UDP to the relay's QUIC port is blocked, or the relay has no domain"
        );
    }
}
