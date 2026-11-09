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

/// What happens to file changes a routine makes in its chat's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Changes {
    /// They wait for you to apply them (and builds wait for them).
    #[default]
    Wait,
    /// They're made straight away (you can undo them).
    Apply,
}

/// What a routine may do while it runs on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Allow {
    /// The chat's tools (web search, Google, the tools on your PC...).
    pub tools: bool,
    pub changes: Changes,
    /// Commands in its folders, but only those already allowed "always" there: nobody is
    /// there to OK others.
    pub commands: bool,
}

impl Default for Allow {
    fn default() -> Self {
        Allow { tools: true, changes: Changes::Wait, commands: true }
    }
}

/// Quiet hours: a run that finishes in them is told when they end ("22:00" to "07:00").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Quiet {
    pub from: String,
    pub to: String,
}

impl Quiet {
    pub fn check(&self) -> CoreResult<()> {
        if hour_minute(&self.from).is_none() || hour_minute(&self.to).is_none() {
            return Err(CoreError::Invalid("Give quiet hours like 22:00 to 07:00.".into()));
        }
        if self.from.trim() == self.to.trim() {
            return Err(CoreError::Invalid("Quiet hours need to start and end at different times.".into()));
        }
        Ok(())
    }

    /// When the quiet hours `t` is in end (Unix seconds), or `None` when `t` isn't in them.
    pub fn ends(&self, zone: &TimeZone, t: i64) -> Option<i64> {
        let (fh, fm) = hour_minute(&self.from)?;
        let (th, tm) = hour_minute(&self.to)?;
        let (from, to) = (i32::from(fh) * 60 + i32::from(fm), i32::from(th) * 60 + i32::from(tm));
        let here = Timestamp::from_second(t).ok()?.to_zoned(zone.clone());
        let m = i32::from(here.hour()) * 60 + i32::from(here.minute());
        let date = here.date();
        let end_date = if from < to {
            if !(from..to).contains(&m) {
                return None;
            }
            date
        } else if m >= from {
            date.tomorrow().ok()?
        } else if m < to {
            date
        } else {
            return None;
        };
        Some(end_date.at(th, tm, 0, 0).to_zoned(zone.clone()).ok()?.timestamp().as_second())
    }
}

fn quiet_key(account_id: &str) -> String {
    format!("routines.quiet.{account_id}")
}

/// My quiet hours, if I have them.
pub async fn quiet(pool: &SqlitePool, account_id: &str) -> CoreResult<Option<Quiet>> {
    Ok(crate::settings::get(pool, &quiet_key(account_id))
        .await?
        .and_then(|v| serde_json::from_str(&v).ok()))
}

/// Set my quiet hours, or take them away (`None`).
pub async fn set_quiet(pool: &SqlitePool, account_id: &str, q: Option<Quiet>) -> CoreResult<Option<Quiet>> {
    let value = match &q {
        Some(q) => {
            q.check()?;
            serde_json::to_string(&Quiet { from: q.from.trim().into(), to: q.to.trim().into() })
                .map_err(|e| CoreError::Internal(e.to_string()))?
        }
        None => String::new(),
    };
    crate::settings::set(pool, &quiet_key(account_id), &value).await?;
    quiet(pool, account_id).await
}

/// When a run of this routine finishing at `t` should be told: then, or when my quiet hours end.
pub async fn tell_time(pool: &SqlitePool, r: &Routine, t: i64) -> i64 {
    match quiet(pool, &r.account_id).await {
        Ok(Some(q)) => q.ends(&r.zone(), t).unwrap_or(t),
        _ => t,
    }
}

fn hour_minute(time: &str) -> Option<(i8, i8)> {
    let (h, m) = time.trim().split_once(':')?;
    let (h, m): (i8, i8) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m))
}

