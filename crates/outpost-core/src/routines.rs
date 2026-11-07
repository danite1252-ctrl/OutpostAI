//! Routines: a persona doing something on its own, on a schedule ("every weekday at 7:30,
//! give me a morning brief"). Each run is a reply in the routine's own chat, so every result
//! can be read, or talked about further.
//!
//! Times are kept in the person's own time zone (daylight saving included), not the server's.

use jiff::tz::{Offset, TimeZone};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;

use crate::{now, CoreError, CoreResult};

/// Most routines one account may have.
pub const MAX_PER_ACCOUNT: i64 = 50;
const NAME_MAX: usize = 80;
const INSTRUCTIONS_MAX: usize = 4000;

/// When a routine runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// Once, at this time (Unix seconds).
    Once { at: i64 },
    /// At this time of day ("07:30", 24-hour), on these days (0 = Sunday … 6 = Saturday).
    Days { time: String, days: Vec<u8> },
    /// Every so many hours (1 to 168).
    Every { hours: u32 },
    /// Never on its own: only from its link, or Run now.
    Link,
    /// When something happens in Outpost: `job_finished` (a picture, video or 3D model of
    /// yours finished) or `training_finished` (a character's training).
    Event { event: String },
}

/// What can start a routine that runs when something happens.
pub const EVENTS: [&str; 2] = ["job_finished", "training_finished"];

impl Schedule {
    fn check(&self) -> CoreResult<()> {
        match self {
            Schedule::Once { at } if *at <= 0 => Err(CoreError::Invalid("Pick when it should run.".into())),
            Schedule::Days { time, days } => {
                hour_minute(time).ok_or_else(|| CoreError::Invalid("Give the time like 07:30.".into()))?;
                if days.is_empty() || days.iter().any(|d| *d > 6) {
                    return Err(CoreError::Invalid("Pick at least one day.".into()));
                }
                Ok(())
            }
            Schedule::Every { hours } if !(1..=168).contains(hours) => {
                Err(CoreError::Invalid("Every 1 to 168 hours.".into()))
            }
            Schedule::Event { event } if !EVENTS.contains(&event.as_str()) => {
                Err(CoreError::Invalid("Pick what starts it.".into()))
            }
            _ => Ok(()),
        }
    }

    /// The first time it's due after `after` (Unix seconds), in that time zone. `None`: never
    /// again (a one-off that's been).
    pub fn next_after(&self, zone: &TimeZone, after: i64) -> Option<i64> {
        match self {
            Schedule::Once { at } => (*at > after).then_some(*at),
            Schedule::Every { hours } => Some(after + i64::from(*hours) * 3600),
            Schedule::Link | Schedule::Event { .. } => None,
            Schedule::Days { time, days } => {
                let (h, m) = hour_minute(time)?;
                let start = Timestamp::from_second(after).ok()?.to_zoned(zone.clone());
                let mut date = start.date();
                for _ in 0..9 {
                    let weekday = date.weekday().to_sunday_zero_offset() as u8;
                    if days.contains(&weekday) {
                        if let Ok(at) = date.at(h, m, 0, 0).to_zoned(zone.clone()) {
                            let at = at.timestamp().as_second();
                            if at > after {
                                return Some(at);
                            }
                        }
                    }
                    date = date.tomorrow().ok()?;
                }
                None
            }
        }
    }
}

/// When a routine tells you (a notification on your phone and PC) about a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Notify {
    /// Every run.
    #[default]
    Always,
    /// Only when there's something new or worth saying (the persona decides), and failures.
    Worth,
    /// Only when it fails.
    Failures,
    /// Never (look in its chat).
    Never,
}

impl Notify {
    fn as_str(self) -> &'static str {
        match self {
            Notify::Always => "always",
            Notify::Worth => "worth",
            Notify::Failures => "failures",
            Notify::Never => "never",
        }
    }
    fn parse(s: &str) -> Notify {
        match s {
            "worth" => Notify::Worth,
            "failures" => Notify::Failures,
            "never" => Notify::Never,
            _ => Notify::Always,
        }
    }
    /// Should this run be told? `ok`: it worked; `news`: it had something to say.
    pub fn tells(self, ok: bool, news: bool) -> bool {
        match self {
            Notify::Always => true,
            Notify::Worth => !ok || news,
            Notify::Failures => !ok,
            Notify::Never => false,
        }
    }
}

