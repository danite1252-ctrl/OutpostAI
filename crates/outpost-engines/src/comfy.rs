//! A small client for ComfyUI's own API on 127.0.0.1:8188.
//!
//! Jobs are queued with `POST /prompt`. Progress comes over ComfyUI's websocket
//! (step counts while sampling); completion and results come from
//! `GET /history/{id}`, which is polled too, so a dropped websocket never loses a job.

use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use crate::types::EngineId;

#[derive(Debug, thiserror::Error)]
pub enum ComfyError {
    #[error("ComfyUI isn't running. Start it in Settings > Engines.")]
    Offline,
    /// ComfyUI refused the graph (missing model, bad value...). Shown as is.
    #[error("{0}")]
    Rejected(String),
    #[error("{0}")]
    Failed(String),
    #[error("Stopped.")]
    Interrupted,
}

/// One file a finished job produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputFile {
    pub filename: String,
    #[serde(default)]
    pub subfolder: String,
    #[serde(rename = "type", default = "output_type")]
    pub kind: String,
}

fn output_type() -> String {
    "output".into()
}

/// Where a running job is up to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct JobProgress {
    /// "Loading models", "Generating", "Decoding", "Saving".
    pub phase: String,
    pub step: u32,
    pub steps: u32,
    pub percent: Option<f32>,
}

/// What the running (or last seen) ComfyUI can do.
#[derive(Debug, Clone, Default)]
pub struct ComfyNodes {
    /// From `/system_stats`, e.g. `0.42.5`; `None` on old builds that don't say.
    pub version: Option<String>,
    /// Every node type it has, from `/object_info`.
    pub nodes: std::collections::HashSet<String>,
}

pub struct Comfy {
    base: String,
    /// The last node list read, kept while ComfyUI is stopped. Read again when the
    /// version changes or after a while (custom nodes can be added).
    nodes: std::sync::Mutex<Option<(ComfyNodes, std::time::Instant)>>,
    http: reqwest::Client,
    /// Long requests (uploads, downloads of results).
    big: reqwest::Client,
    client_id: String,
}

impl Comfy {
    pub fn new() -> Self {
        Self::at(&EngineId::Comfyui.base_url())
    }

