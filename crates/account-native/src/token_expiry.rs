//! The access-token expiry scheduling hint.
//!
//! [`access_token_expiry_unix`] reads only the `exp` claim of a JWT payload. It is a scheduling
//! hint for the renewal lifecycle, never an authentication decision: the signature is not
//! verified and the token is not trusted. Malformed or oversized input returns `None`.

use serde_json::Value;

/// Hard cap on the whole access token; a WorkOS JWT is a few kilobytes at most.
const MAX_TOKEN_BYTES: usize = 16 * 1024;
/// Hard cap on the decoded payload; the `exp` claim needs only a few bytes.
const MAX_PAYLOAD_BYTES: usize = 8 * 1024;

/// The `exp` claim of the token's JWT payload as Unix seconds, or `None` when the token is not a
/// well-formed JWT with a non-negative integer `exp`. This never verifies a signature.
pub(crate) fn access_token_expiry_unix(token: &str) -> Option<u64> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        return None;
    }
    let mut segments = token.split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    let _signature = segments.next()?;
    if header.is_empty() || payload.is_empty() || segments.next().is_some() {
        return None;
    }
    let decoded = decode_base64url(payload)?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value.get("exp").and_then(Value::as_u64)
}

/// A bounded base64url decoder (RFC 4648 §5, no padding). Invalid characters, non-canonical
/// trailing bits and oversized output are refused instead of panicking.
fn decode_base64url(segment: &str) -> Option<Vec<u8>> {
    if segment.is_empty() || segment.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in segment.bytes() {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            if out.len() >= MAX_PAYLOAD_BYTES {
                return None;
            }
            out.push(((buffer >> bits) & 0xff) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    // Canonical base64url leaves no non-zero trailing bits.
    if buffer != 0 {
        return None;
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::access_token_expiry_unix;

    /// base64url of `{"exp":1700000000}`.
    const EXP_PAYLOAD: &str = "eyJleHAiOjE3MDAwMDAwMDB9";
    /// base64url of `{"alg":"RS256","typ":"JWT"}`.
    const HEADER: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9";

    fn jwt(payload: &str) -> String {
        format!("{HEADER}.{payload}.c2lnbmF0dXJl")
    }

    #[test]
    fn access_token_expiry_reads_only_a_valid_exp_claim() {
        assert_eq!(access_token_expiry_unix(&jwt(EXP_PAYLOAD)), Some(1_700_000_000));
        // Other claims are ignored; only the payload `exp` is read.
        let with_claims = "eyJzdWIiOiJ1c2VyXzAxIiwiZXhwIjoxNzAwMDAwMDAwLCJpc3MiOiJodHRwczovL2FwaS53b3Jrb3MuY29tIn0";
        assert_eq!(access_token_expiry_unix(&jwt(with_claims)), Some(1_700_000_000));
        // The signature is never verified, so an empty signature segment still yields the hint.
        assert_eq!(access_token_expiry_unix(&format!("{HEADER}.{EXP_PAYLOAD}.")), Some(1_700_000_000));
        // An `exp` claim in the header is not the payload claim.
        assert_eq!(access_token_expiry_unix(&format!("{EXP_PAYLOAD}.e30.c2lnbmF0dXJl")), None);
        // Missing, negative, float and string `exp` values are not a usable integer expiry.
        for payload in ["e30", "eyJzdWIiOiJ1c2VyXzAxIn0", "eyJleHAiOi0xfQ", "eyJleHAiOjEuNX0", "eyJleHAiOiIxNzAwMDAwMDAwIn0"] {
            assert_eq!(access_token_expiry_unix(&jwt(payload)), None, "{payload}");
        }
        // A malformed or non-JSON payload has no readable claim.
        for payload in ["eyJleHAiOg", "bm90IGpzb24"] {
            assert_eq!(access_token_expiry_unix(&jwt(payload)), None, "{payload}");
        }
    }

    #[test]
    fn access_token_expiry_rejects_malformed_segments() {
        let malformed = [
            String::new(),
            EXP_PAYLOAD.to_owned(),
            format!("{HEADER}.{EXP_PAYLOAD}"),
            format!("{HEADER}.{EXP_PAYLOAD}.c2lnbmF0dXJl.extra"),
            format!(".{EXP_PAYLOAD}.c2lnbmF0dXJl"),
            format!("{HEADER}..c2lnbmF0dXJl"),
            jwt("eyJleHAiOjE3MDAwMDAwMDB!"),
            jwt("eyJleHAiOjE3MDAwMDAwMDB9="),
            jwt("e31"),
            jwt(&"A".repeat(11_000)),
            format!("{HEADER}.{}.c2lnbmF0dXJl", "A".repeat(16 * 1024)),
        ];
        for token in malformed {
            assert_eq!(access_token_expiry_unix(&token), None, "{token}");
        }
    }
}
