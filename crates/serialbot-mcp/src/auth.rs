// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Bearer-token protection of the MCP endpoint.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Returns a new random token: 32 bytes from the OS RNG as 64 hex digits.
pub fn generate_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(crate::payload::to_hex(&bytes).replace(' ', ""))
}

/// Compares two byte strings in time that depends only on their lengths.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether an `Authorization` header value carries `token` as a bearer
/// token. The scheme is matched case-insensitively (RFC 9110).
fn authorized(header_value: Option<&[u8]>, token: &str) -> bool {
    let Some(value) = header_value else {
        return false;
    };
    let Some((scheme, credentials)) = value.split_at_checked(7) else {
        return false;
    };
    scheme.eq_ignore_ascii_case(b"bearer ") && constant_time_eq(credentials, token.as_bytes())
}

/// Axum middleware that answers `401 Unauthorized` unless the request
/// carries `Authorization: Bearer <token>`.
pub(crate) async fn require_token(
    State(token): State<Arc<str>>,
    request: Request,
    next: Next,
) -> Response {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)
        .map(axum::http::HeaderValue::as_bytes);
    if authorized(value, &token) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "missing or invalid bearer token",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn constant_time_eq_matches_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn authorization_header_parsing() {
        assert!(authorized(Some(b"Bearer t0k"), "t0k"));
        assert!(authorized(Some(b"bearer t0k"), "t0k"));
        assert!(!authorized(Some(b"Bearer t0kx"), "t0k"));
        assert!(!authorized(Some(b"Basic t0k"), "t0k"));
        assert!(!authorized(Some(b"Bearer"), "t0k"));
        assert!(!authorized(Some(b"t0k"), "t0k"));
        assert!(!authorized(None, "t0k"));
    }
}
