use encoding_rs::Encoding;
use futures_util::StreamExt;
use reqwest::Response;
use reqwest::header::CONTENT_TYPE;

use crate::error::NexusError;

/// Reads response body stream up to max_response_bytes, aborting if exceeded.
pub async fn read_bounded_body(
    response: Response,
    url: &str,
    max_response_bytes: usize,
) -> Result<String, NexusError> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|val| val.to_str().ok())
        .map(str::to_owned);

    let content_len = response.content_length();
    if let Some(content_length) = content_len
        && content_length as usize > max_response_bytes
    {
        log::warn!(
            "[Nexus::Egress] Content-Length {content_length} exceeds limit of {max_response_bytes} for {url}"
        );
        return Err(NexusError::ResponseTooLarge {
            limit_bytes: max_response_bytes,
            url: url.to_owned(),
        });
    }

    let initial_cap = content_len
        .map(|cl| (cl as usize).min(max_response_bytes))
        .unwrap_or(16 * 1024);
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::with_capacity(initial_cap);

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(NexusError::Http)?;
        if buffer.len() + chunk.len() > max_response_bytes {
            log::warn!(
                "[Nexus::Egress] Response exceeded limit of {max_response_bytes} bytes for {url}"
            );
            return Err(NexusError::ResponseTooLarge {
                limit_bytes: max_response_bytes,
                url: url.to_owned(),
            });
        }
        buffer.extend_from_slice(&chunk);
    }

    Ok(decode_body_bytes(&buffer, content_type.as_deref()))
}

/// Decodes raw body bytes into String using Content-Type charset or UTF-8 fallback.
fn decode_body_bytes(bytes: &[u8], content_type: Option<&str>) -> String {
    let encoding = content_type
        .and_then(extract_charset_label)
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);

    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

/// Extracts charset label from Content-Type header string.
fn extract_charset_label(content_type: &str) -> Option<&str> {
    for param in content_type.split(';') {
        let trimmed = param.trim();
        if let Some(charset) = trimmed.strip_prefix("charset=") {
            let cleaned = charset.trim_matches('"').trim_matches('\'').trim();
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}
