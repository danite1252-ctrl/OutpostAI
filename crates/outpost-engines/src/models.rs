//! Chat and embedding models in Ollama: what's installed (with Ollama's own
//! capability list), downloads with progress, and removal.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::catalog::Badge;
use crate::types::EngineId;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstalledModel {
    /// Name to use as `model`, e.g. `huihui_ai/qwen3.5-abliterated:9b`.
    pub model: String,
    pub size_bytes: u64,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub family: Option<String>,
    /// From Ollama's own capability list for this model.
    pub badges: Vec<Badge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PullState {
    pub model: String,
    /// Ollama's status line, e.g. "pulling manifest", "downloading", "success".
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub percent: Option<f32>,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum ModelError {
    /// Ollama isn't running.
    Offline,
    /// That model is already downloading.
    Busy,
    Invalid(String),
    Ollama(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Offline => f.write_str("Ollama isn't running. Start it in Settings → Engines."),
            ModelError::Busy => f.write_str("That model is already downloading."),
            ModelError::Invalid(m) | ModelError::Ollama(m) => f.write_str(m),
        }
    }
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: String,
    #[serde(default)]
    details: TagDetails,
}

#[derive(Deserialize, Default)]
struct TagDetails {
    parameter_size: Option<String>,
    quantization_level: Option<String>,
    family: Option<String>,
}

#[derive(Deserialize)]
struct Show {
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Deserialize)]
struct PullLine {
    #[serde(default)]
    status: String,
    total: Option<u64>,
    completed: Option<u64>,
    error: Option<String>,
}

pub struct ModelStore {
    base: String,
    /// Short timeout for listing and showing.
    quick: reqwest::Client,
    /// No overall timeout: pulls run for many minutes.
    long: reqwest::Client,
    pulls: Mutex<HashMap<String, PullState>>,
    /// Capabilities by model digest, so we ask Ollama once per model version.
    caps: Mutex<HashMap<String, Vec<Badge>>>,
}

/// Model names Ollama accepts: `name`, `name:tag`, `user/name:tag`.
pub fn valid_name(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/'))
        && !model.contains("..")
        && !model.starts_with('/')
}

fn badges_from(capabilities: &[String]) -> Vec<Badge> {
    capabilities
        .iter()
        .filter_map(|c| match c.as_str() {
            "vision" => Some(Badge::Vision),
            "tools" => Some(Badge::Tools),
            "thinking" => Some(Badge::Thinking),
            "embedding" => Some(Badge::Embedding),
            _ => None,
        })
        .collect()
}

impl ModelStore {
    pub fn new() -> Arc<Self> {
        Arc::new(ModelStore {
            base: EngineId::Ollama.base_url(),
            quick: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client"),
            long: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .expect("HTTP client"),
            pulls: Mutex::new(HashMap::new()),
            caps: Mutex::new(HashMap::new()),
        })
    }

    fn pulls(&self) -> std::sync::MutexGuard<'_, HashMap<String, PullState>> {
        self.pulls.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Installed models, or `Offline` when Ollama isn't answering.
    pub async fn installed(&self) -> Result<Vec<InstalledModel>, ModelError> {
        let tags: Tags = self
            .quick
            .get(format!("{}/api/tags", self.base))
            .send()
            .await
            .map_err(|_| ModelError::Offline)?
            .json()
            .await
            .map_err(|e| ModelError::Ollama(e.to_string()))?;
        let mut out = Vec::with_capacity(tags.models.len());
        for m in tags.models {
            let badges = self.capabilities(&m.name, &m.digest).await;
            out.push(InstalledModel {
                model: m.name,
                size_bytes: m.size,
                parameter_size: m.details.parameter_size,
                quantization: m.details.quantization_level,
                family: m.details.family,
                badges,
            });
        }
        out.sort_by(|a, b| a.model.cmp(&b.model));
        Ok(out)
    }

    async fn capabilities(&self, model: &str, digest: &str) -> Vec<Badge> {
        if let Some(b) = self.caps.lock().ok().and_then(|c| c.get(digest).cloned()) {
            return b;
        }
        let resp = self
            .quick
            .post(format!("{}/api/show", self.base))
            .json(&serde_json::json!({ "model": model }))
            .send()
            .await;
        let badges = match resp {
            Ok(r) => r
                .json::<Show>()
                .await
                .map(|s| badges_from(&s.capabilities))
                .unwrap_or_default(),
            Err(_) => return Vec::new(),
        };
        if let Ok(mut c) = self.caps.lock() {
            c.insert(digest.to_string(), badges.clone());
        }
        badges
    }

