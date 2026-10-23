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
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(ApiError::unauthorized)?;
        let Some((session, account)) = sessions::authenticate(&state.pool, token).await? else {
            return Err(ApiError::unauthorized());
        };
        // Desktop sign-ins never leave this PC. If one turns up from the network it was
        // copied, so refuse it. Unknown origin counts as the network.
        if session.kind == SessionKind::Desktop {
            // The ConnectInfo extractor also understands MockConnectInfo (used in tests).
            let local = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
                .await
                .is_ok_and(|ConnectInfo(addr)| addr.ip().is_loopback());
            if !local {
                return Err(ApiError::unauthorized());
            }
        }
        Ok(Auth { session, account })
    }
}
