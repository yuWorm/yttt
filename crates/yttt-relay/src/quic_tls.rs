//! A certificate for QUIC address discovery, read from files another
//! program keeps current: a node behind a reverse proxy, such as Caddy on a
//! machine shared with the control plane, serves HTTP in the clear behind
//! it, and only QUIC needs the certificate.
//!
//! The files are read again when they change. Until they exist, QUIC
//! handshakes fail and devices fall back to the relay alone.

use std::{
    fmt,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use parking_lot::RwLock;
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use tokio_util::sync::CancellationToken;

use crate::config::TlsFiles;

const CHECK_EVERY: Duration = Duration::from_secs(60);

/// The current certificate from `files`, replaced when they change.
pub(crate) struct Reloading {
    files: TlsFiles,
    current: RwLock<Option<(Arc<CertifiedKey>, Option<SystemTime>)>>,
}

impl fmt::Debug for Reloading {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Reloading")
            .field("cert_chain", &self.files.cert_chain)
            .finish_non_exhaustive()
    }
}

impl Reloading {
    pub(crate) fn new(files: TlsFiles) -> Arc<Self> {
        let reloading = Arc::new(Self {
            files,
            current: RwLock::new(None),
        });
        reloading.reload();
        reloading
    }

    /// Reads the files again if the chain changed since the last read.
    pub(crate) fn reload(&self) {
        let modified = modified(&self.files.cert_chain);
        if modified.is_none() {
            return;
        }
        if let Some((_, seen)) = &*self.current.read()
            && *seen == modified
        {
            return;
        }
        match load(&self.files) {
            Ok(key) => *self.current.write() = Some((Arc::new(key), modified)),
            // A half-written renewal; the next check reads it again.
            Err(error) => tracing::warn!(%error, "could not read the QUIC certificate"),
        }
    }

    pub(crate) fn loaded(&self) -> bool {
        self.current.read().is_some()
    }

    /// A rustls configuration that answers with the current certificate.
    pub(crate) fn server_config(self: &Arc<Self>) -> Arc<rustls::ServerConfig> {
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .with_no_client_auth()
        .with_cert_resolver(self.clone());
        Arc::new(config)
    }

    /// Checks the files every minute until `cancel`.
    pub(crate) async fn watch(self: Arc<Self>, cancel: CancellationToken) {
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(CHECK_EVERY) => self.reload(),
            }
        }
    }
}

impl ResolvesServerCert for Reloading {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.current.read().as_ref().map(|(key, _)| key.clone())
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn load(files: &TlsFiles) -> Result<CertifiedKey, String> {
    let chain = CertificateDer::pem_file_iter(&files.cert_chain)
        .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
        .map_err(|error| format!("{}: {error}", files.cert_chain.display()))?;
    let key = PrivateKeyDer::from_pem_file(&files.private_key)
        .map_err(|error| format!("{}: {error}", files.private_key.display()))?;
    let signing = rustls::crypto::ring::default_provider()
        .key_provider
        .load_private_key(key)
        .map_err(|error| format!("{}: {error}", files.private_key.display()))?;
    Ok(CertifiedKey::new(chain, signing))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_cert(dir: &Path, name: &str) {
        let cert = rcgen::generate_simple_self_signed(vec![name.to_owned()]).unwrap();
        std::fs::write(dir.join("cert.pem"), cert.cert.pem()).unwrap();
        std::fs::write(dir.join("key.pem"), cert.key_pair.serialize_pem()).unwrap();
    }

    #[test]
    fn the_certificate_arrives_later_and_is_replaced_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let files = TlsFiles {
            cert_chain: dir.path().join("cert.pem"),
            private_key: dir.path().join("key.pem"),
        };
        let reloading = Reloading::new(files);
        assert!(!reloading.loaded(), "nothing yet");

        write_cert(dir.path(), "relay.example");
        reloading.reload();
        let first = reloading.current.read().as_ref().unwrap().0.clone();

        // A renewal: same paths, new certificate, later modification time.
        std::thread::sleep(Duration::from_millis(1100));
        write_cert(dir.path(), "relay.example");
        reloading.reload();
        let second = reloading.current.read().as_ref().unwrap().0.clone();
        assert_ne!(first.cert, second.cert);

        // Unchanged files are not read again.
        reloading.reload();
        assert!(Arc::ptr_eq(
            &second,
            &reloading.current.read().as_ref().unwrap().0
        ));
    }
}