    pub fn at(base: &str) -> Self {
        Comfy {
            base: base.trim_end_matches('/').to_string(),
            nodes: std::sync::Mutex::new(None),
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("HTTP client"),
            big: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(600))
                .build()
                .expect("HTTP client"),
            client_id: format!("outpost-{}", std::process::id()),
        }
    }

    /// ComfyUI's version and node types: read from ComfyUI when it's up, else the
    /// last ones seen. `None` if it has never been seen running.
    pub async fn node_types(&self) -> Option<ComfyNodes> {
        let cached = self.nodes.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let stats = self
            .http
            .get(format!("{}/system_stats", self.base))
            .send()
            .await
            .ok()
            .filter(|r| r.status().is_success());
        let Some(stats) = stats else {
            return cached.map(|(n, _)| n);
        };
        let version = stats
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| v.pointer("/system/comfyui_version").and_then(Value::as_str).map(str::to_string));
        if let Some((n, at)) = &cached {
            if n.version == version && at.elapsed() < Duration::from_secs(300) {
                return Some(n.clone());
            }
        }
        let info = self.big.get(format!("{}/object_info", self.base)).send().await.ok()?;
        let body: Value = info.json().await.ok()?;
        let nodes = body.as_object()?.keys().cloned().collect();
        let fresh = ComfyNodes { version, nodes };
        *self.nodes.lock().unwrap_or_else(|p| p.into_inner()) = Some((fresh.clone(), std::time::Instant::now()));
        Some(fresh)
    }

    pub async fn is_up(&self) -> bool {
        self.http
            .get(format!("{}/system_stats", self.base))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// Queue a graph. Returns ComfyUI's prompt id.
    pub async fn submit(&self, graph: &Value) -> Result<String, ComfyError> {
        let resp = self
            .http
            .post(format!("{}/prompt", self.base))
            .json(&json!({ "prompt": graph, "client_id": self.client_id }))
            .send()
            .await
            .map_err(|_| ComfyError::Offline)?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(ComfyError::Rejected(explain_rejection(&body)));
        }
        body.get("prompt_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ComfyError::Rejected("ComfyUI didn't accept the job.".into()))
    }

    /// The job's result once it has finished; `None` while it is queued or running.
    pub async fn finished(
        &self,
        prompt_id: &str,
    ) -> Result<Option<Result<Vec<(String, OutputFile)>, ComfyError>>, ComfyError> {
        let resp = self
            .http
            .get(format!("{}/history/{prompt_id}", self.base))
            .send()
            .await
            .map_err(|_| ComfyError::Offline)?;
        let body: Value = resp.json().await.map_err(|_| ComfyError::Offline)?;
        Ok(read_history(&body, prompt_id))
    }

    /// Wait for a queued job, reporting progress, and return its output files
    /// (with the node each came from).
    /// `cancelled` is checked on every tick: once true, the job is stopped (again and
    /// again if needed, since ComfyUI ignores an interrupt sent before a job starts).
    /// Gives up after `limit`, and if ComfyUI loses track of the job.
    pub async fn wait(
        &self,
        prompt_id: &str,
        limit: Duration,
        cancelled: impl Fn() -> bool,
        mut on_progress: impl FnMut(JobProgress),
    ) -> Result<Vec<(String, OutputFile)>, ComfyError> {
        let ws_url = format!("{}/ws?clientId={}", self.base.replacen("http", "ws", 1), self.client_id);
        let mut ws = tokio_tungstenite::connect_async(ws_url.as_str())
            .await
            .ok()
            .map(|(s, _)| s);
        let mut progress = JobProgress {
            phase: "Waiting for the GPU".into(),
            ..Default::default()
        };
        on_progress(progress.clone());
        let mut tick = tokio::time::interval(Duration::from_millis(1500));
        let mut offline_for = 0u32;
        let mut ticks = 0u32;
        let mut missing_for = 0u32;
        let started = std::time::Instant::now();
        loop {
            tokio::select! {
                msg = async { match ws.as_mut() { Some(s) => s.next().await, None => std::future::pending().await } } => {
                    match msg {
                        Some(Ok(m)) => {
                            if let Ok(text) = m.to_text() {
                                if let Some(update) = read_ws(text, prompt_id, &progress) {
                                    progress = update;
                                    on_progress(progress.clone());
                                }
                            }
                        }
                        // Websocket gone: carry on with polling alone.
                        _ => ws = None,
                    }
                }
                _ = tick.tick() => {
                    ticks += 1;
                    if cancelled() {
                        self.dequeue(prompt_id).await;
                        self.interrupt(prompt_id).await;
                    }
                    match self.finished(prompt_id).await {
                        Ok(Some(result)) => return result,
                        Ok(None) => offline_for = 0,
                        Err(e) => {
                            offline_for += 1;
                            // ComfyUI stopped or crashed mid-job.
                            if offline_for >= 8 {
                                return Err(e);
                            }
                            continue;
                        }
                    }
                    // Every few seconds: is the job still known to ComfyUI at all?
                    // (Dropped from the queue, or ComfyUI restarted and forgot it.)
                    if ticks % 4 == 0 {
                        match self.in_queue(prompt_id).await {
                            Ok(true) => missing_for = 0,
                            Ok(false) => {
                                missing_for += 1;
                                // Twice in a row, in case it finished between the two checks.
                                if missing_for >= 2 && matches!(self.finished(prompt_id).await, Ok(None)) {
                                    return Err(if cancelled() {
                                        ComfyError::Interrupted
                                    } else {
                                        ComfyError::Failed("ComfyUI lost this job (did it restart?). Try again.".into())
                                    });
                                }
                            }
                            Err(_) => {}
                        }
                    }
                    if started.elapsed() > limit {
                        self.interrupt(prompt_id).await;
                        return Err(ComfyError::Failed(format!(
                            "Gave up after {} minutes.",
                            limit.as_secs() / 60
                        )));
                    }
                }
            }
        }
    }

    /// Download a result file.
    pub async fn fetch(&self, f: &OutputFile) -> Result<Vec<u8>, ComfyError> {
        let resp = self
            .big
            .get(format!("{}/view", self.base))
            .query(&[
                ("filename", f.filename.as_str()),
                ("subfolder", f.subfolder.as_str()),
                ("type", f.kind.as_str()),
            ])
            .send()
            .await
            .map_err(|_| ComfyError::Offline)?;
        if !resp.status().is_success() {
            return Err(ComfyError::Failed(format!("ComfyUI couldn't return {}.", f.filename)));
        }
        Ok(resp.bytes().await.map_err(|_| ComfyError::Offline)?.to_vec())
    }

    /// Upload an input image; returns the name to put in the graph.
    pub async fn upload_image(&self, bytes: Vec<u8>, filename: &str) -> Result<String, ComfyError> {
        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.to_string());
        let form = reqwest::multipart::Form::new()
            .part("image", part)
            .text("type", "input")
            .text("subfolder", "outpost")
            .text("overwrite", "true");
        let resp = self
            .big
            .post(format!("{}/upload/image", self.base))
            .multipart(form)
            .send()
            .await
            .map_err(|_| ComfyError::Offline)?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ComfyError::Failed(format!(
                "ComfyUI refused the picture: {}",
                text.trim()
            )));
        }
        let body: Value = resp.json().await.map_err(|_| ComfyError::Offline)?;
        let name = body.get("name").and_then(Value::as_str).unwrap_or(filename);
        Ok(
            match body.get("subfolder").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                Some(sub) => format!("{sub}/{name}"),
                None => name.to_string(),
            },
        )
    }

    /// Stop whatever is running now.
    /// Stop this job if it's the one running (ComfyUI ignores the id of any other).
    pub async fn interrupt(&self, prompt_id: &str) {
        let _ = self
            .http
            .post(format!("{}/interrupt", self.base))
            .json(&json!({ "prompt_id": prompt_id }))
            .send()
            .await;
    }

    /// Is the job running or waiting in ComfyUI's own queue?
    pub async fn in_queue(&self, prompt_id: &str) -> Result<bool, ComfyError> {
        let body: Value = self
            .http
            .get(format!("{}/queue", self.base))
            .send()
            .await
            .map_err(|_| ComfyError::Offline)?
            .json()
            .await
            .map_err(|_| ComfyError::Offline)?;
        Ok(queue_has(&body, prompt_id))
    }

    /// Remove a job that hasn't started yet.
    pub async fn dequeue(&self, prompt_id: &str) {
        let _ = self
            .http
            .post(format!("{}/queue", self.base))
            .json(&json!({ "delete": [prompt_id] }))
            .send()
            .await;
    }

    /// Unload ComfyUI's models and free video memory (before chat needs it back).
    pub async fn free(&self) {
        let _ = self
            .http
            .post(format!("{}/free", self.base))
            .json(&json!({ "unload_models": true, "free_memory": true }))
            .send()
            .await;
    }
}

