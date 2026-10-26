//! Tailscale: Outpost's address on the owner's private Tailscale network, so phones
//! (and Hermes) can reach it away from home without opening anything to the internet.
//!
//! Tailscale gives each machine an address in 100.64.0.0/10 and, with MagicDNS, a name
//! like `desktop-dan.tail1234.ts.net`. Both are found here; nothing is changed in
//! Tailscale itself.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct TailscaleInfo {
    /// This PC's Tailscale address.
    pub ip: String,
    /// Its MagicDNS name, when the Tailscale command can tell us.
    pub dns_name: Option<String>,
}

/// Is this address in Tailscale's range (100.64.0.0/10)?
pub fn is_tailscale(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

/// This PC's Tailscale address, from its network interfaces.
pub fn ip() -> Option<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) if is_tailscale(v4) => Some(v4),
            _ => None,
        })
        .next()
}

/// The MagicDNS name, from `tailscale status --json` (best effort, a few seconds at most).
pub async fn dns_name() -> Option<String> {
    let candidates = [
        "tailscale".to_string(),
        r"C:\Program Files\Tailscale\tailscale.exe".to_string(),
        "/usr/bin/tailscale".to_string(),
    ];
    for exe in candidates {
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.args(["status", "--json"]).kill_on_drop(true);
        #[cfg(windows)]
        {
            // No console window flashing up.
            cmd.creation_flags(0x0800_0000);
        }
        let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(3), cmd.output()).await else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        if let Some(name) = dns_name_from_status(&out.stdout) {
            return Some(name);
        }
    }
    None
}

fn dns_name_from_status(json: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let name = v.pointer("/Self/DNSName")?.as_str()?.trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_string())
}

/// What's known about Tailscale on this PC right now.
pub async fn detect() -> Option<TailscaleInfo> {
    let ip = ip()?;
    Some(TailscaleInfo {
        ip: ip.to_string(),
        dns_name: dns_name().await,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_range() {
        assert!(is_tailscale(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_tailscale(Ipv4Addr::new(100, 101, 7, 9)));
        assert!(is_tailscale(Ipv4Addr::new(100, 127, 255, 254)));
        assert!(!is_tailscale(Ipv4Addr::new(100, 63, 0, 1)));
        assert!(!is_tailscale(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!is_tailscale(Ipv4Addr::new(192, 168, 1, 20)));
    }

    #[test]
    fn magic_dns_name() {
        let status = br#"{"Self": {"DNSName": "desktop-dan.tail1234.ts.net.", "TailscaleIPs": ["100.101.7.9"]}}"#;
        assert_eq!(
            dns_name_from_status(status).as_deref(),
            Some("desktop-dan.tail1234.ts.net")
        );
        assert_eq!(dns_name_from_status(br#"{"Self": {"DNSName": ""}}"#), None);
        assert_eq!(dns_name_from_status(b"not json"), None);
    }
}
