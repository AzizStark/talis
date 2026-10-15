//! Bounded inline-image subset of RFC 2397. Never interpret inline data as a path.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_URL: usize = 8192;
const MAX_BASE64: usize = MAX_BYTES.div_ceil(3) * 4;
// Only the short, explicitly supported headers below are accepted.
const MAX_DATA_URL: usize = MAX_BASE64 + 32;

pub(super) fn is_data(url: &str) -> bool {
    url.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

pub(super) fn length_allowed(url: &str) -> bool {
    !url.is_empty() && url.len() <= if is_data(url) { MAX_DATA_URL } else { MAX_URL }
}

pub(super) struct Embedded<'a> {
    payload: &'a str,
}
impl<'a> Embedded<'a> {
    pub(super) fn parse(url: &'a str) -> Result<Self> {
        ensure!(
            is_data(url) && length_allowed(url),
            "Invalid embedded artwork URL"
        );
        // Bound header scanning; no arbitrary parameters, SVG, HTML or percent
        // decoding. PNG/JPEG with standard Base64 is the supported contract.
        let comma = url
            .bytes()
            .take(32)
            .position(|b| b == b',')
            .context("Unsupported embedded artwork header")?;
        let header = &url[5..comma];
        ensure!(
            header.eq_ignore_ascii_case("image/png;base64")
                || header.eq_ignore_ascii_case("image/jpeg;base64"),
            "Embedded artwork must be a Base64 PNG or JPEG"
        );
        let payload = &url[comma + 1..];
        ensure!(
            !payload.is_empty() && payload.len() <= MAX_BASE64,
            "Embedded artwork is empty or exceeds 16 MiB"
        );
        Ok(Self { payload })
    }

    pub(super) fn decode(self) -> Result<Vec<u8>> {
        // The engine rejects whitespace, invalid alphabet/padding and trailing
        // bits. Input length bounds the allocation even for malicious metadata.
        let bytes = STANDARD
            .decode(self.payload)
            .context("Invalid embedded artwork Base64")?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_BYTES,
            "Embedded artwork is empty or exceeds 16 MiB"
        );
        // Some player metadata labels JPEG bytes as PNG (or vice versa).
        // Trust only supported image signatures, not the player's MIME hint;
        // the existing bounded decoder still validates the complete image.
        ensure!(
            matches!(
                image::guess_format(&bytes).ok(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
            ),
            "Embedded artwork content must be PNG or JPEG"
        );
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_limits_do_not_relax_remote_urls() {
        assert!(!length_allowed(""));
        assert!(length_allowed(&"x".repeat(MAX_URL)));
        assert!(!length_allowed(&"x".repeat(MAX_URL + 1)));
        assert!(length_allowed(&format!(
            "DATA:{}",
            "x".repeat(MAX_DATA_URL - 5)
        )));
        assert!(!length_allowed(&format!(
            "data:{}",
            "x".repeat(MAX_DATA_URL - 4)
        )));
        assert!(!is_data("dátá:"));
    }

    #[test]
    fn only_supported_headers_and_strict_base64_are_accepted() {
        for url in [
            "data:image/png,abc",
            "data:image/svg+xml;base64,PHN2Zz4=",
            "data:text/html;base64,PHN2Zz4=",
            "data:image/gif;base64,R0lGODlh",
            "data:image/png;charset=utf-8;base64,AAAA",
            "data:image/png;base64,",
        ] {
            assert!(Embedded::parse(url).is_err());
        }
        for payload in [
            "a", "a===", "AA=A", "AAAA!", "AA-_", "AAAA\n", "%2BAAAA", "AB==", "AA==AAAA", "AA==",
        ] {
            let url = format!("data:image/png;base64,{payload}");
            assert!(Embedded::parse(&url).unwrap().decode().is_err());
        }
        for bytes in [
            b"GIF89a".as_slice(),
            b"<svg></svg>".as_slice(),
            b"<html></html>".as_slice(),
        ] {
            let url = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
            assert!(Embedded::parse(&url).unwrap().decode().is_err());
        }
    }

    #[test]
    fn encoded_and_decoded_size_limits_are_independent() {
        let mut bytes = vec![0; MAX_BYTES];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let url = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
        assert_eq!(
            Embedded::parse(&url).unwrap().decode().unwrap().len(),
            MAX_BYTES
        );
        let url = format!("data:image/png;base64,{}", "A".repeat(MAX_BASE64 + 1));
        assert!(Embedded::parse(&url).is_err());
        // 16 MiB + 1 still fits in the maximum encoded length due to padding.
        let url = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(vec![0; MAX_BYTES + 1])
        );
        assert!(
            Embedded::parse(&url)
                .unwrap()
                .decode()
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
    }
}
