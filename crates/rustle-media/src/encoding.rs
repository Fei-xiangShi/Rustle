//! Encoding detection and conversion for legacy music files
//!
//! Many old MP3 files (especially Chinese/Japanese songs from 2000s)
//! have metadata encoded in GBK, Shift-JIS, or other legacy encodings.
//! This module provides fallback decoding when UTF-8 fails.

use encoding_rs::{BIG5, EUC_JP, EUC_KR, GBK, SHIFT_JIS, WINDOWS_1252};

/// Try to decode bytes as UTF-8, falling back to common legacy encodings
///
/// Encoding detection priority:
/// 1. UTF-8 (standard)
/// 2. GBK (Simplified Chinese)
/// 3. Big5 (Traditional Chinese)
/// 4. Shift-JIS (Japanese)
/// 5. EUC-JP (Japanese)
/// 6. EUC-KR (Korean)
/// 7. Windows-1252 (Western European)
/// 8. ISO-8859-1 (Latin-1)
pub fn decode_string(bytes: &[u8]) -> String {
    // First try UTF-8
    if let Ok(s) = std::str::from_utf8(bytes) {
        // Check if it looks valid (no replacement characters in the middle)
        if !s.contains('\u{FFFD}') {
            return s.to_string();
        }
    }

    // Try common encodings in order of likelihood for music files
    let encodings = [
        GBK,          // Chinese (most common for old Chinese songs)
        BIG5,         // Traditional Chinese
        SHIFT_JIS,    // Japanese
        EUC_JP,       // Japanese (alternative)
        EUC_KR,       // Korean
        WINDOWS_1252, // Western European (common for Western music)
    ];

    for encoding in encodings {
        let (decoded, _, had_errors) = encoding.decode(bytes);
        if !had_errors {
            // Additional validation: check for common garbage patterns
            let s = decoded.to_string();
            if is_likely_valid_text(&s) {
                return s;
            }
        }
    }

    // Last resort: lossy UTF-8 conversion
    String::from_utf8_lossy(bytes).to_string()
}

/// Heuristic check if decoded text looks valid
fn is_likely_valid_text(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    // Count suspicious characters
    let suspicious_count = s
        .chars()
        .filter(|c| {
            // Control characters (except common whitespace)
            (*c < ' ' && *c != '\t' && *c != '\n' && *c != '\r') ||
        // Private use area
        (*c >= '\u{E000}' && *c <= '\u{F8FF}') ||
        // Replacement character
        *c == '\u{FFFD}'
        })
        .count();

    // Allow up to 5% suspicious characters
    let threshold = (s.len() / 20).max(1);
    suspicious_count <= threshold
}

/// Normalize whitespace and trim a string
pub fn normalize_string(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_passthrough() {
        let input = "Hello World 你好世界";
        assert_eq!(decode_string(input.as_bytes()), input);
    }

    #[test]
    fn test_gbk_decode() {
        // "周杰伦" in GBK encoding
        let gbk_bytes: &[u8] = &[0xD6, 0xDC, 0xBD, 0xDC, 0xC2, 0xD7];
        let decoded = decode_string(gbk_bytes);
        assert_eq!(decoded, "周杰伦");
    }
}
