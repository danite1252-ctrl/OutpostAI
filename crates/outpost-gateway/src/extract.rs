//! Request extractors: who is calling.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use outpost_core::accounts::{Account, Role};
use outpost_core::sessions::{self, Session, SessionKind};

use crate::error::ApiError;
use crate::AppState;

/// How a request arrived: over the encrypted phone port, and from where.
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    pub secure: bool,
    pub peer: Option<SocketAddr>,
}

impl Transport {
    pub fn loopback(&self) -> bool {
        self.peer.is_some_and(|a| a.ip().is_loopback())
    }

    /// Phones must use the encrypted port; this PC may use either.
    pub fn require_secure(&self) -> Result<(), ApiError> {
        if self.secure || self.loopback() {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "Phones connect over Outpost's encrypted port (7421). Update the app, or pair again.",
            ))
        }
    }
}

impl FromRequestParts<AppState> for Transport {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let peer = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|ConnectInfo(addr)| addr);
        Ok(Transport {
            secure: parts.extensions.get::<crate::tls::Secure>().is_some(),
            peer,
        })
    }
}

/// The signed-in caller. Add this to a handler's arguments to require a bearer token.
pub struct Auth {
    pub session: Session,
    pub account: Account,
}

impl Auth {
    /// Only from the desktop app on this PC (e.g. making a pairing code).
    pub fn require_desktop(&self, what: &str) -> Result<(), ApiError> {
        if self.session.kind == SessionKind::Desktop {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!("{what} only works in the desktop app on the Outpost PC.")))
        }
    }

    /// High-risk actions (accounts, app keys, deleting models, installing engines and
    /// workflows) are for a person: the desktop app, or a phone that sends the account's
    /// PIN again as the `X-Outpost-Pin` header. App keys can't do them, since they may
    /// travel over plain HTTP.
    pub async fn require_fresh_pin(&self, state: &AppState, headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
        match self.session.kind {
            SessionKind::Desktop => return Ok(()),
            SessionKind::App => {
                return Err(ApiError::forbidden(
                    "App keys can't do this. Use the desktop app on the Outpost PC, or a paired phone.",
                ))
            }
            SessionKind::Device => {}
        }
        let pin = headers
            .get("x-outpost-pin")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    axum::http::StatusCode::FORBIDDEN,
                    "pin_required",
                    "Enter your PIN again to do this from a phone.",
                )
            })?;
        outpost_core::auth::check_secret(&state.pool, &self.account.id, pin, "device_confirm").await?;
        Ok(())
    }

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
                "Outpost doesn't recognise this key. It may be mistyped or revoked. Make a new one in Connections > App keys.",
            ));
        };
        // Desktop sign-ins never leave this PC. If one turns up from the network it was
        // copied, so refuse it. Unknown origin counts as the network.
        if session.kind == SessionKind::Desktop && !peer.is_some_and(|a| a.ip().is_loopback()) {
            tracing::warn!(%from, "desktop sign-in used from another device");
            return Err(ApiError::unauthorized_because(
                "That is a desktop sign-in, which only works on the Outpost PC. Use an app key from Connections > App keys.",
            ));
        }
        // A paired phone only over the encrypted port (or from this PC itself), so a
        // copied phone token can't be used over plain HTTP.
        let secure = parts.extensions.get::<crate::tls::Secure>().is_some();
        if session.kind == SessionKind::Device && !secure && !peer.is_some_and(|a| a.ip().is_loopback()) {
            tracing::warn!(%from, "phone sign-in used without the encrypted port");
            return Err(ApiError::unauthorized_because(
                "Phones connect over Outpost's encrypted port (7421). Update the app, or pair again.",
            ));
        }
        Ok(Auth { session, account })
    }
}
