//! The client side of a remote Outpost: how the desktop app uses an Outpost server on
//! another machine (a home server with no screen) instead of the one on this PC.
//!
//! The server's encrypted port has its own certificate that nobody vouches for, so, like
//! the phone app, we **pin** it: the first look ([`probe`]) shows its fingerprint as three
//! check words, which the person compares with the words the server shows
//! (`outpost-gateway --setup-code`, or Settings › Network on a device already paired).
//! After that only that exact certificate is accepted.
//!
//! The desktop window then talks to [`tunnel`]: a port on 127.0.0.1 that forwards each
//! connection, byte for byte, over a pinned TLS connection to the server. The window keeps
//! speaking plain HTTP to "its" gateway, so streaming replies, files and voice work the same.
//! The tunnel adds no access of its own: the server sees an ordinary encrypted
//! connection from this PC, which still needs a paired device's token for anything private.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

/// The encrypted port a server uses unless the address says otherwise.
pub const DEFAULT_SECURE_PORT: u16 = 7421;

/// What the first look at a server found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub host: String,
    pub port: u16,
    /// SHA-256 of the certificate it showed (lowercase hex).
    pub fingerprint: String,
    /// The fingerprint as three words, to compare with the server's.
    pub words: Vec<String>,
    /// Its `/api/health`: name, version, and whether it still needs an owner.
    pub health: serde_json::Value,
}

/// `192.168.1.98`, `outpost.local:7421`, `[fe80::1]:7421`, or a pasted
/// `https://host:port/...` → host and port.
pub fn parse_address(input: &str) -> anyhow::Result<(String, u16)> {
    let mut s = input.trim();
    for scheme in ["https://", "http://", "outpost://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest;
        }
    }
    let s = s.split(['/', '?', '#']).next().unwrap_or("").trim();
    if s.is_empty() {
        bail!("Type the server's address, for example 192.168.1.98.");
    }
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        // [IPv6]:port
        let (host, after) = rest.split_once(']').ok_or_else(|| anyhow!("That address isn't complete."))?;
        let port = after.strip_prefix(':').map(str::to_string);
        (host.to_string(), port)
    } else if s.matches(':').count() == 1 {
        let (h, p) = s.split_once(':').unwrap_or((s, ""));
        (h.to_string(), Some(p.to_string()))
    } else {
        // A name, an IPv4 address, or a bare IPv6 address.
        (s.to_string(), None)
    };
    let port = match port {
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| anyhow!("The port after \":\" must be a number, like 7421."))?,
        None => DEFAULT_SECURE_PORT,
    };
    if host.is_empty() || host.chars().any(|c| c.is_whitespace()) {
        bail!("That doesn't look like an address.");
    }
    Ok((host, port))
}

/// The three check words for a fingerprint (the same ones phones and the server show).
pub fn words(fingerprint: &str) -> Vec<String> {
    outpost_core::pairing::words(fingerprint).iter().map(|w| w.to_string()).collect()
}

