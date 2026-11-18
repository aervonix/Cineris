//! A file's bytes as the text an extractor reads.

use std::borrow::Cow;

/// How much of the start of a file is looked at for a NUL byte, as `read`
/// does (`docs/architecture/05-tools.md`, "`read`").
pub const BINARY_PROBE: usize = 8 * 1024;

/// Whether bytes are a binary file's: a NUL byte in the first
/// [`BINARY_PROBE`] bytes. Text in UTF-16 has NUL bytes too, and is taken
/// for binary: no extractor reads it.
#[must_use]
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE)].contains(&0)
}

/// The text of a file's bytes: without a UTF-8 byte-order mark, and with
/// every byte that is not UTF-8 replaced. Line endings are left as they
/// are: the extractors read lines with `str::lines`, which takes `\r\n` as
/// one ending.
#[must_use]
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nul_byte_near_the_start_makes_a_file_binary() {
        assert!(is_binary(b"\x7fELF\x02\x01\x00"));
        assert!(!is_binary(b"fn main() {}\n"));
        let mut late = vec![b'a'; BINARY_PROBE];
        late.push(0);
        assert!(!is_binary(&late));
    }

    #[test]
    fn decoding_drops_a_byte_order_mark_and_replaces_bad_bytes() {
        assert_eq!(decode(b"\xEF\xBB\xBFfn a() {}"), "fn a() {}");
        assert_eq!(decode(b"fn \xFFa() {}"), "fn \u{FFFD}a() {}");
    }
}
