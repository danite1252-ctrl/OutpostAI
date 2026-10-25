//! The encrypted port for phones (7421 by default, next to the plain API port).
//!
//! Outpost makes its own certificate the first time (kept in `<data>/tls/`). Nobody
//! vouches for it, so phones don't check it against a certificate authority: they
//! pin its SHA-256 fingerprint when they pair (from the QR code, or after the person
//! checks the three words on both screens), and refuse any other certificate later.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::ConnectInfo;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// Marks a request that came in over the encrypted port.
#[derive(Debug, Clone, Copy)]
pub struct Secure;

/// Outpost's certificate and key.
pub struct Identity {
    pub cert_der: Vec<u8>,
    key_der: Vec<u8>,
    /// SHA-256 of the certificate, lowercase hex: what phones pin.
    pub fingerprint: String,
}

/// What phones need to know about the encrypted port.
#[derive(Debug, Clone)]
pub struct TlsInfo {
    pub port: u16,
    pub fingerprint: String,
}

pub fn fingerprint(cert_der: &[u8]) -> String {
    Sha256::digest(cert_der).iter().map(|b| format!("{b:02x}")).collect()
}

/// Load Outpost's certificate, or make one the first time.
pub fn load_or_create(data_dir: &Path) -> anyhow::Result<Identity> {
    let dir = data_dir.join("tls");
    let (cert_path, key_path) = (dir.join("outpost-cert.der"), dir.join("outpost-key.der"));
    if let (Ok(cert_der), Ok(key_der)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        return Ok(Identity {
            fingerprint: fingerprint(&cert_der),
            cert_der,
            key_der,
        });
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let made = rcgen::generate_simple_self_signed(vec!["outpost.local".to_string(), "localhost".to_string()])
        .context("making Outpost's certificate")?;
    let cert_der = made.cert.der().to_vec();
    let key_der = made.key_pair.serialize_der();
    write_private(&key_path, &key_der).context("saving Outpost's key")?;
    std::fs::write(&cert_path, &cert_der).context("saving Outpost's certificate")?;
    tracing::info!(fingerprint = %fingerprint(&cert_der), "made Outpost's certificate for phones");
    Ok(Identity {
        fingerprint: fingerprint(&cert_der),
        cert_der,
        key_der,
    })
}

/// Write the private key readable by this user only. (On Windows the data folder is
/// already in the user's own profile, which other users can't read.)
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

pub fn server_config(id: &Identity) -> anyhow::Result<Arc<rustls::ServerConfig>> {
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(id.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone())),
        )?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Serve `app` over TLS on `listener` until the task is dropped.
pub async fn serve(listener: TcpListener, config: Arc<rustls::ServerConfig>, app: Router) {
    let acceptor = TlsAcceptor::from(config);
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "secure port: accept failed");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let (acceptor, app) = (acceptor.clone(), app.clone());
        tokio::spawn(async move {
            // A client that never finishes the handshake doesn't hold a connection open.
            let tls = match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await {
                Ok(Ok(tls)) => tls,
                Ok(Err(e)) => {
                    tracing::debug!(%peer, error = %e, "secure port: handshake failed");
                    return;
                }
                Err(_) => return,
            };
            let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let app = app.clone();
                async move {
                    let mut req = req.map(axum::body::Body::new);
                    req.extensions_mut().insert(ConnectInfo(peer));
                    req.extensions_mut().insert(Secure);
                    app.oneshot(req).await
                }
            });
            if let Err(e) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(TokioIo::new(tls), service)
                .await
            {
                tracing::debug!(%peer, error = %e, "secure port: connection ended");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("outpost-tls-{}", uuid_like()));
        let a = load_or_create(&dir).unwrap();
        let b = load_or_create(&dir).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 64);
        assert!(server_config(&a).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn uuid_like() -> String {
        format!("{}-{:?}", std::process::id(), std::time::SystemTime::now())
            .replace([' ', ':', '{', '}', ',', '.'], "")
    }
}
