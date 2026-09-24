use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::error::NexusError;

/// Evaluates an IPv4 address against private, loopback, link-local, and reserved ranges.
fn is_safe_ipv4(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();

    if ip.is_loopback() || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return false;
    }

    if octets[0] == 0 {
        return false;
    }

    // RFC 1918 Private networks (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
    if octets[0] == 10 {
        return false;
    }
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return false;
    }
    if octets[0] == 192 && octets[1] == 168 {
        return false;
    }

    // RFC 3927 Link-local (includes 169.254.169.254 cloud metadata)
    if octets[0] == 169 && octets[1] == 254 {
        return false;
    }

    // RFC 6598 Carrier-grade NAT (100.64.0.0/10)
    if octets[0] == 100 && (octets[1] & 0xC0) == 64 {
        return false;
    }

    // RFC 5737 Documentation (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)
    if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
    {
        return false;
    }

    // RFC 2544 Benchmarking (198.18.0.0/15)
    if octets[0] == 198 && (octets[1] & 0xFE) == 18 {
        return false;
    }

    // Reserved for future use (240.0.0.0/4)
    if octets[0] >= 240 {
        return false;
    }

    true
}

/// Evaluates an IPv6 address against private, loopback, link-local, and reserved ranges.
fn is_safe_ipv6(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();

    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }

    // IPv4-mapped IPv6 (::ffff:0:0/96)
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_safe_ipv4(&v4);
    }

    // IPv4-compatible IPv6 (deprecated ::0.0.0.0/96)
    if segments[0] == 0
        && segments[1] == 0
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0
    {
        let v4 = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        );
        return is_safe_ipv4(&v4);
    }

    // Unique Local Addresses (fc00::/7)
    if (segments[0] & 0xFE00) == 0xFC00 {
        return false;
    }

    // Link-local unicast (fe80::/10)
    if (segments[0] & 0xFFC0) == 0xFE80 {
        return false;
    }

    // Discard-only prefix (100::/64)
    if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return false;
    }

    // Documentation prefix (2001:db8::/32)
    if segments[0] == 0x2001 && segments[1] == 0x0DB8 {
        return false;
    }

    true
}

/// Evaluates an IP address against RFC1918, link-local, loopback, and reserved ranges.
pub fn is_safe_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_safe_ipv4(v4),
        IpAddr::V6(v6) => is_safe_ipv6(v6),
    }
}

/// Resolves host domain to socket addresses and verifies all resolved IPs against the SSRF policy.
pub async fn resolve_and_validate_host(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, NexusError> {
    let address_target = format!("{host}:{port}");
    let resolved: Vec<SocketAddr> = tokio::net::lookup_host(&address_target)
        .await
        .map_err(|err| NexusError::DnsResolution {
            host: host.to_owned(),
            message: err.to_string(),
        })?
        .collect();

    if resolved.is_empty() {
        return Err(NexusError::DnsResolution {
            host: host.to_owned(),
            message: "No addresses returned by resolver".to_string(),
        });
    }

    for socket_addr in &resolved {
        let ip = socket_addr.ip();
        if !is_safe_public_ip(&ip) {
            log::warn!("[Nexus::Egress] Blocked SSRF attempt to non-public IP: {ip} for host: {host}");
            return Err(NexusError::PrivateIpBlocked(ip));
        }
    }

    Ok(resolved)
}
