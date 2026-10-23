use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("not found")]
    NotFound,

    #[error("{0}")]
    Conflict(String),

    #[error("{0}")]
    Invalid(String),

    #[error("not allowed: {0}")]
    Forbidden(String),

    /// Wrong PIN or password. `attempts_left` before the account locks.
    #[error("wrong PIN or password")]
    BadSecret { attempts_left: i64 },

    /// Too many wrong attempts; try again after `retry_after` seconds.
    #[error("account locked, try again in {retry_after}s")]
    Locked { retry_after: i64 },

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("internal error: {0}")]
    Internal(String),
}
