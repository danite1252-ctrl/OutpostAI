//! Ready-made personas (Assistant, Coder, Researcher, a few fun ones...): their instructions,
//! settings and the library tools that suit them. Adding one makes it yours, to change as
//! you like; it gets those tools when you have them.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use outpost_core::accounts::Role;
use outpost_core::personas::{self, Persona, PersonaInput};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::extract::Auth;
use crate::AppState;

const TEMPLATES: &str = include_str!("../persona-templates.json");

/// One ready-made persona.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Template {
    pub id: String,
    pub name: String,
    /// "Everyday", "Work" or "Fun".
    pub group: String,
    /// One line about it.
    pub blurb: String,
    pub avatar_color: String,
    pub temperature: Option<f64>,
    /// Tool library ids it works best with (added when you have them).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Its instructions (`{name}` becomes whoever's chatting).
    pub prompt: String,
}

pub fn all() -> Vec<Template> {
    serde_json::from_str(TEMPLATES).unwrap_or_default()
}

/// The ready-made personas.
#[utoipa::path(get, path = "/api/personas/templates", tag = "chat", security(("bearer" = [])),
    responses((status = 200, description = "Templates", body = [Template])))]
pub async fn list(_auth: Auth) -> Json<Vec<Template>> {
    Json(all())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Added {
    pub persona: Persona,
    /// The tools it was given (names), from the ones you can use.
    pub tools: Vec<String>,
}

/// Add a ready-made persona to your personas, with the suggested tools you have.
#[utoipa::path(post, path = "/api/personas/templates/{id}", tag = "chat", security(("bearer" = [])),
    params(("id" = String, Path, description = "Template id")),
    responses((status = 201, description = "Added", body = Added), (status = 404, description = "No such template", body = ErrorBody)))]
pub async fn add(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<(StatusCode, Json<Added>)> {
    let t = all()
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "There's no such ready-made persona."))?;
    let persona = personas::create(
        &state.pool,
        &auth.account.id,
        PersonaInput {
            name: Some(t.name.clone()),
            system_prompt: Some(t.prompt.clone()),
            temperature: t.temperature,
            avatar_color: Some(t.avatar_color.clone()),
            ..Default::default()
        },
    )
    .await?;
    let restricted = auth.account.role == Role::Restricted;
    let have = outpost_core::tools::usable(&state.pool, &auth.account.id, restricted).await?;
    let chosen: Vec<_> = have
        .iter()
        .filter(|tool| tool.enabled && tool.catalog_id.as_deref().is_some_and(|c| t.tools.iter().any(|x| x == c)))
        .collect();
    let ids: Vec<String> = chosen.iter().map(|tool| tool.id.clone()).collect();
    if !ids.is_empty() {
        outpost_core::tools::set_persona_tools(&state.pool, &auth.account.id, restricted, &persona.id, &ids).await?;
    }
    Ok((StatusCode::CREATED, Json(Added { persona, tools: chosen.iter().map(|tool| tool.name.clone()).collect() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_read_and_are_complete() {
        let t = all();
        assert!(t.len() >= 10);
        for p in &t {
            assert!(!p.prompt.trim().is_empty() && p.prompt.contains("{name}"), "{}", p.id);
            assert!(p.avatar_color.len() == 7 && p.avatar_color.starts_with('#'), "{}", p.id);
            assert!(["Everyday", "Work", "Fun"].contains(&p.group.as_str()), "{}", p.id);
        }
        let mut ids: Vec<_> = t.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), t.len());
    }
}
