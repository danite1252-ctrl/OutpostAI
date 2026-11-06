//! Running routines: every half minute, the ones that are due start, each as a message in its
//! own chat answered by its persona (with the persona's tools), as if the person had sent it.
//! A run is recorded (its reply, or why it failed); a routine never runs twice at once.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::Role;
use outpost_core::routines::{self, Notify, Routine, RoutineInput, Run};
use outpost_core::sessions::{Session, SessionKind};
use serde::Deserialize;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

/// What the persona answers when there's nothing worth telling (routines that only tell you
/// when there's something new).
const NOTHING_NEW: &str = "NOTHING_NEW";

/// Longest a run may take before it's stopped.
const RUN_MAX: Duration = Duration::from_secs(30 * 60);

/// Routines running now.
static RUNNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Check for due routines, for as long as Outpost runs.
pub async fn scheduler(state: AppState) {
    let _ = routines::fail_interrupted(&state.pool).await;
    // A moment for everything else to start first.
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        match routines::due(&state.pool, outpost_core::now()).await {
            Ok(due) => {
                for r in due {
                    tokio::spawn(run(state.clone(), r, "schedule"));
                }
            }
            Err(e) => tracing::warn!(error = %e, "couldn't check routines"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Run a routine now (on schedule, or Run now).
pub async fn run(state: AppState, r: Routine, reason: &'static str) {
    if !RUNNING.lock().unwrap_or_else(|p| p.into_inner()).insert(r.id.clone()) {
        return; // already running
    }
    let run_id = match routines::start_run(&state.pool, &r, reason).await {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(error = %e, routine = %r.id, "couldn't start a routine");
            RUNNING.lock().unwrap_or_else(|p| p.into_inner()).remove(&r.id);
            return;
        }
    };
    tracing::info!(routine = %r.id, name = %r.name, reason, "routine running");
    let result = attempt(&state, &r).await;
    let (ok, news, message_id, note) = match result {
        Ok(done) => {
            // Only told when there's news: the reply said "nothing new", or a quick separate
            // check of the result says so.
            let quiet = r.notify == Notify::Worth
                && (done.text.contains(NOTHING_NEW)
                    || match &done.model {
                        Some(model) => crate::llm::judge_news(&state, model, &r.instructions, &done.text).await == Some(false),
                        None => false,
                    });
            let note = if quiet { "Nothing new this time.".to_string() } else { done.gist };
            (true, !quiet, Some(done.id), Some(note.trim().to_string()))
        }
        Err(why) => (false, false, None, Some(why)),
    };
    let tell = r.notify.tells(ok, news);
    if let Err(e) = routines::finish_run(&state.pool, &r.id, &run_id, ok, message_id.as_deref(), note.as_deref(), tell).await {
        tracing::warn!(error = %e, "couldn't record a routine's run");
    }
    RUNNING.lock().unwrap_or_else(|p| p.into_inner()).remove(&r.id);
}

/// What a run said.
struct Done {
    /// Its reply.
    id: String,
    text: String,
    /// The start of it.
    gist: String,
    model: Option<String>,
}

/// One run: the routine's instructions sent in its chat, and its reply waited for. Returns
/// what it said, or why it failed.
async fn attempt(state: &AppState, r: &Routine) -> Result<Done, String> {
    let account = outpost_core::accounts::get(&state.pool, &r.account_id).await.map_err(|e| e.to_string())?;
    if !matches!(account.role, Role::Owner | Role::Member) {
        return Err("This account can't run routines.".into());
    }
    // Its own chat (made the first time, or again if it was deleted).
    let chat = match &r.chat_id {
        Some(id) => outpost_core::chats::get(&state.pool, &account.id, id).await.ok(),
        None => None,
    };
    let chat = match chat {
        Some(c) => c,
        None => {
            let c = outpost_core::chats::create(&state.pool, &account.id, r.persona_id.as_deref(), Some(&r.name))
                .await
                .map_err(|e| format!("Couldn't make its chat: {e}"))?;
            routines::set_chat(&state.pool, &r.id, &c.id).await.map_err(|e| e.to_string())?;
            c
        }
    };
    let only_news = if r.notify == Notify::Worth {
        format!(
            " The user is only told when there's something new or worth knowing: if there's nothing (no change, \
             nothing found), reply with just {NOTHING_NEW} and nothing else."
        )
    } else {
        String::new()
    };
    let content = format!(
        "[Routine \"{}\": this runs on its own, on a schedule, and nobody is watching as it runs. Do the task below \
         and give the result. Don't ask questions: if something's missing, do what you can and say what you couldn't.{only_news}]\n\n{}",
        r.name, r.instructions
    );
    let auth = Auth {
        session: Session {
            id: format!("routine-{}", r.id),
            account_id: account.id.clone(),
            kind: SessionKind::App,
            label: format!("Routine: {}", r.name),
            created_at: outpost_core::now(),
            last_used_at: outpost_core::now(),
        },
        account,
    };
    let req = crate::routes::chat::SendRequest {
        content,
        utc_offset_minutes: r.utc_offset_minutes,
        time_zone: r.time_zone.clone(),
        // The chat's tools on your PC, if one with Outpost open offers them.
        use_pc_tools: true,
        ..Default::default()
    };
    let started = outpost_core::now();
    crate::live::take_failure(&chat.id);
    let mut rx = crate::routes::chat::start_reply(state, &auth, &chat.id, req).await.map_err(|e| e.message)?;
    let finished = tokio::time::timeout(RUN_MAX, async { while rx.recv().await.is_some() {} }).await;
    if finished.is_err() {
        crate::workspace::request_stop(&chat.id);
        return Err(format!("It took longer than {} minutes, so it was stopped.", RUN_MAX.as_secs() / 60));
    }
    let messages = outpost_core::chats::messages(&state.pool, &chat.id).await.map_err(|e| e.to_string())?;
    match messages.iter().rev().find(|m| m.role == "assistant" && m.created_at >= started) {
        Some(m) if !m.content.trim().is_empty() => Ok(Done {
            id: m.id.clone(),
            gist: gist(&m.content),
            text: m.content.clone(),
            model: m.model.clone(),
        }),
        _ => Err(match crate::live::take_failure(&chat.id) {
            Some(why) => format!("The reply failed: {why}"),
            None => "It didn't give a reply (open the routine's chat to see).".into(),
        }),
    }
}

/// What a reply came to, for the list and a notification: its last paragraphs (where the
/// answer is, after any "let me check…"), about 240 characters.
fn gist(reply: &str) -> String {
    let paragraphs: Vec<&str> = reply
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != NOTHING_NEW)
        .collect();
    // The last paragraph (the answer); the one before too only when that's very short.
    let mut kept: Vec<&str> = Vec::new();
    let mut len = 0;
    for p in paragraphs.iter().rev() {
        if !kept.is_empty() && (len >= 40 || len + p.chars().count() > 240) {
            break;
        }
        len += p.chars().count();
        kept.push(p);
    }
    kept.reverse();
    let text = kept.join(" ").replace(NOTHING_NEW, "");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 240 {
        let cut: String = text.chars().take(239).collect();
        format!("{}…", cut.trim_end())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_gist_is_where_the_answer_is() {
        let reply = "Let me check for Paper builds.\n\nLet me look at the downloads page.\n\nThe latest Paper 26.3 build is #245, out today.";
        assert_eq!(super::gist(reply), "The latest Paper 26.3 build is #245, out today.");
        assert_eq!(super::gist("Checked the page.\n\nYes: 245."), "Checked the page. Yes: 245.");
        assert_eq!(super::gist("Short answer.\n\nNOTHING_NEW"), "Short answer.");
        let long = "x".repeat(500);
        assert!(super::gist(&long).chars().count() <= 240);
    }
}

fn may_use(auth: &Auth) -> ApiResult<()> {
    match auth.account.role {
        Role::Owner | Role::Member => Ok(()),
        _ => Err(ApiError::forbidden("Routines are for owners and members.")),
    }
}

/// My routines.
#[utoipa::path(get, path = "/api/routines", tag = "routines", security(("bearer" = [])),
    responses((status = 200, description = "Routines", body = [Routine])))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<Routine>>> {
    may_use(&auth)?;
    Ok(Json(routines::list(&state.pool, &auth.account.id).await?))
}

/// Make a routine.
#[utoipa::path(post, path = "/api/routines", tag = "routines", security(("bearer" = [])), request_body = RoutineInput,
    responses((status = 201, description = "Made", body = Routine), (status = 400, description = "Something's missing", body = ErrorBody)))]
