//! Announces this Outpost on the home network (mDNS, `_outpost._tcp`) so the phone
//! app can list it without typing an address. Only the name, version, ports and a
//! rough hardware tier are shared; nothing about accounts or data.

use std::collections::HashMap;
use std::net::IpAddr;

use mdns_sd::{ServiceDaemon, ServiceInfo};

pub const SERVICE_TYPE: &str = "_outpost._tcp.local.";

/// A name safe for an mDNS host name: letters, digits and dashes.
pub fn host_label(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "outpost".into()
    } else {
        out.chars().take(60).collect()
    }
}

/// Start announcing. Keep the returned daemon alive for as long as it should be.
pub fn advertise(name: &str, ip: IpAddr, secure_port: u16, api_port: u16, tier: &str) -> anyhow::Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new()?;
    let mut props = HashMap::new();
    props.insert("v".to_string(), "1".to_string());
    props.insert("name".to_string(), name.to_string());
    props.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());
    props.insert("api".to_string(), api_port.to_string());
    props.insert("tier".to_string(), tier.to_string());
    let host = format!("{}.local.", host_label(name));
    let info = ServiceInfo::new(SERVICE_TYPE, name, &host, ip.to_string().as_str(), secure_port, props)?;
    daemon.register(info)?;
    Ok(daemon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_are_safe() {
        assert_eq!(host_label("DESKTOP-DAN"), "desktop-dan");
        assert_eq!(host_label("Dan's PC!"), "dan-s-pc");
        assert_eq!(host_label("***"), "outpost");
    }
}