impl Default for Comfy {
    fn default() -> Self {
        Self::new()
    }
}

/// `GET /queue` lists jobs as `[number, prompt_id, ...]` under running and pending.
fn queue_has(body: &Value, prompt_id: &str) -> bool {
    ["queue_running", "queue_pending"].iter().any(|k| {
        body.get(*k)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|item| item.get(1).and_then(Value::as_str) == Some(prompt_id))
    })
}

/// Turn ComfyUI's validation error into one readable sentence.
pub fn explain_rejection(body: &Value) -> String {
    // A node this ComfyUI doesn't have: it is older than the workflow.
    if body.pointer("/error/type").and_then(Value::as_str) == Some("missing_node_type") {
        let class = body
            .pointer("/error/extra_info/class_type")
            .and_then(Value::as_str)
            .unwrap_or("a node it needs");
        return format!(
            "This ComfyUI is too old for this workflow (it has no {class}). Reinstall it in Settings › Engines to get the version Outpost needs (or update your own ComfyUI), or pick another model."
        );
    }
    let top = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("ComfyUI refused the job");
    let mut details: Vec<String> = Vec::new();
    if let Some(nodes) = body.get("node_errors").and_then(Value::as_object) {
        for (_, node) in nodes {
            let class = node.get("class_type").and_then(Value::as_str).unwrap_or("a node");
            for err in node.get("errors").and_then(Value::as_array).into_iter().flatten() {
                let msg = err.get("message").and_then(Value::as_str).unwrap_or("error");
                let detail = err.get("details").and_then(Value::as_str).unwrap_or("");
                if err.get("type").and_then(Value::as_str) == Some("value_not_in_list") {
                    details.push(format!("{class} can't find {detail}: is the model downloaded?"));
                } else {
                    details.push(format!("{class}: {msg} {detail}").trim().to_string());
                }
            }
        }
    }
    if details.is_empty() {
        top.to_string()
    } else {
        format!("{top}. {}", details.join("; "))
    }
}