/// A local date and time ("2026-11-07 17:00" or "2026-11-07T17:00") in that zone, as Unix seconds.
pub fn local_time(zone: &TimeZone, text: &str) -> Option<i64> {
    let text = text.trim().replacen(' ', "T", 1);
    let dt: jiff::civil::DateTime = text.parse().ok()?;
    Some(dt.to_zoned(zone.clone()).ok()?.timestamp().as_second())
}

/// "Weekdays at 07:30", "Every 4 hours", "Once, Sat 7 Nov 17:00": a schedule as words.
pub fn describe(schedule: &Schedule, zone: &TimeZone) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    match schedule {
        Schedule::Link => "from its link (or Run now)".into(),
        Schedule::Event { event } if event == "training_finished" => "when a training finishes".into(),
        Schedule::Event { .. } => "when a picture, video or 3D model finishes".into(),
        Schedule::Every { hours: 1 } => "every hour".into(),
        Schedule::Every { hours } => format!("every {hours} hours"),
        Schedule::Once { at } => match Timestamp::from_second(*at) {
            Ok(t) => format!("once, {}", t.to_zoned(zone.clone()).strftime("%a %-d %b %H:%M")),
            Err(_) => "once".into(),
        },
        Schedule::Days { time, days } => {
            let mut d = days.clone();
            d.sort_unstable();
            d.dedup();
            let which = match d.as_slice() {
                [0, 1, 2, 3, 4, 5, 6] => "every day".to_string(),
                [1, 2, 3, 4, 5] => "weekdays".to_string(),
                [0, 6] => "weekends".to_string(),
                _ => d.iter().filter_map(|i| DAYS.get(*i as usize)).copied().collect::<Vec<_>>().join(", "),
            };
            format!("{which} at {time}")
        }
    }
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
    /// What it may do while it runs on its own.
    pub allow: Allow,
    /// Where it says its result out loud (when it tells you, outside quiet hours): "phone",
    /// and Home Assistant speakers by entity id ("media_player.kitchen").
    pub say_on: Vec<String>,
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
    allow_tools: i64,
    allow_changes: String,
    allow_commands: i64,
    say_on: String,
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
            allow: Allow {
                tools: self.allow_tools != 0,
                changes: if self.allow_changes == "apply" { Changes::Apply } else { Changes::Wait },
                commands: self.allow_commands != 0,
            },
            say_on: serde_json::from_str(&self.say_on).unwrap_or_default(),
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
                       notify, hook_hash IS NOT NULL AS has_link, allow_tools, allow_changes, allow_commands, say_on, chat_id, next_at, last_at, last_status, last_note, created_at, updated_at";

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
    /// What it may do while it runs on its own.
    pub allow: Option<Allow>,
    /// Where it says its result out loud: "phone" and/or Home Assistant speakers
    /// ("media_player.kitchen").
    pub say_on: Option<Vec<String>>,
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

