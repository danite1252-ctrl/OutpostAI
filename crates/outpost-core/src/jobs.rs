//! Create jobs: one queue for the GPU, shared by every account, first come first
//! served. The gateway's worker takes the oldest queued job, runs it in ComfyUI and
//! records how it went.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn finished(self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Job {
    pub id: String,
    pub account_id: String,
    pub pack_id: String,
    /// The values given, as JSON text.
    pub inputs: String,
    pub status: JobStatus,
    pub error: Option<String>,
    pub seed: Option<i64>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

const COLS: &str = "id, account_id, pack_id, inputs, status, error, seed, created_at, started_at, finished_at";

/// Most jobs one account may have waiting at once.
pub const MAX_QUEUED_PER_ACCOUNT: i64 = 20;

/// Refuse a new job if the account already has [`MAX_QUEUED_PER_ACCOUNT`] waiting.
pub async fn check_room(pool: &SqlitePool, account_id: &str) -> CoreResult<()> {
    let waiting: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE account_id = ? AND status IN ('queued', 'running')")
            .bind(account_id)
            .fetch_one(pool)
            .await?;
    if waiting >= MAX_QUEUED_PER_ACCOUNT {
        return Err(CoreError::Conflict(format!(
            "You already have {waiting} jobs waiting. Let some finish first."
        )));
    }
    Ok(())
}

/// Jobs waiting to start, for everyone.
pub async fn queued_count(pool: &SqlitePool) -> CoreResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'queued'")
        .fetch_one(pool)
        .await?)
}

pub async fn create(pool: &SqlitePool, account_id: &str, pack_id: &str, inputs: &serde_json::Value) -> CoreResult<Job> {
    let waiting: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE account_id = ? AND status IN ('queued', 'running')")
            .bind(account_id)
            .fetch_one(pool)
            .await?;
    if waiting >= MAX_QUEUED_PER_ACCOUNT {
        return Err(CoreError::Conflict(format!(
            "You already have {waiting} jobs waiting. Let some finish first."
        )));
    }
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.into(),
        pack_id: pack_id.into(),
        inputs: inputs.to_string(),
        status: JobStatus::Queued,
        error: None,
        seed: None,
        created_at: now(),
        started_at: None,
        finished_at: None,
    };
    sqlx::query(
        "INSERT INTO jobs (id, account_id, pack_id, inputs, status, created_at) VALUES (?, ?, ?, ?, 'queued', ?)",
    )
    .bind(&job.id)
    .bind(&job.account_id)
    .bind(&job.pack_id)
    .bind(&job.inputs)
    .bind(job.created_at)
    .execute(pool)
    .await?;
    Ok(job)
}

