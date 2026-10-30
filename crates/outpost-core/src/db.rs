use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::CoreResult;

/// Open (or create) the Outpost database at `path` and bring its schema up to date.
pub async fn open(path: &Path) -> CoreResult<SqlitePool> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| crate::CoreError::Internal(format!("cannot create {}: {e}", dir.display())))?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Open another Outpost's database to read from (e.g. this PC's own, while the app uses a
/// server). Never changes it: no migrations, no writes.
pub async fn open_read_only(path: &Path) -> CoreResult<SqlitePool> {
    let options = SqliteConnectOptions::new().filename(path).read_only(true);
    Ok(SqlitePoolOptions::new().max_connections(2).connect_with(options).await?)
}

/// An in-memory database for tests. One connection, so every query sees the same data.
pub async fn open_in_memory() -> CoreResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> CoreResult<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
