//! ============================================================================
//! ssrf_egress_test.rs — High-impact test suite for polyc-egress SSRF security
//! ============================================================================
//! Category     : Integration Test
//! Component    : nexus::fetcher::dns
//! Prerequisites: None (hermetic network test)
//! Execution    : cargo test --test ssrf_egress_test
//! Metrics      : SSRF blocked rate, zero false-negatives on private subnets
//! ============================================================================

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use nexus::EgressFetcher;
use nexus::fetcher::dns::is_safe_public_ip;

#[tokio::test]
async fn test_ssrf_rejects_ipv4_private_and_metadata_ranges() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let blocked_ips: Vec<Ipv4Addr> = vec![
            // Loopback (127.0.0.0/8)
            Ipv4Addr::new(127, 0, 0, 1),
            Ipv4Addr::new(127, 255, 255, 254),
            // RFC 1918 (10.0.0.0/8)
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 255, 255, 254),
            // RFC 1918 (172.16.0.0/12)
            Ipv4Addr::new(172, 16, 0, 1),
            Ipv4Addr::new(172, 31, 255, 254),
            // RFC 1918 (192.168.0.0/16)
            Ipv4Addr::new(192, 168, 0, 1),
            Ipv4Addr::new(192, 168, 1, 254),
            // Link-local & AWS/GCP cloud metadata (169.254.0.0/16)
            Ipv4Addr::new(169, 254, 169, 254),
            Ipv4Addr::new(169, 254, 1, 1),
            // Carrier-grade NAT (100.64.0.0/10)
            Ipv4Addr::new(100, 64, 0, 1),
            Ipv4Addr::new(100, 127, 255, 254),
            // Current network / Unspecified (0.0.0.0/8)
            Ipv4Addr::new(0, 0, 0, 0),
            Ipv4Addr::new(0, 1, 2, 3),
            // Broadcast & Reserved (240.0.0.0/4, 255.255.255.255)
            Ipv4Addr::new(240, 0, 0, 1),
            Ipv4Addr::new(255, 255, 255, 255),
            // Documentation (RFC 5737)
            Ipv4Addr::new(192, 0, 2, 1),
            Ipv4Addr::new(198, 51, 100, 1),
            Ipv4Addr::new(203, 0, 113, 1),
        ];

        for ip in blocked_ips {
            assert!(
                !is_safe_public_ip(&IpAddr::V4(ip)),
                "SSRF guard failed to block unsafe IPv4: {ip}"
            );
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_ssrf_rejects_ipv6_private_and_mapped_ranges() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let blocked_ipv6: Vec<Ipv6Addr> = vec![
            // Loopback (::1)
            Ipv6Addr::LOCALHOST,
            // Unspecified (::)
            Ipv6Addr::UNSPECIFIED,
            // Unique Local Addresses (fc00::/7)
            "fc00::1".parse().unwrap(),
            "fd12:3456:789a:1::1".parse().unwrap(),
            // Link-local unicast (fe80::/10)
            "fe80::1".parse().unwrap(),
            // Discard prefix (100::/64)
            "100::1".parse().unwrap(),
            // Documentation (2001:db8::/32)
            "2001:db8::1".parse().unwrap(),
            // IPv4-mapped loopback (::ffff:127.0.0.1)
            "0:0:0:0:0:ffff:7f00:0001".parse().unwrap(),
            // IPv4-mapped cloud metadata (::ffff:169.254.169.254)
            "0:0:0:0:0:ffff:a9fe:a9fe".parse().unwrap(),
            // IPv4-mapped private 10.0.0.1 (::ffff:10.0.0.1)
            "0:0:0:0:0:ffff:0a00:0001".parse().unwrap(),
            // NAT64 Well-Known (64:ff9b::/96)
            "64:ff9b::192.0.2.1".parse().unwrap(),
            // NAT64 Local-Use (64:ff9b:1::/48)
            "64:ff9b:1::1".parse().unwrap(),
            // Teredo tunneling (2001::/32)
            "2001:0000:4136:e378:8000:63bf:3fff:fdd2".parse().unwrap(),
            // Benchmarking (2001:2::/48)
            "2001:2::1".parse().unwrap(),
            // ORCHIDv2 (2001:20::/28)
            "2001:20::1".parse().unwrap(),
            // 6to4 tunneling (2002::/16)
            "2002:c000:0201::1".parse().unwrap(),
            // Deprecated Site-local (fec0::/10)
            "fec0::1".parse().unwrap(),
        ];

        for ip in blocked_ipv6 {
            assert!(
                !is_safe_public_ip(&IpAddr::V6(ip)),
                "SSRF guard failed to block unsafe IPv6: {ip}"
            );
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_ssrf_permits_valid_public_ips() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let safe_ips: Vec<IpAddr> = vec![
            "1.1.1.1".parse().unwrap(),
            "8.8.8.8".parse().unwrap(),
            "93.184.216.34".parse().unwrap(),
            "140.82.112.4".parse().unwrap(),
            "2606:4700:4700::1111".parse().unwrap(),
            "2001:4860:4860::8888".parse().unwrap(),
        ];

        for ip in safe_ips {
            assert!(
                is_safe_public_ip(&ip),
                "SSRF guard falsely rejected safe public IP: {ip}"
            );
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn test_egress_rejects_unsupported_schemes() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let fetcher = EgressFetcher::default();

        let dangerous_urls = [
            "file:///etc/passwd",
            "ftp://example.com/file",
            "gopher://127.0.0.1:70",
            "data:text/html,<html></html>",
        ];

        for url in dangerous_urls {
            let result = fetcher.fetch_page(url).await;
            assert!(
                result.is_err(),
                "Fetcher must reject unsupported scheme for: {url}"
            );
        }
    })
    .await
    .expect("test timed out");
}