/// One of my jobs.
pub async fn get(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Job> {
    sqlx::query_as(&format!("SELECT {COLS} FROM jobs WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// Any job, for the worker.
pub async fn get_any(pool: &SqlitePool, id: &str) -> CoreResult<Job> {
    sqlx::query_as(&format!("SELECT {COLS} FROM jobs WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}

/// My recent jobs, newest first.
pub async fn list(pool: &SqlitePool, account_id: &str, limit: i64) -> CoreResult<Vec<Job>> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLS} FROM jobs WHERE account_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?"
    ))
    .bind(account_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

/// How many jobs (anyone's) are ahead of this queued one; 0 = next.
pub async fn position(pool: &SqlitePool, job: &Job) -> CoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE status IN ('queued', 'running')
         AND (created_at < ? OR (created_at = ? AND rowid < (SELECT rowid FROM jobs WHERE id = ?)))",
    )
    .bind(job.created_at)
    .bind(job.created_at)
    .bind(&job.id)
    .fetch_one(pool)
    .await?)
}

/// The oldest queued job, marked running. `None` when the queue is empty.
pub async fn take_next(pool: &SqlitePool) -> CoreResult<Option<Job>> {
    let mut tx = pool.begin().await?;
    let next: Option<Job> = sqlx::query_as(&format!(
        "SELECT {COLS} FROM jobs WHERE status = 'queued' ORDER BY created_at, rowid LIMIT 1"
    ))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(mut job) = next else {
        return Ok(None);
    };
    let t = now();
    sqlx::query("UPDATE jobs SET status = 'running', started_at = ? WHERE id = ? AND status = 'queued'")
        .bind(t)
        .bind(&job.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    job.status = JobStatus::Running;
    job.started_at = Some(t);
    Ok(Some(job))
}

pub async fn set_seed(pool: &SqlitePool, id: &str, seed: u64) -> CoreResult<()> {
    sqlx::query("UPDATE jobs SET seed = ? WHERE id = ?")
        .bind(seed as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Record how a job ended.
pub async fn finish(pool: &SqlitePool, id: &str, status: JobStatus, error: Option<&str>) -> CoreResult<()> {
    sqlx::query("UPDATE jobs SET status = ?, error = ?, finished_at = ? WHERE id = ?")
        .bind(status)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cancel one of my jobs. A queued job is cancelled here; for a running one the
/// caller also stops ComfyUI. Returns the status it had.
pub async fn cancel(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<JobStatus> {
    let job = get(pool, account_id, id).await?;
    if job.status == JobStatus::Queued {
        let changed =
            sqlx::query("UPDATE jobs SET status = 'cancelled', finished_at = ? WHERE id = ? AND status = 'queued'")
                .bind(now())
                .bind(id)
                .execute(pool)
                .await?
                .rows_affected();
        if changed == 0 {
            // The worker took it a moment ago: report what it is now.
            return Ok(get(pool, account_id, id).await?.status);
        }
    }
    Ok(job.status)
}

/// At start-up: jobs left "running" by a closed Outpost failed.
pub async fn recover(pool: &SqlitePool) -> CoreResult<u64> {
    Ok(sqlx::query(
        "UPDATE jobs SET status = 'failed', error = 'Outpost closed while this was running.', finished_at = ?
         WHERE status = 'running'",
    )
    .bind(now())
    .execute(pool)
    .await?
    .rows_affected())
}

/// A seed JavaScript can hold exactly (below 2^53).
pub fn random_seed() -> u64 {
    let mut b = [0u8; 8];
    getrandom::getrandom(&mut b).expect("system random numbers");
    u64::from_le_bytes(b) & ((1u64 << 53) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Role};
    use crate::db::open_in_memory;
    use serde_json::json;

    async fn account(pool: &SqlitePool, name: &str) -> String {
        accounts::create(
            pool,
            NewAccount {
                name: name.into(),
                role: Role::Member,
                secret: Some("4821".into()),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn queue_runs_in_order_and_keeps_jobs_private() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let eve = account(&pool, "Eve").await;
        let a = create(&pool, &dan, "z_image_turbo", &json!({"prompt": "one"}))
            .await
            .unwrap();
        let b = create(&pool, &eve, "z_image_turbo", &json!({"prompt": "two"}))
            .await
            .unwrap();
        assert_eq!(position(&pool, &a).await.unwrap(), 0);
        assert_eq!(position(&pool, &b).await.unwrap(), 1);
        assert!(matches!(get(&pool, &eve, &a.id).await, Err(CoreError::NotFound)));

        let first = take_next(&pool).await.unwrap().unwrap();
        assert_eq!(first.id, a.id);
        assert_eq!(first.status, JobStatus::Running);
        set_seed(&pool, &a.id, 123).await.unwrap();
        finish(&pool, &a.id, JobStatus::Done, None).await.unwrap();
        let done = get(&pool, &dan, &a.id).await.unwrap();
        assert_eq!(done.status, JobStatus::Done);
        assert_eq!(done.seed, Some(123));
        assert_eq!(position(&pool, &b).await.unwrap(), 0);

        // Cancel a queued job; the queue is then empty.
        assert_eq!(cancel(&pool, &eve, &b.id).await.unwrap(), JobStatus::Queued);
        assert_eq!(get(&pool, &eve, &b.id).await.unwrap().status, JobStatus::Cancelled);
        assert!(take_next(&pool).await.unwrap().is_none());
        assert_eq!(list(&pool, &dan, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn running_jobs_fail_after_a_restart() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        create(&pool, &dan, "z_image_turbo", &json!({})).await.unwrap();
        let job = take_next(&pool).await.unwrap().unwrap();
        assert_eq!(recover(&pool).await.unwrap(), 1);
        let after = get(&pool, &dan, &job.id).await.unwrap();
        assert_eq!(after.status, JobStatus::Failed);
        assert!(after.error.unwrap().contains("closed"));
    }

    #[tokio::test]
    async fn one_account_cant_flood_the_queue() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        for _ in 0..MAX_QUEUED_PER_ACCOUNT {
            create(&pool, &dan, "z_image_turbo", &json!({})).await.unwrap();
        }
        assert!(matches!(
            create(&pool, &dan, "z_image_turbo", &json!({})).await,
            Err(CoreError::Conflict(_))
        ));
    }

    #[test]
    fn seeds_fit_in_javascript() {
        for _ in 0..100 {
            assert!(random_seed() < (1u64 << 53));
        }
    }
}
