//! The activity log's route: every tool call a persona made. Owners see everyone's; people
//! see their own.

use axum::extract::{Query, State};
use axum::Json;
use outpost_core::accounts::Role;
use outpost_core::activity::{self, Entry, Filter};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::error::{ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ActivityQuery {
    /// Only this person's (owners; others only ever see their own).
    pub person: Option<String>,
    /// Calls before this time (Unix seconds), to page back.
    pub before: Option<i64>,
    /// Only this tool ("Home Assistant").
    pub server: Option<String>,
    /// How many (up to 200; 50 by default).
    pub limit: Option<i64>,
}

/// Tool calls, newest first: who, from which chat or routine, the action as the model sent it,
/// and what came of it.
#[utoipa::path(get, path = "/api/activity", tag = "activity", security(("bearer" = [])), params(ActivityQuery),
    responses((status = 200, description = "Calls, newest first", body = [Entry]), (status = 401, description = "Not signed in", body = ErrorBody)))]
pub async fn list(State(state): State<AppState>, auth: Auth, Query(q): Query<ActivityQuery>) -> ApiResult<Json<Vec<Entry>>> {
    let everyone = auth.account.role == Role::Owner;
    let f = Filter { person: q.person, before: q.before, server: q.server, limit: q.limit.unwrap_or(50) };
    Ok(Json(activity::list(&state.pool, &auth.account.id, everyone, &f).await?))
}

/// Someone's tools-off hours: tools that do things don't work then (look-ups still do).
#[derive(Debug, serde::Serialize, Deserialize, utoipa::ToSchema)]
pub struct ToolHours {
    /// None: tools work any time.
    pub off: Option<outpost_core::routines::Quiet>,
}

/// Someone's tools-off hours (owners, or your own).
#[utoipa::path(get, path = "/api/accounts/{id}/tool-hours", tag = "activity", security(("bearer" = [])),
    params(("id" = String, Path, description = "The person")),
    responses((status = 200, description = "Their hours", body = ToolHours), (status = 403, description = "Not yours to see", body = ErrorBody)))]
pub async fn get_hours(State(state): State<AppState>, auth: Auth, axum::extract::Path(id): axum::extract::Path<String>) -> ApiResult<Json<ToolHours>> {
    if auth.account.role != Role::Owner && auth.account.id != id {
        return Err(crate::error::ApiError::forbidden("Only an owner can see someone else's tool hours."));
    }
    Ok(Json(ToolHours { off: activity::tools_off(&state.pool, &id).await? }))
}

/// Set someone's tools-off hours (owners; `off: null` takes them away).
#[utoipa::path(put, path = "/api/accounts/{id}/tool-hours", tag = "activity", security(("bearer" = [])),
    params(("id" = String, Path, description = "The person")), request_body = ToolHours,
    responses((status = 200, description = "Set", body = ToolHours), (status = 403, description = "Owners only", body = ErrorBody)))]
pub async fn set_hours(
    State(state): State<AppState>,
    auth: Auth,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(input): Json<ToolHours>,
) -> ApiResult<Json<ToolHours>> {
    if auth.account.role != Role::Owner {
        return Err(crate::error::ApiError::forbidden("Only an owner can set tool hours."));
    }
    auth.require_person("Setting tool hours")?;
    Ok(Json(ToolHours { off: activity::set_tools_off(&state.pool, &id, input.off).await? }))
}