/// Accepts exactly one certificate (by SHA-256), or, with no pin, any certificate while
/// noting which one it saw. Either way the server must prove it holds the certificate's
/// key: the handshake signature is checked as usual.
#[derive(Debug)]
struct Pinned {
    expect: Option<String>,
    seen: std::sync::Mutex<Option<String>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = crate::tls::fingerprint(end_entity.as_ref());
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(fp.clone());
        }
        match &self.expect {
            Some(want) if !same(want, &fp) => Err(rustls::Error::General(
                "this isn't the Outpost server you paired with (its certificate is different)".into(),
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn connector(expect: Option<&str>) -> anyhow::Result<(TlsConnector, Arc<Pinned>)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(Pinned {
        expect: expect.map(str::to_string),
        seen: Default::default(),
        provider: provider.clone(),
    });
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    // The tunnel passes bytes through untouched, and the window speaks HTTP/1.1.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((TlsConnector::from(Arc::new(config)), verifier))
}

fn server_name(host: &str) -> ServerName<'static> {
    // Outpost's certificate isn't checked against the name (it's pinned), but TLS needs one.
    ServerName::try_from(host.to_string()).unwrap_or_else(|_| ServerName::try_from("outpost.local").expect("name"))
}

async fn connect(host: &str, port: u16, tls: &TlsConnector) -> anyhow::Result<TlsStream<TcpStream>> {
    connect_within(host, port, tls, Duration::from_secs(6)).await
}

async fn connect_within(host: &str, port: u16, tls: &TlsConnector, wait: Duration) -> anyhow::Result<TlsStream<TcpStream>> {
    let tcp = tokio::time::timeout(wait, TcpStream::connect((host, port)))
        .await
        .map_err(|_| anyhow!("{host} didn't answer on port {port}."))?
        .with_context(|| format!("Couldn't reach {host} on port {port}"))?;
    let _ = tcp.set_nodelay(true);
    let stream = tokio::time::timeout(Duration::from_secs(8), tls.connect(server_name(host), tcp))
        .await
        .map_err(|_| anyhow!("{host} answered but didn't finish the secure connection."))??;
    Ok(stream)
}

/// Have a first look at a server: connect, note its certificate, and ask for its health.
pub async fn probe(address: &str) -> anyhow::Result<Probe> {
    let (host, port) = parse_address(address)?;
    let (tls, verifier) = connector(None)?;
    let mut stream = connect(&host, port, &tls).await.map_err(|e| {
        anyhow!(
            "{e:#}. Check the address, that Outpost is running there with network access on, \
             and that its firewall allows port {port}."
        )
    })?;
    let fingerprint = verifier
        .seen
        .lock()
        .ok()
        .and_then(|s| s.clone())
        .ok_or_else(|| anyhow!("The server didn't show a certificate."))?;
    let health = get_json(&mut stream, &host, "/api/health")
        .await
        .context("That address answered, but it doesn't look like Outpost")?;
    if health.get("name").is_none() || health.get("setup_required").is_none() {
        bail!("That address answered, but it doesn't look like Outpost.");
    }
    Ok(Probe {
        words: words(&fingerprint),
        host,
        port,
        fingerprint,
        health,
    })
}

/// A minimal `GET` over a fresh connection (the server answers JSON with a length).
async fn get_json(stream: &mut TlsStream<TcpStream>, host: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(8), async {
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                // Some servers close without a TLS goodbye; what arrived is still good.
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            if buf.len() > 256 * 1024 {
                break;
            }
        }
        Ok(())
    })
    .await;
    if !matches!(read, Ok(Ok(()))) && buf.is_empty() {
        bail!("no reply");
    }
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| anyhow!("not an HTTP reply"))?;
    let status_ok = head.lines().next().is_some_and(|l| l.split_whitespace().nth(1) == Some("200"));
    if !status_ok {
        bail!("unexpected reply: {}", head.lines().next().unwrap_or(""));
    }
    Ok(serde_json::from_str(body.trim())?)
}

/// The server's addresses, best first (the home network, then Tailscale), shared with the
/// tunnel so new ones apply at once.
pub type Hosts = Arc<std::sync::RwLock<Vec<String>>>;

/// How long the home address gets before falling back to the next one (Tailscale).
const QUICK_TRY: Duration = Duration::from_millis(1500);
/// After an address fails, skip it this long (away from home, every request would
/// otherwise wait for it first).
const SKIP_FAILED: Duration = Duration::from_secs(30);

/// Which addresses recently failed, so the next try goes straight to one that works.
#[derive(Default)]
struct Failures(std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>);

impl Failures {
    fn recent(&self, host: &str) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|f| f.get(host).copied())
            .is_some_and(|t| t.elapsed() < SKIP_FAILED)
    }
    fn failed(&self, host: &str) {
        if let Ok(mut f) = self.0.lock() {
            f.insert(host.to_string(), std::time::Instant::now());
        }
    }
    fn worked(&self, host: &str) {
        if let Ok(mut f) = self.0.lock() {
            f.remove(host);
        }
    }
}

