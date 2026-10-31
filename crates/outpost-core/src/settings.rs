//! Outpost-wide settings, stored as key/value pairs.

use sqlx::SqlitePool;

use crate::CoreResult;

/// "on" when other devices on the network may reach the API (app keys still required).
pub const NETWORK_ACCESS: &str = "network.lan_access";

/// "on" when the owner's Tailscale devices may reach the API and the phone port, even
/// with network access off.
pub const TAILSCALE_ACCESS: &str = "network.tailscale";

/// Where ai-toolkit (the LoRA trainer) is installed. Unset: `C:\dev\ai-toolkit`.
pub const AI_TOOLKIT_DIR: &str = "training.ai_toolkit_dir";

/// The listening (speech-to-text) model voice uses, e.g. `large-v3-turbo`. Unset: `small`.
pub const VOICE_LISTENING: &str = "voice.listening";

/// "off" to stop describing gallery pictures for search (on until turned off).
pub const GALLERY_DESCRIBE: &str = "gallery.describe";

pub async fn get(pool: &SqlitePool, key: &str) -> CoreResult<Option<String>> {
    Ok(sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?)
}

pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> CoreResult<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_bool(pool: &SqlitePool, key: &str) -> CoreResult<bool> {
    Ok(get(pool, key).await?.as_deref() == Some("on"))
}

pub async fn set_bool(pool: &SqlitePool, key: &str, on: bool) -> CoreResult<()> {
    set(pool, key, if on { "on" } else { "off" }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[tokio::test]
    async fn round_trip() {
        let pool = open_in_memory().await.unwrap();
        assert!(!get_bool(&pool, NETWORK_ACCESS).await.unwrap());
        set_bool(&pool, NETWORK_ACCESS, true).await.unwrap();
        assert!(get_bool(&pool, NETWORK_ACCESS).await.unwrap());
        set_bool(&pool, NETWORK_ACCESS, false).await.unwrap();
        assert!(!get_bool(&pool, NETWORK_ACCESS).await.unwrap());
    }
}
