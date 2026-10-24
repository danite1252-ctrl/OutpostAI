//! Everything Outpost is downloading, in one list: chat models from Ollama and
//! Create's model files. The desktop app's Downloads button shows it.

use axum::extract::State;
use axum::Json;
use outpost_engines::packs;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::extract::Auth;
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadKind {
    /// A chat or embedding model, through Ollama.
    Model,
    /// A Create workflow's model files.
    Workflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Download {
    /// Stable while it runs: `model:<name>` or `workflow:<pack id>`.
    pub id: String,
    pub kind: DownloadKind,
    pub name: String,
    /// The file or step now under way.
    pub detail: Option<String>,
    /// File (or part) being fetched, from 1, and how many there are when known.
    pub part: u32,
    pub parts: Option<u32>,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub percent: Option<f32>,
    pub speed_bps: Option<u64>,
    pub eta_secs: Option<u64>,
    pub running: bool,
    pub error: Option<String>,
    /// When it ended (Unix seconds); `None` while running.
    pub finished_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Downloads {
    /// Running first, then the most recently finished.
    pub items: Vec<Download>,
    pub running: usize,
}

/// Downloads running now, and those that finished since Outpost started.
#[utoipa::path(get, path = "/api/downloads", tag = "models", security(("bearer" = [])),
    responses((status = 200, description = "Downloads", body = Downloads)))]
pub async fn list(State(state): State<AppState>, _auth: Auth) -> Json<Downloads> {
    Json(collect(&state))
}

pub(crate) fn collect(state: &AppState) -> Downloads {
    let mut items: Vec<Download> = state
        .models
        .pull_states()
        .into_iter()
        .map(|p| Download {
            id: format!("model:{}", p.model),
            kind: DownloadKind::Model,
            name: p.model.clone(),
            detail: Some(pull_step(&p.status)),
            part: p.part,
            parts: None,
            done_bytes: p.completed,
            total_bytes: p.total,
            percent: p.percent,
            speed_bps: p.speed_bps,
            eta_secs: p.eta_secs,
            running: !p.done,
            error: p.error,
            finished_at: p.finished_at,
        })
        .collect();
    let all = packs::built_in();
    for (id, s) in state.assets.all_setups() {
        let name = all.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.clone());
        items.push(Download {
            id: format!("workflow:{id}"),
            kind: DownloadKind::Workflow,
            name,
            detail: s.current.clone(),
            part: s.file_index,
            parts: Some(s.file_count),
            done_bytes: s.done_bytes,
            total_bytes: s.total_bytes,
            percent: s.percent,
            speed_bps: s.speed_bps,
            eta_secs: s.eta_secs,
            running: s.running,
            error: s.error,
            finished_at: s.finished_at,
        });
    }
    items.sort_by(|a, b| {
        b.running
            .cmp(&a.running)
            .then(b.finished_at.cmp(&a.finished_at))
            .then(a.name.cmp(&b.name))
    });
    let running = items.iter().filter(|d| d.running).count();
    Downloads { items, running }
}

/// Ollama's status line in plain words.
fn pull_step(status: &str) -> String {
    match status {
        "starting" | "pulling manifest" => "Starting".into(),
        "verifying sha256 digest" => "Checking the download".into(),
        "writing manifest" | "removing any unused layers" => "Finishing".into(),
        "success" => "Done".into(),
        "failed" => "Failed".into(),
        s if s.starts_with("pulling ") => "Downloading".into(),
        s => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_steps_read_plainly() {
        assert_eq!(pull_step("pulling 6a0746a1ec1a"), "Downloading");
        assert_eq!(pull_step("verifying sha256 digest"), "Checking the download");
        assert_eq!(pull_step("success"), "Done");
    }
}