pub async fn create(State(state): State<AppState>, auth: Auth, Json(input): Json<RoutineInput>) -> ApiResult<(StatusCode, Json<Routine>)> {
    may_use(&auth)?;
    Ok((StatusCode::CREATED, Json(routines::create(&state.pool, &auth.account.id, input).await?)))
}

/// Change a routine (pause it with `enabled: false`).
#[utoipa::path(put, path = "/api/routines/{id}", tag = "routines", security(("bearer" = [])), request_body = RoutineInput,
    params(("id" = String, Path, description = "Routine id")),
    responses((status = 200, description = "Changed", body = Routine), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn update(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Json(input): Json<RoutineInput>) -> ApiResult<Json<Routine>> {
    may_use(&auth)?;
    Ok(Json(routines::update(&state.pool, &auth.account.id, &id, input).await?))
}

/// Delete a routine (its chat stays).
#[utoipa::path(delete, path = "/api/routines/{id}", tag = "routines", security(("bearer" = [])),
    params(("id" = String, Path, description = "Routine id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not found", body = ErrorBody)))]
pub async fn delete(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    may_use(&auth)?;
    routines::delete(&state.pool, &auth.account.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Run a routine now (its schedule carries on as before).
#[utoipa::path(post, path = "/api/routines/{id}/run", tag = "routines", security(("bearer" = [])),
    params(("id" = String, Path, description = "Routine id")),
    responses((status = 202, description = "Started", body = Routine), (status = 409, description = "Already running", body = ErrorBody)))]
pub async fn run_now(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Routine>)> {
    may_use(&auth)?;
    let r = routines::get(&state.pool, &auth.account.id, &id).await?;
    if RUNNING.lock().unwrap_or_else(|p| p.into_inner()).contains(&r.id) {
        return Err(ApiError::conflict("It's running now."));
    }
    tokio::spawn(run(state.clone(), r, "now"));
    // A moment for it to be marked running.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok((StatusCode::ACCEPTED, Json(routines::get(&state.pool, &auth.account.id, &id).await?)))
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

/// A routine's latest runs, newest first.
#[utoipa::path(get, path = "/api/routines/{id}/runs", tag = "routines", security(("bearer" = [])),
    params(("id" = String, Path, description = "Routine id"), ("limit" = Option<i64>, Query, description = "How many (default 20)")),
    responses((status = 200, description = "Runs", body = [Run])))]
pub async fn runs(State(state): State<AppState>, auth: Auth, Path(id): Path<String>, Query(q): Query<RunsQuery>) -> ApiResult<Json<Vec<Run>>> {
    may_use(&auth)?;
    Ok(Json(routines::runs(&state.pool, &auth.account.id, &id, q.limit.unwrap_or(20)).await?))
}
