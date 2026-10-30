//! Outpost core: everything that touches the database or secrets.
//!
//! The gateway (HTTP) and the desktop shell both build on this crate; nothing
//! here knows about HTTP.

pub mod accounts;
pub mod attachments;
pub mod audit;
pub mod auth;
pub mod chats;
pub mod db;
pub mod documents;
pub mod error;
pub mod jobs;
pub mod media;
pub mod media_index;
pub mod memory;
pub mod pairing;
pub mod personas;
pub mod secret;
pub mod sessions;
pub mod settings;
pub mod training;
pub mod trigger;
pub mod webhooks;

pub use error::{CoreError, CoreResult};

/// Current time as Unix seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
