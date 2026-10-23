//! Request extractors: who is calling.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use outpost_core::accounts::{Account, Role};
use outpost_core::sessions::{self, Session, SessionKind};

use crate::error::ApiError;
use crate::AppState;

/// The signed-in caller. Add this to a handler's arguments to require a bearer token.
pub struct Auth {
    pub session: Session,
    pub account: Account,
}

impl Auth {
    pub fn require_owner(&self) -> Result<(), ApiError> {
        if self.account.role == Role::Owner {
            Ok(())
        } else {
            Err(ApiError::forbidden("Only an owner can do that."))
        }
    }
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        // The ConnectInfo extractor also understands MockConnectInfo (used in tests).
        let peer = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|ConnectInfo(addr)| addr);
        let from = peer.map(|a| a.ip().to_string()).unwrap_or_else(|| "unknown".into());
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::trim);
        // "Bearer" is case-insensitive in HTTP.
        let token = header
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, t)| t.trim())
            .filter(|t| !t.is_empty());
        let Some(token) = token else {
            tracing::warn!(%from, path = %parts.uri.path(), sent_header = header.is_some(), "request without a key");
            return Err(match header {
                None => ApiError::unauthorized_because(
                    "No key was sent. Sign in, or send your app key as \"Authorization: Bearer op_...\".",
                ),
                Some(_) => ApiError::unauthorized_because(
                    "The Authorization header must be \"Bearer op_...\" with your Outpost app key.",
                ),
            });
        };
        let Some((session, account)) = sessions::authenticate(&state.pool, token).await? else {
            let hint: String = token.chars().take(6).collect();
            tracing::warn!(%from, path = %parts.uri.path(), key_starts = %hint, key_len = token.len(), "unknown or revoked key");
            return Err(ApiError::unauthorized_because(
                "Outpost doesn't recognise this key. It may be mistyped or revoked. Make a new one in Settings > App keys.",
            ));
        };
        // Desktop sign-ins never leave this PC. If one turns up from the network it was
        // copied, so refuse it. Unknown origin counts as the network.
        if session.kind == SessionKind::Desktop && !peer.is_some_and(|a| a.ip().is_loopback()) {
            tracing::warn!(%from, "desktop sign-in used from another device");
            return Err(ApiError::unauthorized_because(
                "That is a desktop sign-in, which only works on the Outpost PC. Use an app key from Settings > App keys.",
            ));
        }
        Ok(Auth { session, account })
    }
}
