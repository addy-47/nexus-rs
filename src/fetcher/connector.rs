use std::net::SocketAddr;
use std::time::Duration;

use crate::error::NexusError;

/// Builds a pinned primp client that resolves the specified host strictly to the provided socket addresses
/// and uses browser TLS fingerprint impersonation to bypass Cloudflare/bot mitigations.
pub fn build_pinned_client(
    host: &str,
    resolved_addrs: &[SocketAddr],
    timeout: Duration,
) -> Result<primp::Client, NexusError> {
    let profiles = [
        (
            primp::Impersonate::ChromeV146,
            primp::ImpersonateOS::Windows,
        ),
        (primp::Impersonate::ChromeV146, primp::ImpersonateOS::MacOS),
        (
            primp::Impersonate::FirefoxV146,
            primp::ImpersonateOS::Windows,
        ),
    ];
    let pick = rand::random_range(0..profiles.len());
    let (browser, os) = profiles[pick];

    let client = primp::Client::builder()
        .resolve_to_addrs(host, resolved_addrs)
        .impersonate(browser)
        .impersonate_os(os)
        .redirect(primp::redirect::Policy::none())
        .no_proxy()
        .tcp_nodelay(true)
        .timeout(timeout)
        .build()?;

    Ok(client)
}