fn hour_minute(time: &str) -> Option<(i8, i8)> {
    let (h, m) = time.trim().split_once(':')?;
    let (h, m): (i8, i8) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m))
}

/// A person's time zone: by name when known ("Australia/Sydney", daylight saving and all),
/// else their UTC offset, else UTC.
pub fn zone(name: Option<&str>, offset_minutes: Option<i32>) -> TimeZone {
    if let Some(tz) = name.and_then(|n| TimeZone::get(n).ok()) {
        return tz;
    }
    match offset_minutes.and_then(|m| Offset::from_seconds(m * 60).ok()) {
        Some(o) => TimeZone::fixed(o),
        None => TimeZone::UTC,
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Routine {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub persona_id: Option<String>,
    pub instructions: String,
    pub schedule: Schedule,
    pub time_zone: Option<String>,
    pub utc_offset_minutes: Option<i32>,
    pub enabled: bool,
    /// When it tells you about a run.
    pub notify: Notify,
    /// It can be started from its secret link.
    pub has_link: bool,
    /// Its chat (where each run's reply goes), once it has run.
    pub chat_id: Option<String>,
    /// When it runs next (Unix seconds); none when paused or finished.
    pub next_at: Option<i64>,
    pub last_at: Option<i64>,
    /// "ok", "failed" or "running".
    pub last_status: Option<String>,
    pub last_note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Routine {
    pub fn zone(&self) -> TimeZone {
        zone(self.time_zone.as_deref(), self.utc_offset_minutes)
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    account_id: String,
    name: String,
    persona_id: Option<String>,
    instructions: String,
    schedule: String,
    time_zone: Option<String>,
    utc_offset_minutes: Option<i64>,
    enabled: i64,
    notify: String,
    has_link: i64,
    chat_id: Option<String>,
    next_at: Option<i64>,
    last_at: Option<i64>,
    last_status: Option<String>,
    last_note: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl Row {
    fn into_routine(self) -> CoreResult<Routine> {
        Ok(Routine {
            schedule: serde_json::from_str(&self.schedule).map_err(|e| CoreError::Internal(e.to_string()))?,
            id: self.id,
            account_id: self.account_id,
            name: self.name,
            persona_id: self.persona_id,
            instructions: self.instructions,
            time_zone: self.time_zone,
            utc_offset_minutes: self.utc_offset_minutes.map(|m| m as i32),
            enabled: self.enabled != 0,
            notify: Notify::parse(&self.notify),
            has_link: self.has_link != 0,
            chat_id: self.chat_id,
            next_at: self.next_at,
            last_at: self.last_at,
            last_status: self.last_status,
            last_note: self.last_note,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const COLUMNS: &str = "id, account_id, name, persona_id, instructions, schedule, time_zone, utc_offset_minutes, enabled, \
                       notify, hook_hash IS NOT NULL AS has_link, chat_id, next_at, last_at, last_status, last_note, created_at, updated_at";

/// A new routine, or changes to one (fields left out stay as they are).
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct RoutineInput {
    pub name: Option<String>,
    /// The persona that does it (its instructions, model and tools). Empty: no persona.
    pub persona_id: Option<String>,
    /// What to do, in plain words.
    pub instructions: Option<String>,
    pub schedule: Option<Schedule>,
    pub enabled: Option<bool>,
    /// When it tells you about a run.
    pub notify: Option<Notify>,
    /// The person's time zone ("Australia/Sydney"), from their app.
    pub time_zone: Option<String>,
    /// Minutes ahead of UTC, from their app (used when the zone's name isn't known).
    pub utc_offset_minutes: Option<i32>,
}

pub async fn list(pool: &SqlitePool, account_id: &str) -> CoreResult<Vec<Routine>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM routines WHERE account_id = ? ORDER BY name COLLATE NOCASE"))
        .bind(account_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(Row::into_routine).collect()
}

pub async fn get(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<Routine> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM routines WHERE id = ? AND account_id = ?"))
        .bind(id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    row.ok_or(CoreError::NotFound)?.into_routine()
}

/// Routines due now (any account), enabled, oldest first.
pub async fn due(pool: &SqlitePool, at: i64) -> CoreResult<Vec<Routine>> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM routines WHERE enabled = 1 AND next_at IS NOT NULL AND next_at <= ? ORDER BY next_at"
    ))
    .bind(at)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(Row::into_routine).collect()
}

fn clean_name(name: &str) -> CoreResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("Give the routine a name.".into()));
    }
    Ok(name.chars().take(NAME_MAX).collect())
}

fn clean_instructions(text: &str) -> CoreResult<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(CoreError::Invalid("Say what the routine should do.".into()));
    }
    if text.chars().count() > INSTRUCTIONS_MAX {
        return Err(CoreError::Invalid(format!("Keep the instructions under {INSTRUCTIONS_MAX} characters.")));
    }
    Ok(text.to_string())
}

fn next_for(schedule: &Schedule, enabled: bool, zone: &TimeZone, last: Option<i64>) -> Option<i64> {
    if !enabled {
        return None;
    }
    let t = now();
    match schedule {
        // Every few hours: that long after its last run (or after now, for a new one).
        Schedule::Every { hours } => {
            let gap = i64::from(*hours) * 3600;
            Some(last.map_or(t + gap, |l| (l + gap).max(t + 60)))
        }
        s => s.next_after(zone, t),
    }
}

pub async fn create(pool: &SqlitePool, account_id: &str, input: RoutineInput) -> CoreResult<Routine> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM routines WHERE account_id = ?")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    if count >= MAX_PER_ACCOUNT {
        return Err(CoreError::Conflict(format!("An account can have up to {MAX_PER_ACCOUNT} routines.")));
    }
    let name = clean_name(input.name.as_deref().unwrap_or_default())?;
    let instructions = clean_instructions(input.instructions.as_deref().unwrap_or_default())?;
    let schedule = input.schedule.ok_or_else(|| CoreError::Invalid("Pick when it runs.".into()))?;
    schedule.check()?;
    let persona_id = input.persona_id.filter(|p| !p.trim().is_empty());
    if let Some(p) = &persona_id {
        crate::personas::get_usable(pool, account_id, p).await?;
    }
    let enabled = input.enabled.unwrap_or(true);
    let zone = zone(input.time_zone.as_deref(), input.utc_offset_minutes);
    let next_at = next_for(&schedule, enabled, &zone, None);
    let t = now();
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO routines (id, account_id, name, persona_id, instructions, schedule, time_zone, utc_offset_minutes, \
         enabled, notify, next_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(account_id)
    .bind(&name)
    .bind(&persona_id)
    .bind(&instructions)
    .bind(serde_json::to_string(&schedule).map_err(|e| CoreError::Internal(e.to_string()))?)
    .bind(&input.time_zone)
    .bind(input.utc_offset_minutes)
    .bind(enabled as i64)
    .bind(input.notify.unwrap_or_default().as_str())
    .bind(next_at)
    .bind(t)
    .bind(t)
    .execute(pool)
    .await?;
    get(pool, account_id, &id).await
}

pub async fn update(pool: &SqlitePool, account_id: &str, id: &str, input: RoutineInput) -> CoreResult<Routine> {
    let r = get(pool, account_id, id).await?;
    let name = match &input.name {
        Some(n) => clean_name(n)?,
        None => r.name.clone(),
    };
    let instructions = match &input.instructions {
        Some(i) => clean_instructions(i)?,
        None => r.instructions.clone(),
    };
    let schedule = match input.schedule {
        Some(s) => {
            s.check()?;
            s
        }
        None => r.schedule.clone(),
    };
    let persona_id = match input.persona_id {
        Some(p) if p.trim().is_empty() => None,
        Some(p) => {
            crate::personas::get_usable(pool, account_id, &p).await?;
            Some(p)
        }
        None => r.persona_id.clone(),
    };
    let enabled = input.enabled.unwrap_or(r.enabled);
    let notify = input.notify.unwrap_or(r.notify);
    let time_zone = input.time_zone.or(r.time_zone.clone());
    let offset = input.utc_offset_minutes.or(r.utc_offset_minutes);
    let next_at = next_for(&schedule, enabled, &zone(time_zone.as_deref(), offset), r.last_at);
    sqlx::query(
        "UPDATE routines SET name = ?, persona_id = ?, instructions = ?, schedule = ?, time_zone = ?, utc_offset_minutes = ?, \
         enabled = ?, notify = ?, next_at = ?, updated_at = ? WHERE id = ? AND account_id = ?",
    )
    .bind(&name)
    .bind(&persona_id)
    .bind(&instructions)
    .bind(serde_json::to_string(&schedule).map_err(|e| CoreError::Internal(e.to_string()))?)
    .bind(&time_zone)
    .bind(offset)
    .bind(enabled as i64)
    .bind(notify.as_str())
    .bind(next_at)
    .bind(now())
    .bind(id)
    .bind(account_id)
    .execute(pool)
    .await?;
    get(pool, account_id, id).await
}

pub async fn delete(pool: &SqlitePool, account_id: &str, id: &str) -> CoreResult<()> {
    let done = sqlx::query("DELETE FROM routines WHERE id = ? AND account_id = ?")
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

/// Give a routine a new secret link (any old one stops working), or take it away. Returns
/// the link's token, shown this once.
pub async fn set_link(pool: &SqlitePool, account_id: &str, id: &str, on: bool) -> CoreResult<Option<String>> {
    get(pool, account_id, id).await?;
    let (token, hash) = if on {
        let (t, h) = crate::secret::new_token()?;
        (Some(t), Some(h))
    } else {
        (None, None)
    };
    sqlx::query("UPDATE routines SET hook_hash = ?, updated_at = ? WHERE id = ? AND account_id = ?")
        .bind(&hash)
        .bind(now())
        .bind(id)
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(token)
}

/// The routine a link's token starts, if any.
pub async fn by_link(pool: &SqlitePool, token: &str) -> CoreResult<Option<Routine>> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM routines WHERE hook_hash = ?"))
        .bind(crate::secret::hash_token(token))
        .fetch_optional(pool)
        .await?;
    row.map(Row::into_routine).transpose()
}

/// My routines (switched on) that start when this happens.
pub async fn on_event(pool: &SqlitePool, account_id: &str, event: &str) -> CoreResult<Vec<Routine>> {
    Ok(list(pool, account_id)
        .await?
        .into_iter()
        .filter(|r| r.enabled && matches!(&r.schedule, Schedule::Event { event: e } if e == event))
        .collect())
}

/// The routine's chat, once made.
pub async fn set_chat(pool: &SqlitePool, id: &str, chat_id: &str) -> CoreResult<()> {
    sqlx::query("UPDATE routines SET chat_id = ? WHERE id = ?").bind(chat_id).bind(id).execute(pool).await?;
    Ok(())
}

/// A run starts: it's marked running, and its next time is worked out now (so it isn't
/// started twice).
pub async fn start_run(pool: &SqlitePool, r: &Routine, reason: &str) -> CoreResult<String> {
    let t = now();
    let next_at = match &r.schedule {
        Schedule::Once { .. } if reason == "schedule" => None,
        s => next_for(s, r.enabled, &r.zone(), Some(t)),
    };
    let next_at = if reason == "schedule" { next_at } else { r.next_at.filter(|n| *n > t).or(next_at) };
    // A one-off that's run stays on, with nothing next: done (not paused).
    let enabled = r.enabled;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO routine_runs (id, routine_id, started_at, status, reason) VALUES (?, ?, ?, 'running', ?)")
        .bind(&id)
        .bind(&r.id)
        .bind(t)
        .bind(reason)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE routines SET last_at = ?, last_status = 'running', last_note = NULL, next_at = ?, enabled = ? WHERE id = ?")
        .bind(t)
        .bind(next_at)
        .bind(enabled as i64)
        .bind(&r.id)
        .execute(pool)
        .await?;
    Ok(id)
}

/// A run ended: `ok` (with the reply it made) or `failed` (and why).
/// `tell`: it's worth a notification.
pub async fn finish_run(
    pool: &SqlitePool,
    routine_id: &str,
    run_id: &str,
    ok: bool,
    message_id: Option<&str>,
    note: Option<&str>,
    tell: bool,
) -> CoreResult<()> {
    let status = if ok { "ok" } else { "failed" };
    sqlx::query("UPDATE routine_runs SET finished_at = ?, status = ?, message_id = ?, note = ?, tell = ? WHERE id = ?")
        .bind(now())
        .bind(status)
        .bind(message_id)
        .bind(note)
        .bind(tell as i64)
        .bind(run_id)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE routines SET last_status = ?, last_note = ? WHERE id = ?")
        .bind(status)
        .bind(note)
        .bind(routine_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Runs left "running" by a restart are failed (they can't finish now).
pub async fn fail_interrupted(pool: &SqlitePool) -> CoreResult<()> {
    let note = "Outpost restarted while this was running.";
    sqlx::query("UPDATE routine_runs SET status = 'failed', finished_at = ?, note = ? WHERE status = 'running'")
        .bind(now())
        .bind(note)
        .execute(pool)
        .await?;
    sqlx::query("UPDATE routines SET last_status = 'failed', last_note = ? WHERE last_status = 'running'")
        .bind(note)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct Run {
    pub id: String,
    pub routine_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub message_id: Option<String>,
    pub note: Option<String>,
    /// "schedule" or "now" (Run now).
    pub reason: String,
}

/// A finished run worth telling someone about (for notifications).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Told {
    pub id: String,
    pub name: String,
    pub chat_id: Option<String>,
    pub status: String,
    pub note: Option<String>,
    pub finished_at: i64,
}

/// My routines' runs that finished since a time and should be told.
pub async fn told_since(pool: &SqlitePool, account_id: &str, since: i64) -> CoreResult<Vec<Told>> {
    Ok(sqlx::query_as(
        "SELECT rr.id, r.name, r.chat_id, rr.status, rr.note, rr.finished_at FROM routine_runs rr \
         JOIN routines r ON r.id = rr.routine_id \
         WHERE r.account_id = ? AND rr.tell = 1 AND rr.finished_at >= ? ORDER BY rr.finished_at LIMIT 20",
    )
    .bind(account_id)
    .bind(since)
    .fetch_all(pool)
    .await?)
}

/// When my next routine that tells me anything is due, and how many of mine are running now
/// (so a phone knows when to look).
pub async fn watch_times(pool: &SqlitePool, account_id: &str) -> CoreResult<(Option<i64>, i64)> {
    let next: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(next_at) FROM routines WHERE account_id = ? AND enabled = 1 AND next_at IS NOT NULL AND notify <> 'never'",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    let running: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM routines WHERE account_id = ? AND last_status = 'running'")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    Ok((next, running))
}

/// A routine's latest runs, newest first.
pub async fn runs(pool: &SqlitePool, account_id: &str, routine_id: &str, limit: i64) -> CoreResult<Vec<Run>> {
    get(pool, account_id, routine_id).await?;
    Ok(sqlx::query_as(
        "SELECT id, routine_id, started_at, finished_at, status, message_id, note, reason FROM routine_runs \
         WHERE routine_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(routine_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_run_at_that_time_in_the_persons_own_zone() {
        let sydney = zone(Some("Australia/Sydney"), None);
        // Fri 6 Nov 2026, 20:00 in Sydney (UTC+11, summer time) = 09:00 UTC.
        let fri_8pm = 1_793_955_600;
        let weekdays = Schedule::Days { time: "07:30".into(), days: vec![1, 2, 3, 4, 5] };
        let next = weekdays.next_after(&sydney, fri_8pm).unwrap();
        // The weekend is skipped: Monday 9 Nov, 07:30 Sydney = Sunday 8 Nov 20:30 UTC.
        assert_eq!(next, 1_794_169_800, "{next}");
        // Every day: tomorrow (Saturday) at 07:30.
        let daily = Schedule::Days { time: "07:30".into(), days: (0..7).collect() };
        assert_eq!(daily.next_after(&sydney, fri_8pm).unwrap(), 1_793_997_000);
        // Later the same day.
        let tonight = Schedule::Days { time: "21:15".into(), days: vec![5] };
        assert_eq!(tonight.next_after(&sydney, fri_8pm).unwrap(), fri_8pm + 75 * 60);
        // Once: only if it's still ahead.
        assert_eq!(Schedule::Once { at: fri_8pm + 10 }.next_after(&sydney, fri_8pm), Some(fri_8pm + 10));
        assert_eq!(Schedule::Once { at: fri_8pm - 10 }.next_after(&sydney, fri_8pm), None);
        // Bad times and days are refused.
        assert!(Schedule::Days { time: "25:00".into(), days: vec![1] }.check().is_err());
        assert!(Schedule::Days { time: "07:30".into(), days: vec![] }.check().is_err());
        assert!(Schedule::Every { hours: 0 }.check().is_err());
    }

    #[tokio::test]
    async fn a_one_off_is_done_after_it_runs_not_paused() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let me = crate::accounts::create(
            &pool,
            crate::accounts::NewAccount { name: "Dan".into(), role: crate::accounts::Role::Owner, secret: Some("4821".into()) },
        )
        .await
        .unwrap();
        let r = create(
            &pool,
            &me.id,
            RoutineInput {
                name: Some("Once".into()),
                instructions: Some("Say hi.".into()),
                schedule: Some(Schedule::Once { at: now() + 600 }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        start_run(&pool, &r, "schedule").await.unwrap();
        let after = get(&pool, &me.id, &r.id).await.unwrap();
        assert!(after.enabled, "still on");
        assert_eq!(after.next_at, None, "nothing next: done");
    }

    #[tokio::test]
    async fn a_link_starts_its_routine_until_its_taken_away() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let me = crate::accounts::create(
            &pool,
            crate::accounts::NewAccount { name: "Dan".into(), role: crate::accounts::Role::Owner, secret: Some("4821".into()) },
        )
        .await
        .unwrap();
        let r = create(
            &pool,
            &me.id,
            RoutineInput {
                name: Some("Door".into()),
                instructions: Some("Tell me who's at the door.".into()),
                schedule: Some(Schedule::Link),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(r.next_at, None, "never on its own");
        assert!(!r.has_link);
        let token = set_link(&pool, &me.id, &r.id, true).await.unwrap().unwrap();
        assert_eq!(by_link(&pool, &token).await.unwrap().unwrap().id, r.id);
        assert!(get(&pool, &me.id, &r.id).await.unwrap().has_link);
        // A new link: the old one stops working.
        let newer = set_link(&pool, &me.id, &r.id, true).await.unwrap().unwrap();
        assert!(by_link(&pool, &token).await.unwrap().is_none());
        set_link(&pool, &me.id, &r.id, false).await.unwrap();
        assert!(by_link(&pool, &newer).await.unwrap().is_none());
        // Events: only the right one, only when switched on.
        let e = create(
            &pool,
            &me.id,
            RoutineInput {
                name: Some("Shared".into()),
                instructions: Some("Post it.".into()),
                schedule: Some(Schedule::Event { event: "job_finished".into() }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(on_event(&pool, &me.id, "job_finished").await.unwrap()[0].id, e.id);
        assert!(on_event(&pool, &me.id, "training_finished").await.unwrap().is_empty());
        assert!(Schedule::Event { event: "nope".into() }.check().is_err());
    }

    #[test]
    fn who_gets_told() {
        assert!(Notify::Always.tells(true, false));
        assert!(Notify::Worth.tells(true, true) && !Notify::Worth.tells(true, false) && Notify::Worth.tells(false, false));
        assert!(Notify::Failures.tells(false, true) && !Notify::Failures.tells(true, true));
        assert!(!Notify::Never.tells(false, true));
    }

    #[test]
    fn without_a_zone_name_the_offset_is_used() {
        let plus11 = zone(None, Some(660));
        let daily = Schedule::Days { time: "07:30".into(), days: (0..7).collect() };
        assert_eq!(daily.next_after(&plus11, 1_793_955_600).unwrap(), 1_793_997_000);
        // Nothing known: UTC.
        let utc = zone(Some("Not/AZone"), None);
        assert_eq!(daily.next_after(&utc, 1_793_955_600).unwrap(), 1_794_036_600);
    }
}