    /// Downloads in progress and ones that finished or failed since Outpost started.
    pub fn pull_states(&self) -> Vec<PullState> {
        let mut v: Vec<_> = self.pulls().values().cloned().collect();
        v.sort_by(|a, b| a.model.cmp(&b.model));
        v
    }

    /// Start downloading a model in the background.
    pub fn pull(self: &Arc<Self>, model: &str) -> Result<(), ModelError> {
        if !valid_name(model) {
            return Err(ModelError::Invalid(format!("\"{model}\" is not a valid model name.")));
        }
        {
            let mut pulls = self.pulls();
            if pulls.get(model).is_some_and(|p| !p.done) {
                return Err(ModelError::Busy);
            }
            pulls.insert(
                model.to_string(),
                PullState {
                    model: model.to_string(),
                    status: "starting".into(),
                    completed: 0,
                    total: 0,
                    percent: None,
                    done: false,
                    error: None,
                },
            );
        }
        let me = self.clone();
        let model = model.to_string();
        tokio::spawn(async move {
            let result = me.run_pull(&model).await;
            let mut pulls = me.pulls();
            if let Some(p) = pulls.get_mut(&model) {
                p.done = true;
                if let Err(e) = result {
                    p.error = Some(e);
                    p.status = "failed".into();
                } else {
                    p.status = "success".into();
                    p.percent = Some(100.0);
                }
            }
        });
        Ok(())
    }

    async fn run_pull(&self, model: &str) -> Result<(), String> {
        let resp = self
            .long
            .post(format!("{}/api/pull", self.base))
            .json(&serde_json::json!({ "model": model, "stream": true }))
            .send()
            .await
            .map_err(|_| ModelError::Offline.to_string())?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama refused the download: {text}"));
        }
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let Ok(update) = serde_json::from_slice::<PullLine>(&line) else {
                    continue;
                };
                if let Some(err) = update.error {
                    return Err(err);
                }
                let mut pulls = self.pulls();
                if let Some(p) = pulls.get_mut(model) {
                    if !update.status.is_empty() {
                        p.status = update.status;
                    }
                    if let (Some(total), Some(done)) = (update.total, update.completed) {
                        if total > 0 {
                            p.total = total;
                            p.completed = done;
                            p.percent = Some((done as f32 / total as f32 * 100.0).min(100.0));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Remove an installed model from Ollama.
    pub async fn remove(&self, model: &str) -> Result<(), ModelError> {
        if !valid_name(model) {
            return Err(ModelError::Invalid(format!("\"{model}\" is not a valid model name.")));
        }
        let resp = self
            .quick
            .delete(format!("{}/api/delete", self.base))
            .json(&serde_json::json!({ "model": model }))
            .send()
            .await
            .map_err(|_| ModelError::Offline)?;
        if resp.status().is_success() {
            self.pulls().remove(model);
            Ok(())
        } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
            Err(ModelError::Invalid(format!("\"{model}\" is not installed.")))
        } else {
            Err(ModelError::Ollama(resp.text().await.unwrap_or_default()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names() {
        assert!(valid_name("qwen3.5:9b"));
        assert!(valid_name("huihui_ai/qwen3.5-abliterated:9b"));
        assert!(!valid_name(""));
        assert!(!valid_name("../etc/passwd"));
        assert!(!valid_name("qwen; rm -rf /"));
        assert!(!valid_name("/abs"));
    }

    #[test]
    fn capabilities_become_badges() {
        let caps = vec![
            "completion".to_string(),
            "vision".into(),
            "tools".into(),
            "thinking".into(),
        ];
        assert_eq!(badges_from(&caps), vec![Badge::Vision, Badge::Tools, Badge::Thinking]);
    }

    #[tokio::test]
    async fn offline_ollama_is_reported() {
        // Nothing listens on the Ollama port in CI; skip if something does here.
        let store = ModelStore::new();
        if let Err(e) = store.installed().await {
            assert!(matches!(e, ModelError::Offline));
        }
    }
}