fn changes_str(c: Changes) -> &'static str {
    match c {
        Changes::Wait => "wait",
        Changes::Apply => "apply",
    }
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
    let allow = input.allow.unwrap_or_default();
    let say_on = clean_say_on(input.say_on.unwrap_or_default())?;
    let zone = zone(input.time_zone.as_deref(), input.utc_offset_minutes);
    let next_at = next_for(&schedule, enabled, &zone, None);
    let t = now();
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO routines (id, account_id, name, persona_id, instructions, schedule, time_zone, utc_offset_minutes, \
         enabled, notify, allow_tools, allow_changes, allow_commands, say_on, next_at, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(allow.tools as i64)
    .bind(changes_str(allow.changes))
    .bind(allow.commands as i64)
    .bind(say_json(&say_on))
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
    let allow = input.allow.unwrap_or(r.allow);
    let say_on = match input.say_on {
        Some(s) => clean_say_on(s)?,
        None => r.say_on.clone(),
    };
    let time_zone = input.time_zone.or(r.time_zone.clone());
    let offset = input.utc_offset_minutes.or(r.utc_offset_minutes);
    let next_at = next_for(&schedule, enabled, &zone(time_zone.as_deref(), offset), r.last_at);
    sqlx::query(
        "UPDATE routines SET name = ?, persona_id = ?, instructions = ?, schedule = ?, time_zone = ?, utc_offset_minutes = ?, \
         enabled = ?, notify = ?, allow_tools = ?, allow_changes = ?, allow_commands = ?, say_on = ?, next_at = ?, updated_at = ? \
         WHERE id = ? AND account_id = ?",
    )
    .bind(&name)
    .bind(&persona_id)
    .bind(&instructions)
    .bind(serde_json::to_string(&schedule).map_err(|e| CoreError::Internal(e.to_string()))?)
    .bind(&time_zone)
    .bind(offset)
    .bind(enabled as i64)
    .bind(notify.as_str())
    .bind(allow.tools as i64)
    .bind(changes_str(allow.changes))
    .bind(allow.commands as i64)
    .bind(say_json(&say_on))
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

/// It was due while a run of it was still going (waiting for a lent PC, say): that time is
/// skipped, not run straight after.
pub async fn skip_due(pool: &SqlitePool, r: &Routine) -> CoreResult<()> {
    let next_at = match &r.schedule {
        Schedule::Once { .. } => None,
        s => next_for(s, r.enabled, &r.zone(), Some(now())),
    };
    sqlx::query("UPDATE routines SET next_at = ? WHERE id = ?").bind(next_at).bind(&r.id).execute(pool).await?;
    Ok(())
}

/// What a run is doing now (waiting for a lent PC...), shown while it runs.
pub async fn note_running(pool: &SqlitePool, routine_id: &str, run_id: &str, note: &str) -> CoreResult<()> {
    sqlx::query("UPDATE routine_runs SET note = ? WHERE id = ? AND status = 'running'").bind(note).bind(run_id).execute(pool).await?;
    sqlx::query("UPDATE routines SET last_note = ? WHERE id = ? AND last_status = 'running'")
        .bind(note)
        .bind(routine_id)
        .execute(pool)
        .await?;
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
    tell_at: i64,
) -> CoreResult<()> {
    let status = if ok { "ok" } else { "failed" };
    sqlx::query("UPDATE routine_runs SET finished_at = ?, status = ?, message_id = ?, note = ?, tell = ?, tell_at = ? WHERE id = ?")
        .bind(now())
        .bind(status)
        .bind(message_id)
        .bind(note)
        .bind(tell as i64)
        .bind(tell.then_some(tell_at))
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

/// One-off routines that ran fine are removed an hour after they've been told (so the list
/// isn't cluttered with old reminders; a failed one stays, to see why). Their chats stay.
pub async fn remove_finished_once(pool: &SqlitePool, at: i64) -> CoreResult<u64> {
    let done = sqlx::query(
        "DELETE FROM routines WHERE json_extract(schedule, '$.kind') = 'once' AND next_at IS NULL AND last_status = 'ok' \
         AND NOT EXISTS (SELECT 1 FROM routine_runs rr WHERE rr.routine_id = routines.id \
                         AND (rr.status = 'running' OR COALESCE(rr.tell_at, rr.finished_at, 0) > ?))",
    )
    .bind(at - 3600)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
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

/// Where a routine may speak: "phone", or a Home Assistant speaker's entity id. At most five,
/// no repeats.
pub fn clean_say_on(list: Vec<String>) -> CoreResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for s in list {
        let s = s.trim().to_string();
        let speaker = s.strip_prefix("media_player.").is_some_and(|rest| {
            !rest.is_empty() && rest.len() <= 100 && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        });
        if s != "phone" && !speaker {
            return Err(CoreError::Invalid(format!("\"{s}\" isn't the phone or a Home Assistant speaker.")));
        }
        if !out.contains(&s) {
            out.push(s);
        }
    }
    if out.len() > 5 {
        return Err(CoreError::Invalid("Pick up to five places to say it.".into()));
    }
    Ok(out)
}

fn say_json(list: &[String]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".into())
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
    /// The routine's `say_on` (JSON).
    pub say_on: String,
    pub persona_id: Option<String>,
    /// Told when it finished (not held over quiet hours).
    pub on_time: bool,
}

/// My routines' runs to tell about now: told since `since`, up to `now` (one that finished in
/// quiet hours is told when they end).
pub async fn told_since(pool: &SqlitePool, account_id: &str, since: i64, now: i64) -> CoreResult<Vec<Told>> {
    Ok(sqlx::query_as(
        "SELECT rr.id, r.name, r.chat_id, rr.status, rr.note, rr.finished_at, r.say_on, r.persona_id, \
         COALESCE(rr.tell_at <= rr.finished_at, 0) AS on_time FROM routine_runs rr \
         JOIN routines r ON r.id = rr.routine_id \
         WHERE r.account_id = ? AND rr.tell = 1 AND rr.tell_at >= ? AND rr.tell_at <= ? ORDER BY rr.tell_at LIMIT 20",
    )
    .bind(account_id)
    .bind(since)
    .bind(now)
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
    // News held back by quiet hours: told when they end.
    let held: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(rr.tell_at) FROM routine_runs rr JOIN routines r ON r.id = rr.routine_id \
         WHERE r.account_id = ? AND rr.tell = 1 AND rr.tell_at > ?",
    )
    .bind(account_id)
    .bind(now())
    .fetch_one(pool)
    .await?;
    let next = match (next, held) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
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

    #[test]
    fn quiet_hours_end_in_the_persons_own_zone() {
        let sydney = zone(Some("Australia/Sydney"), None);
        // Fri 6 Nov 2026, 20:00 in Sydney = 1_793_955_600.
        let fri_8pm = 1_793_955_600;
        let night = Quiet { from: "22:00".into(), to: "07:00".into() };
        assert_eq!(night.ends(&sydney, fri_8pm), None, "not yet quiet");
        // 23:30 Friday: told at 07:00 Saturday.
        assert_eq!(night.ends(&sydney, fri_8pm + 210 * 60), Some(fri_8pm + 11 * 3600));
        // 05:00 Saturday: told at 07:00 the same morning.
        assert_eq!(night.ends(&sydney, fri_8pm + 9 * 3600), Some(fri_8pm + 11 * 3600));
        // In the day, not over midnight.
        let lunch = Quiet { from: "12:00".into(), to: "13:00".into() };
        assert_eq!(lunch.ends(&sydney, fri_8pm), None);
        assert!(Quiet { from: "07:00".into(), to: "07:00".into() }.check().is_err());
        assert!(Quiet { from: "7am".into(), to: "08:00".into() }.check().is_err());
    }

    #[tokio::test]
    async fn allowances_are_kept_and_news_waits_for_quiet_hours_to_end() {
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
                name: Some("Build".into()),
                instructions: Some("Build it.".into()),
                schedule: Some(Schedule::Link),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(r.allow, Allow::default());
        let allow = Allow { tools: false, changes: Changes::Apply, commands: false };
        let r = update(&pool, &me.id, &r.id, RoutineInput { allow: Some(allow), ..Default::default() }).await.unwrap();
        assert_eq!(r.allow, allow);
        // A run told an hour from now isn't told yet.
        let run = start_run(&pool, &r, "now").await.unwrap();
        let t = now();
        finish_run(&pool, &r.id, &run, true, None, Some("Built."), true, t + 3600).await.unwrap();
        assert!(told_since(&pool, &me.id, t - 5, t).await.unwrap().is_empty());
        assert_eq!(told_since(&pool, &me.id, t - 5, t + 3600).await.unwrap().len(), 1);
        let (next, _) = watch_times(&pool, &me.id).await.unwrap();
        assert_eq!(next, Some(t + 3600), "a phone looks when it's due");
        // Quiet hours are kept per person.
        let q = Quiet { from: "22:00".into(), to: "07:00".into() };
        assert_eq!(set_quiet(&pool, &me.id, Some(q.clone())).await.unwrap(), Some(q));
        assert_eq!(set_quiet(&pool, &me.id, None).await.unwrap(), None);
    }

    #[test]
    fn local_times_and_schedules_in_words() {
        let sydney = zone(Some("Australia/Sydney"), None);
        assert_eq!(local_time(&sydney, "2026-11-06 20:00"), Some(1_793_955_600));
        assert_eq!(local_time(&sydney, "2026-11-06T20:00"), Some(1_793_955_600));
        assert_eq!(local_time(&sydney, "tomorrow"), None);
        assert_eq!(describe(&Schedule::Once { at: 1_793_955_600 }, &sydney), "once, Fri 6 Nov 20:00");
        assert_eq!(describe(&Schedule::Days { time: "07:30".into(), days: vec![5, 1, 2, 3, 4] }, &sydney), "weekdays at 07:30");
        assert_eq!(describe(&Schedule::Days { time: "18:00".into(), days: vec![0] }, &sydney), "Sun at 18:00");
        assert_eq!(describe(&Schedule::Every { hours: 6 }, &sydney), "every 6 hours");
    }

    #[tokio::test]
    async fn a_one_off_that_ran_is_cleared_away_after_its_told() {
        let pool = crate::db::open_in_memory().await.unwrap();
        let me = crate::accounts::create(
            &pool,
            crate::accounts::NewAccount { name: "Dan".into(), role: crate::accounts::Role::Owner, secret: Some("4821".into()) },
        )
        .await
        .unwrap();
        let once = |name: &str| RoutineInput {
            name: Some(name.into()),
            instructions: Some("Remind me.".into()),
            schedule: Some(Schedule::Once { at: now() + 600 }),
            ..Default::default()
        };
        let ok = create(&pool, &me.id, once("Stretch")).await.unwrap();
        let failed = create(&pool, &me.id, once("Oven")).await.unwrap();
        let daily = create(
            &pool,
            &me.id,
            RoutineInput { schedule: Some(Schedule::Days { time: "07:00".into(), days: vec![1] }), ..once("Brief") },
        )
        .await
        .unwrap();
        let t = now();
        for (r, worked) in [(&ok, true), (&failed, false), (&daily, true)] {
            let run = start_run(&pool, r, "schedule").await.unwrap();
            finish_run(&pool, &r.id, &run, worked, None, None, true, t).await.unwrap();
        }
        // Not yet: it's only just been told.
        assert_eq!(remove_finished_once(&pool, t).await.unwrap(), 0);
        // An hour on: the reminder that worked goes; the failed one and the daily one stay.
        assert_eq!(remove_finished_once(&pool, t + 3601).await.unwrap(), 1);
        let left: Vec<String> = list(&pool, &me.id).await.unwrap().into_iter().map(|r| r.name).collect();
        assert_eq!(left, vec!["Brief".to_string(), "Oven".to_string()]);
    }

    #[test]
    fn it_speaks_on_the_phone_or_home_assistant_speakers_only() {
        let ok = clean_say_on(vec!["phone".into(), " media_player.kitchen_nest ".into(), "phone".into()]).unwrap();
        assert_eq!(ok, vec!["phone".to_string(), "media_player.kitchen_nest".to_string()]);
        assert!(clean_say_on(vec!["light.kitchen".into()]).is_err());
        assert!(clean_say_on(vec!["media_player.".into()]).is_err());
        assert!(clean_say_on(vec!["media_player.Kitchen; rm".into()]).is_err());
        let many: Vec<String> = (0..6).map(|i| format!("media_player.s{i}")).collect();
        assert!(clean_say_on(many).is_err());
    }
}
