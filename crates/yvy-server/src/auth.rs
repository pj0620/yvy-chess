//! Optional password gate. When `YVY_PASSWORD` is set, every request except the health check
//! must carry HTTP Basic credentials with that password (the username is ignored). Browsers
//! prompt once and then attach the credentials to every same-origin request, including the
//! SPA's fetches and the import event stream, so the frontend needs no changes. This is what
//! makes a public deployment safe enough for personal use: nobody else can start imports or
//! games against your history.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;

use crate::state::AppState;

pub async fn require_password(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = state.config.password.as_deref() else {
        return next.run(req).await;
    };
    if req.uri().path() == "/api/health" {
        return next.run(req).await;
    }
    let presented = presented_password(&req);
    if presented
        .as_deref()
        .map(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
        .unwrap_or(false)
    {
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"You v You Chess\", charset=\"UTF-8\"")],
        "password required",
    )
        .into_response()
}

fn presented_password(req: &Request) -> Option<String> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    Some(match credentials.split_once(':') {
        Some((_, password)) => password.to_string(),
        None => credentials,
    })
}

/// Compares without short-circuiting on the first differing byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn compares_bytes() {
        assert!(constant_time_eq(b"hunter2", b"hunter2"));
        assert!(!constant_time_eq(b"hunter2", b"hunter3"));
        assert!(!constant_time_eq(b"hunter2", b"hunter"));
    }
}