/// From `GET /history/{id}`: `None` while unfinished, else the outputs or the error.
pub fn read_history(body: &Value, prompt_id: &str) -> Option<Result<Vec<(String, OutputFile)>, ComfyError>> {
    let entry = body.get(prompt_id)?;
    let status = entry.get("status");
    let completed = status
        .and_then(|s| s.get("completed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let status_str = status
        .and_then(|s| s.get("status_str"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if status_str == "error" {
        let messages = status.and_then(|s| s.get("messages")).and_then(Value::as_array);
        let interrupted = messages
            .into_iter()
            .flatten()
            .any(|m| m.get(0).and_then(Value::as_str) == Some("execution_interrupted"));
        if interrupted {
            return Some(Err(ComfyError::Interrupted));
        }
        let why = messages
            .into_iter()
            .flatten()
            .find(|m| m.get(0).and_then(Value::as_str) == Some("execution_error"))
            .and_then(|m| m.get(1))
            .map(|d| {
                let node = d.get("node_type").and_then(Value::as_str).unwrap_or("A node");
                let msg = d
                    .get("exception_message")
                    .and_then(Value::as_str)
                    .unwrap_or("failed")
                    .trim();
                if msg.contains("out of memory") || msg.contains("OutOfMemory") {
                    "Ran out of video memory. Try a smaller size, or close other programs using the GPU.".to_string()
                } else {
                    format!("{node} failed: {msg}")
                }
            })
            .unwrap_or_else(|| "The job failed in ComfyUI.".into());
        return Some(Err(ComfyError::Failed(why)));
    }
    if !completed && status_str != "success" {
        return None;
    }
    let mut files = Vec::new();
    if let Some(outputs) = entry.get("outputs").and_then(Value::as_object) {
        for (node, out) in outputs {
            for (_, list) in out.as_object().into_iter().flatten() {
                for item in list.as_array().into_iter().flatten() {
                    if let Ok(f) = serde_json::from_value::<OutputFile>(item.clone()) {
                        if !f.filename.is_empty() && f.kind == "output" {
                            files.push((node.clone(), f));
                        }
                    }
                }
            }
        }
    }
    Some(Ok(files))
}

/// One websocket message → updated progress for our prompt, if it changes anything.
fn read_ws(text: &str, prompt_id: &str, current: &JobProgress) -> Option<JobProgress> {
    let msg: Value = serde_json::from_str(text).ok()?;
    let data = msg.get("data")?;
    if data
        .get("prompt_id")
        .and_then(Value::as_str)
        .is_some_and(|p| p != prompt_id)
    {
        return None;
    }
    let kind = msg.get("type")?.as_str()?;
    let mut p = current.clone();
    match kind {
        "execution_start" => {
            p.phase = "Loading models".into();
        }
        "progress" => {
            let value = data.get("value")?.as_u64()? as u32;
            let max = data.get("max")?.as_u64()? as u32;
            p.phase = "Generating".into();
            p.step = value;
            p.steps = max;
            p.percent = (max > 0).then(|| value as f32 / max as f32 * 100.0);
        }
        "executing" => {
            // After the sampler finishes, decoding and saving follow.
            if p.steps > 0 && p.step >= p.steps && data.get("node").is_some_and(|n| !n.is_null()) {
                p.phase = "Finishing".into();
            }
        }
        _ => return None,
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejections_read_well() {
        let body = json!({
            "error": {"type": "prompt_outputs_failed_validation", "message": "Prompt outputs failed validation"},
            "node_errors": {"28": {"class_type": "UNETLoader", "errors": [
                {"type": "value_not_in_list", "message": "Value not in list", "details": "unet_name: 'z_image_turbo_bf16.safetensors' not in []"}
            ]}}
        });
        let text = explain_rejection(&body);
        assert!(text.contains("UNETLoader can't find"), "{text}");
        assert!(text.contains("downloaded"), "{text}");

        let old = json!({"error": {"type": "missing_node_type", "message": "Node 'X' not found.",
            "extra_info": {"node_id": "474", "class_type": "TextEncodeQwenImage21"}}, "node_errors": {}});
        let text = explain_rejection(&old);
        assert!(text.contains("too old") && text.contains("TextEncodeQwenImage21"), "{text}");
    }

    #[test]
    fn history_reports_outputs_errors_and_waiting() {
        assert!(read_history(&json!({}), "p1").is_none());
        let running = json!({"p1": {"status": {"status_str": "", "completed": false}, "outputs": {}}});
        assert!(read_history(&running, "p1").is_none());

        let done = json!({"p1": {
            "status": {"status_str": "success", "completed": true, "messages": []},
            "outputs": {"9": {"images": [
                {"filename": "outpost_00001_.png", "subfolder": "outpost", "type": "output"},
                {"filename": "preview.png", "subfolder": "", "type": "temp"}
            ]}}
        }});
        let files = read_history(&done, "p1").unwrap().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "9");
        assert_eq!(files[0].1.filename, "outpost_00001_.png");

        let oom = json!({"p1": {"status": {"status_str": "error", "completed": false, "messages": [
            ["execution_start", {}],
            ["execution_error", {"node_type": "KSampler", "exception_message": "CUDA out of memory. Tried to allocate..."}]
        ]}}});
        match read_history(&oom, "p1") {
            Some(Err(ComfyError::Failed(m))) => assert!(m.contains("video memory"), "{m}"),
            other => panic!("{other:?}"),
        }
        let stopped = json!({"p1": {"status": {"status_str": "error", "messages": [["execution_interrupted", {}]]}}});
        assert!(matches!(
            read_history(&stopped, "p1"),
            Some(Err(ComfyError::Interrupted))
        ));
    }

    #[test]
    fn queue_lists_are_read() {
        let body = json!({"queue_running": [[3, "p1", {}, {}, []]], "queue_pending": [[4, "p2", {}, {}, []]]});
        assert!(queue_has(&body, "p1"));
        assert!(queue_has(&body, "p2"));
        assert!(!queue_has(&body, "p3"));
        assert!(!queue_has(&json!({}), "p1"));
    }

    #[test]
    fn websocket_progress_is_read_for_our_job_only() {
        let start = JobProgress::default();
        let p = read_ws(
            r#"{"type":"progress","data":{"value":3,"max":8,"prompt_id":"p1"}}"#,
            "p1",
            &start,
        )
        .unwrap();
        assert_eq!((p.step, p.steps), (3, 8));
        assert_eq!(p.phase, "Generating");
        assert!(read_ws(
            r#"{"type":"progress","data":{"value":3,"max":8,"prompt_id":"other"}}"#,
            "p1",
            &start
        )
        .is_none());
        assert!(read_ws("not json", "p1", &start).is_none());
    }
}
