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

/// My jobs waiting or running.
pub async fn waiting_for(pool: &SqlitePool, account_id: &str) -> CoreResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE account_id = ? AND status IN ('queued', 'running')")
            .bind(account_id)
            .fetch_one(pool)
            .await?,
    )
}

/// Jobs waiting to start, for everyone.
pub async fn queued_count(pool: &SqlitePool) -> CoreResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'queued'")
        .fetch_one(pool)
        .await?)
}

/// Who goes first on the GPU: someone at the desktop, then phones, then app keys.
pub fn priority_for(kind: crate::sessions::SessionKind) -> i64 {
    match kind {
        crate::sessions::SessionKind::Desktop => 0,
        crate::sessions::SessionKind::Device => 1,
        crate::sessions::SessionKind::App => 2,
    }
}

/// Queue a job. `priority` (see [`priority_for`]) puts it ahead of waiting jobs with a
/// higher number; within a priority it's first come, first served.
pub async fn create(
    pool: &SqlitePool,
    account_id: &str,
    pack_id: &str,
    inputs: &serde_json::Value,
    priority: i64,
) -> CoreResult<Job> {
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
        "INSERT INTO jobs (id, account_id, pack_id, inputs, status, created_at, priority) VALUES (?, ?, ?, ?, 'queued', ?, ?)",
    )
    .bind(&job.id)
    .bind(&job.account_id)
    .bind(&job.pack_id)
    .bind(&job.inputs)
    .bind(job.created_at)
    .bind(priority.clamp(0, 9))
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

/// The order queued jobs run in: priority, then first come, first served.
const RUN_ORDER: &str = "priority, created_at, rowid";

/// How many jobs (anyone's) are ahead of this queued one; 0 = next. The running job
/// counts as ahead.
pub async fn position(pool: &SqlitePool, job: &Job) -> CoreResult<i64> {
    Ok(sqlx::query_scalar(
        "WITH me AS (SELECT priority, created_at, rowid AS r FROM jobs WHERE id = ?)
         SELECT COUNT(*) FROM jobs, me WHERE jobs.status = 'running'
            OR (jobs.status = 'queued' AND (jobs.priority < me.priority
                OR (jobs.priority = me.priority AND (jobs.created_at < me.created_at
                    OR (jobs.created_at = me.created_at AND jobs.rowid < me.r)))))",
    )
    .bind(&job.id)
    .fetch_one(pool)
    .await?)
}

/// One job in the shared line for the GPU: whose it is, not what it makes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct LineEntry {
    pub job_id: String,
    pub account_id: String,
    pub account_name: String,
    pub avatar_color: String,
    pub pack_id: String,
    pub status: JobStatus,
}

/// Everyone's running and waiting jobs, in the order they run.
pub async fn line(pool: &SqlitePool) -> CoreResult<Vec<LineEntry>> {
    Ok(sqlx::query_as(
        "SELECT j.id AS job_id, j.account_id, a.name AS account_name, a.avatar_color, j.pack_id, j.status
         FROM jobs j JOIN accounts a ON a.id = j.account_id
         WHERE j.status IN ('queued', 'running')
         ORDER BY CASE j.status WHEN 'running' THEN 0 ELSE 1 END, j.priority, j.created_at, j.rowid",
    )
    .fetch_all(pool)
    .await?)
}

/// The oldest queued job, marked running. `None` when the queue is empty.
pub async fn take_next(pool: &SqlitePool) -> CoreResult<Option<Job>> {
    // One statement: picking and marking can't be split by another write (a read-then-
    // write transaction would fail rather than wait when one lands in between).
    Ok(sqlx::query_as(&format!(
        "UPDATE jobs SET status = 'running', started_at = ?
         WHERE id = (SELECT id FROM jobs WHERE status = 'queued' ORDER BY {RUN_ORDER} LIMIT 1) AND status = 'queued'
         RETURNING {COLS}"
    ))
    .bind(now())
    .fetch_optional(pool)
    .await?)
}

/// The oldest queued job whose pack is one of `packs`, marked running. For running some
/// jobs here and others on a helper PC.
pub async fn take_next_of(pool: &SqlitePool, packs: &[String]) -> CoreResult<Option<Job>> {
    if packs.is_empty() {
        return Ok(None);
    }
    let marks = vec!["?"; packs.len()].join(", ");
    let sql = format!(
        "UPDATE jobs SET status = 'running', started_at = ?
         WHERE id = (SELECT id FROM jobs WHERE status = 'queued' AND pack_id IN ({marks}) ORDER BY {RUN_ORDER} LIMIT 1)
           AND status = 'queued'
         RETURNING {COLS}"
    );
    let mut q = sqlx::query_as(&sql).bind(now());
    for p in packs {
        q = q.bind(p);
    }
    Ok(q.fetch_optional(pool).await?)
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
    async fn desktop_jobs_go_before_phones_and_apps() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let app = create(&pool, &dan, "z_image_turbo", &json!({}), 2).await.unwrap();
        let phone = create(&pool, &dan, "z_image_turbo", &json!({}), 1).await.unwrap();
        let desk = create(&pool, &dan, "z_image_turbo", &json!({}), 0).await.unwrap();
        assert_eq!(position(&pool, &desk).await.unwrap(), 0);
        assert_eq!(position(&pool, &phone).await.unwrap(), 1);
        assert_eq!(position(&pool, &app).await.unwrap(), 2);
        let order: Vec<_> = line(&pool).await.unwrap().into_iter().map(|e| e.job_id).collect();
        assert_eq!(order, vec![desk.id.clone(), phone.id.clone(), app.id.clone()]);
        assert_eq!(take_next(&pool).await.unwrap().unwrap().id, desk.id);
        // The running job counts as ahead of everyone waiting.
        assert_eq!(position(&pool, &phone).await.unwrap(), 1);
        assert_eq!(position(&pool, &app).await.unwrap(), 2);
        assert_eq!(priority_for(crate::sessions::SessionKind::App), 2);
    }

    #[tokio::test]
    async fn queue_runs_in_order_and_keeps_jobs_private() {
        let pool = open_in_memory().await.unwrap();
        let dan = account(&pool, "Dan").await;
        let eve = account(&pool, "Eve").await;
        let a = create(&pool, &dan, "z_image_turbo", &json!({"prompt": "one"}), 1)
            .await
            .unwrap();
        let b = create(&pool, &eve, "z_image_turbo", &json!({"prompt": "two"}), 1)
            .await
            .unwrap();
        assert_eq!(position(&pool, &a).await.unwrap(), 0);
        assert_eq!(position(&pool, &b).await.unwrap(), 1);
        assert!(matches!(get(&pool, &eve, &a.id).await, Err(CoreError::NotFound)));

        let first = take_next(&pool).await.unwrap().unwrap();
        assert_eq!(first.id, a.id);
        // The shared line: running first, then waiting, with names only.
        let l = line(&pool).await.unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].account_name.as_str(), l[0].status), ("Dan", JobStatus::Running));
        assert_eq!((l[1].account_name.as_str(), l[1].status), ("Eve", JobStatus::Queued));
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
        create(&pool, &dan, "z_image_turbo", &json!({}), 1).await.unwrap();
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
            create(&pool, &dan, "z_image_turbo", &json!({}), 1).await.unwrap();
        }
        assert!(matches!(
            create(&pool, &dan, "z_image_turbo", &json!({}), 1).await,
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
