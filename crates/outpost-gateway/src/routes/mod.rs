pub mod accounts;
pub mod auth;
pub mod health;
pub mod models;
pub mod setup;

use outpost_core::accounts::Account;
use outpost_core::auth::SignIn;
use outpost_core::sessions::Session;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Returned whenever a new session starts. Keep `token` secret; it is shown only once.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SignInResponse {
    pub token: String,
    pub account: Account,
    pub session: Session,
}

impl From<SignIn> for SignInResponse {
    fn from(s: SignIn) -> Self {
        SignInResponse {
            token: s.token,
            account: s.account,
            session: s.session,
        }
    }
}

/// Requests that change data must come from this PC until pairing (phase 3) exists.
pub(crate) fn require_loopback(addr: &std::net::SocketAddr, what: &str) -> Result<(), crate::error::ApiError> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(crate::error::ApiError::forbidden(format!(
            "{what} only works on the PC Outpost runs on."
        )))
    }
}