/// Reach the server at the first address that answers, in order. Every address but the
/// last gets a short time; recently failed ones are tried last.
async fn connect_any(hosts: &[String], port: u16, tls: &TlsConnector, failures: &Failures) -> anyhow::Result<TlsStream<TcpStream>> {
    let mut order: Vec<&String> = hosts.iter().filter(|h| !failures.recent(h)).collect();
    order.extend(hosts.iter().filter(|h| failures.recent(h)));
    let mut last = anyhow!("This app has no address for the server.");
    for (i, host) in order.iter().enumerate() {
        let wait = if i + 1 == order.len() { Duration::from_secs(6) } else { QUICK_TRY };
        match connect_within(host, port, tls, wait).await {
            Ok(s) => {
                failures.worked(host);
                return Ok(s);
            }
            Err(e) => {
                failures.failed(host);
                last = e;
            }
        }
    }
    Err(last)
}

/// Forward every connection made to `listener` (on 127.0.0.1) to the server, over TLS that
/// only accepts the pinned certificate, at whichever of its addresses answers (the home
/// network first, then Tailscale). Runs until the task is dropped.
pub async fn tunnel(listener: TcpListener, hosts: Hosts, port: u16, fingerprint: String) {
    let tls = match connector(Some(&fingerprint)) {
        Ok((tls, _)) => tls,
        Err(e) => {
            tracing::error!(error = %e, "couldn't prepare the secure connection to the server");
            return;
        }
    };
    let failures = Arc::new(Failures::default());
    loop {
        let (mut local, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "server tunnel: accept failed");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        // Only this PC (the listener is on 127.0.0.1 anyway).
        if !peer.ip().is_loopback() {
            continue;
        }
        let list: Vec<String> = hosts.read().map(|h| h.clone()).unwrap_or_default();
        let (tls, failures) = (tls.clone(), failures.clone());
        tokio::spawn(async move {
            let _ = local.set_nodelay(true);
            match connect_any(&list, port, &tls, &failures).await {
                Ok(mut remote) => {
                    let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
                }
                Err(e) => {
                    let tried = list.join(" or ");
                    tracing::warn!(error = %format!("{e:#}"), %tried, port, "couldn't reach the Outpost server");
                    // Tell the window plainly, rather than just dropping the connection.
                    let msg = serde_json::json!({ "error": {
                        "code": "server_unreachable",
                        "message": format!("Can't reach the Outpost server at {tried}. {e:#}"),
                    }})
                    .to_string();
                    let reply = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                         Access-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{msg}",
                        msg.len()
                    );
                    let _ = local.write_all(reply.as_bytes()).await;
                    let _ = local.shutdown().await;
                }
            }
        });
    }
}

/// Start a tunnel on a free port on 127.0.0.1; returns its address.
pub async fn start_tunnel(
    host: String,
    port: u16,
    fingerprint: String,
) -> anyhow::Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    let hosts: Hosts = Arc::new(std::sync::RwLock::new(vec![host]));
    Ok((addr, tokio::spawn(tunnel(listener, hosts, port, fingerprint))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_people_type() {
        assert_eq!(parse_address("192.168.1.98").unwrap(), ("192.168.1.98".into(), 7421));
        assert_eq!(parse_address(" outpost.local:8000 ").unwrap(), ("outpost.local".into(), 8000));
        assert_eq!(parse_address("https://10.0.0.5:7421/api").unwrap(), ("10.0.0.5".into(), 7421));
        assert_eq!(parse_address("[fe80::1]:7421").unwrap(), ("fe80::1".into(), 7421));
        assert_eq!(parse_address("fe80::1").unwrap(), ("fe80::1".into(), 7421));
        assert!(parse_address("").is_err());
        assert!(parse_address("host:port").is_err());
        assert!(parse_address("my server").is_err());
    }

    #[test]
    fn a_failed_address_is_skipped_for_a_while() {
        let f = Failures::default();
        assert!(!f.recent("192.168.1.98"));
        f.failed("192.168.1.98");
        assert!(f.recent("192.168.1.98"));
        f.worked("192.168.1.98");
        assert!(!f.recent("192.168.1.98"));
    }

    #[test]
    fn pins_compare_ignoring_case() {
        assert!(same("ABcd", "abcd"));
        assert!(!same("abcd", "abce"));
        assert!(!same("abc", "abcd"));
    }
}
