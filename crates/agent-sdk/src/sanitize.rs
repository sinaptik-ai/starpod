//! Tool result sanitization.
//!
//! Provides [`sanitize_tool_result`] — a post-processing pass applied to every
//! tool result before it enters the conversation. It strips base64 data URIs,
//! hex blobs, and enforces a hard byte-length limit.

use regex::Regex;
use std::sync::LazyLock;

/// Default maximum tool result size in bytes.
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 50_000;

/// Minimum blob length (in characters) before stripping kicks in.
const BLOB_THRESHOLD: usize = 200;

static BASE64_DATA_URI_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches data URIs with base64 payloads of BLOB_THRESHOLD+ chars.
    Regex::new(&format!(
        r"data:[^;]+;base64,[A-Za-z0-9+/=]{{{},}}",
        BLOB_THRESHOLD
    ))
    .unwrap()
});

static HEX_BLOB_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Matches hex sequences (optionally prefixed with 0x) of BLOB_THRESHOLD+ chars.
    Regex::new(&format!(r"(?:0x)?[0-9a-fA-F]{{{},}}", BLOB_THRESHOLD)).unwrap()
});

/// Sanitize a tool result string.
///
/// 1. Replace base64 data URIs (≥200 char payload) with a placeholder.
/// 2. Replace hex blobs (≥200 chars) with a placeholder.
/// 3. Hard-truncate at `max_bytes` on a UTF-8 char boundary.
pub fn sanitize_tool_result(content: &str, max_bytes: usize) -> String {
    // Phase 1: strip base64 data URIs.
    let stripped = BASE64_DATA_URI_RE.replace_all(content, |caps: &regex::Captures| {
        let len = caps[0].len();
        format!("[data URI removed, {} bytes]", len)
    });

    // Phase 2: strip hex blobs.
    let stripped = HEX_BLOB_RE.replace_all(&stripped, |caps: &regex::Captures| {
        let len = caps[0].len();
        format!("[hex blob removed, {} chars]", len)
    });

    // Phase 3: hard truncate at byte limit.
    if stripped.len() <= max_bytes {
        return stripped.into_owned();
    }

    let total_len = stripped.len();
    // Find the last char boundary at or before max_bytes.
    let mut boundary = max_bytes;
    while boundary > 0 && !stripped.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut truncated = stripped[..boundary].to_string();
    truncated.push_str(&format!(
        "\n[Output truncated at {} bytes, {} total]",
        boundary, total_len
    ));
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_normal_content() {
        let input = "Hello, this is a normal tool result.";
        assert_eq!(
            sanitize_tool_result(input, DEFAULT_MAX_TOOL_RESULT_BYTES),
            input
        );
    }

    #[test]
    fn strip_base64_data_uri() {
        let payload = "A".repeat(300);
        let input = format!("Before data:image/png;base64,{} After", payload);
        let result = sanitize_tool_result(&input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert!(result.contains("[data URI removed,"));
        assert!(result.contains("After"));
        assert!(!result.contains(&payload));
    }

    #[test]
    fn preserve_short_base64_data_uri() {
        let input = "data:image/png;base64,iVBOR short one";
        let result = sanitize_tool_result(input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert_eq!(result, input);
    }

    #[test]
    fn strip_hex_blob() {
        let hex = "a".repeat(300);
        let input = format!("Result: 0x{} end", hex);
        let result = sanitize_tool_result(&input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert!(result.contains("[hex blob removed,"));
        assert!(result.contains("end"));
        assert!(!result.contains(&hex));
    }

    #[test]
    fn preserve_short_hex() {
        let input = "Hash: 0xdeadbeef";
        let result = sanitize_tool_result(input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert_eq!(result, input);
    }

    #[test]
    fn hard_truncate_at_byte_limit() {
        let input = "x".repeat(100);
        let result = sanitize_tool_result(&input, 50);
        assert!(result.starts_with(&"x".repeat(50)));
        assert!(result.contains("[Output truncated at 50 bytes"));
    }

    #[test]
    fn truncate_respects_char_boundary() {
        // 10 emoji (4 bytes each = 40 bytes total)
        let input = "🎉".repeat(10);
        let result = sanitize_tool_result(&input, 17);
        // Should truncate to 4 full emoji (16 bytes), not split mid-char
        assert!(result.starts_with("🎉🎉🎉🎉"));
        assert!(result.contains("[Output truncated"));
    }

    #[test]
    fn strip_multiple_blobs() {
        let b64 = "B".repeat(250);
        let hex = "f".repeat(250);
        let input = format!("data:text/plain;base64,{} middle {} end", b64, hex);
        let result = sanitize_tool_result(&input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert!(result.contains("[data URI removed,"));
        assert!(result.contains("[hex blob removed,"));
        assert!(result.contains("middle"));
        assert!(result.contains("end"));
    }

    #[test]
    fn strip_base64_only_content() {
        let payload = "A".repeat(500);
        let input = format!("data:image/png;base64,{}", payload);
        let result = sanitize_tool_result(&input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert!(result.contains("[data URI removed,"));
        assert!(!result.contains(&payload));
    }

    #[test]
    fn strip_adjacent_base64_and_hex() {
        let b64 = "C".repeat(300);
        let hex = "a".repeat(300);
        // Separated by a space so the regex doesn't merge them
        let input = format!("data:image/png;base64,{} 0x{}", b64, hex);
        let result = sanitize_tool_result(&input, DEFAULT_MAX_TOOL_RESULT_BYTES);
        assert!(result.contains("[data URI removed,"));
        assert!(result.contains("[hex blob removed,"));
        assert!(!result.contains(&b64));
        assert!(!result.contains(&hex));
    }

    #[test]
    fn passthrough_empty_content() {
        assert_eq!(sanitize_tool_result("", DEFAULT_MAX_TOOL_RESULT_BYTES), "");
    }
}
