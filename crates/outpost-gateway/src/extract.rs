//! Request extractors: who is calling.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use outpost_core::accounts::{Account, Role};
use outpost_core::sessions::{self, Session};

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
        match sessions::authenticate(&state.pool, token).await? {
            Some((session, account)) => Ok(Auth { session, account }),
            None => Err(ApiError::unauthorized()),
        }
    }
}
