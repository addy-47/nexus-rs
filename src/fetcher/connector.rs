use std::net::SocketAddr;
use std::time::Duration;

use reqwest::redirect::Policy;

use crate::error::NexusError;

const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

/// Builds a pinned reqwest client that resolves the specified host strictly to the provided socket addresses.
pub fn build_pinned_client(
    host: &str,
    resolved_addrs: &[SocketAddr],
    timeout: Duration,
) -> Result<reqwest::Client, NexusError> {
    let client = reqwest::Client::builder()
        .resolve_to_addrs(host, resolved_addrs)
        .redirect(Policy::none())
        .no_proxy()
        .tcp_nodelay(true)
        .timeout(timeout)
        .user_agent(DEFAULT_USER_AGENT)
        .build()?;

    Ok(client)
}
