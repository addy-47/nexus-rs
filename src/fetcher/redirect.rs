use std::time::{Duration, Instant};

use reqwest::header::LOCATION;
use reqwest::StatusCode;
use url::Url;

use super::client::read_bounded_body;
use super::connector::build_pinned_client;
use super::dns::resolve_and_validate_host;
use crate::error::NexusError;
use crate::model::PageFetchMetrics;

const MAX_REDIRECT_HOPS: usize = 5;

/// Successful page retrieval outcome containing content and granular network timing metrics.
#[derive(Clone, Debug)]
pub struct FetchedPageOutcome {
    /// Final URL destination after redirect navigation.
    pub final_url: String,
    /// Decoded body content.
    pub body: String,
    /// Granular fetch and DNS telemetry metrics.
    pub metrics: PageFetchMetrics,
}

/// Fetches a URL by executing a manual redirect loop with per-hop DNS pre-flight and socket pinning.
/// Returns `FetchedPageOutcome` containing `final_url`, `body`, and `PageFetchMetrics`.
pub async fn fetch_with_redirect_vetting(
    initial_url: &str,
    timeout: Duration,
    max_response_bytes: usize,
) -> Result<FetchedPageOutcome, NexusError> {
    let overall_start = Instant::now();
    tokio::time::timeout(timeout, async {
        let mut current_url = parse_and_validate_scheme(initial_url)?;
        let mut cached_client: Option<(String, u16, reqwest::Client)> = None;
        let mut total_dns_ms = 0u64;

        for hop in 0..MAX_REDIRECT_HOPS {
            let hop_count = hop + 1;
            let (host, port) = extract_host_and_port(&current_url)?;
            let client = match cached_client {
                Some((ref cached_host, cached_port, ref c))
                    if cached_host == &host && cached_port == port =>
                {
                    c.clone()
                }
                _ => {
                    let dns_start = Instant::now();
                    let resolved_addrs = resolve_and_validate_host(&host, port).await?;
                    total_dns_ms += dns_start.elapsed().as_millis() as u64;
                    let c = build_pinned_client(&host, &resolved_addrs, timeout)?;
                    cached_client = Some((host.clone(), port, c.clone()));
                    c
                }
            };

            let response = client.get(current_url.as_str()).send().await?;
            let status = response.status();

            if is_redirect_status(status) {
                current_url = resolve_redirect_location(&response, &current_url, hop, initial_url)?;
                continue;
            }

            if !status.is_success() {
                log::warn!(
                    "[Nexus::Egress] HTTP request returned status {status} for {}",
                    current_url.as_str()
                );
                return Err(NexusError::HttpStatus {
                    status: status.as_u16(),
                    url: current_url.to_string(),
                });
            }

            let body =
                read_bounded_body(response, current_url.as_str(), max_response_bytes).await?;
            let bytes_read = body.len();
            let total_fetch_ms = overall_start.elapsed().as_millis() as u64;

            let metrics = PageFetchMetrics {
                requested_url: initial_url.to_string(),
                final_url: Some(current_url.to_string()),
                dns_resolution_ms: total_dns_ms,
                total_fetch_ms,
                bytes_read,
                hop_count,
                success: true,
                error: None,
            };

            return Ok(FetchedPageOutcome {
                final_url: current_url.to_string(),
                body,
                metrics,
            });
        }

        Err(NexusError::TooManyRedirects {
            max_hops: MAX_REDIRECT_HOPS,
            url: initial_url.to_owned(),
        })
    })
    .await
    .map_err(|_| NexusError::Timeout {
        url: initial_url.to_owned(),
        timeout_ms: timeout.as_millis() as u64,
    })?
}

/// Parses a URL string and asserts that the scheme is strictly HTTP or HTTPS.
fn parse_and_validate_scheme(url_str: &str) -> Result<Url, NexusError> {
    let parsed = Url::parse(url_str).map_err(|e| NexusError::InvalidUrl(e.to_string()))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(NexusError::InvalidUrl(format!(
            "Unsupported scheme '{scheme}', only http and https are allowed"
        )));
    }
    Ok(parsed)
}

/// Extracts domain host and port from a parsed URL.
fn extract_host_and_port(url: &Url) -> Result<(String, u16), NexusError> {
    let host = url
        .host_str()
        .ok_or_else(|| NexusError::InvalidUrl("Missing host in URL".to_string()))?
        .to_owned();

    let port = url.port_or_known_default().unwrap_or(match url.scheme() {
        "https" => 443,
        _ => 80,
    });

    Ok((host, port))
}

/// Determines whether an HTTP status code indicates a redirection.
fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

/// Resolves the Location header against the current URL and checks the redirect hop budget.
fn resolve_redirect_location(
    response: &reqwest::Response,
    current_url: &Url,
    current_hop: usize,
    initial_url: &str,
) -> Result<Url, NexusError> {
    if current_hop + 1 >= MAX_REDIRECT_HOPS {
        return Err(NexusError::TooManyRedirects {
            max_hops: MAX_REDIRECT_HOPS,
            url: initial_url.to_owned(),
        });
    }

    let location_header = response
        .headers()
        .get(LOCATION)
        .and_then(|val| val.to_str().ok())
        .ok_or_else(|| {
            NexusError::InvalidUrl("Redirect response missing Location header".to_string())
        })?;

    let next_url = current_url
        .join(location_header)
        .map_err(|e| NexusError::InvalidUrl(format!("Invalid redirect URL: {e}")))?;

    let scheme = next_url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(NexusError::InvalidUrl(format!(
            "Redirect target attempted non-http scheme: '{scheme}'"
        )));
    }

    Ok(next_url)
}
