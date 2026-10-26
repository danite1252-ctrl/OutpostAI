//! Sending webhooks: a signed JSON POST to each of the account's URLs that wants the
//! event, tried three times (now, after 10 s, after a minute).
//!
//! Every message looks like `{"event", "id", "at", "data"}` and carries headers
//! `X-Outpost-Event`, `X-Outpost-Delivery` and `X-Outpost-Signature: sha256=<hex>`,
//! the HMAC-SHA256 of the raw body with the webhook's secret.

use std::time::Duration;

use outpost_core::jobs::{self, JobStatus};
use outpost_core::webhooks::{self, Webhook};
use outpost_core::{media, now, training};
use serde_json::{json, Value};

use crate::AppState;

/// Gaps before each retry.
const RETRIES: [Duration; 2] = [Duration::from_secs(10), Duration::from_secs(60)];

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Outpost/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("HTTP client")
}

/// The message body for an event.
pub fn envelope(event: &str, data: Value) -> Value {
    json!({
        "event": event,
        "id": uuid::Uuid::new_v4().to_string(),
        "at": now(),
        "data": data,
    })
}

/// POST one message once. Returns the HTTP status (0 when it couldn't connect) and
/// why it failed, if it did.
pub async fn post_once(client: &reqwest::Client, hook: &Webhook, message: &Value) -> (i64, Option<String>) {
    let body = message.to_string();
    let signature = webhooks::sign(&hook.secret, body.as_bytes());
    let sent = client
        .post(&hook.url)
        .header("content-type", "application/json")
        .header("x-outpost-event", message["event"].as_str().unwrap_or_default())
        .header("x-outpost-delivery", message["id"].as_str().unwrap_or_default())
        .header("x-outpost-signature", format!("sha256={signature}"))
        .body(body)
        .send()
        .await;
    match sent {
        Ok(r) if r.status().is_success() => (i64::from(r.status().as_u16()), None),
        Ok(r) => {
            let status = r.status();
            let text: String = r.text().await.unwrap_or_default().chars().take(200).collect();
            (i64::from(status.as_u16()), Some(format!("{status}: {text}")))
        }
        Err(e) if e.is_timeout() => (0, Some("No answer within 10 seconds.".into())),
        Err(_) => (0, Some("Couldn't connect.".into())),
    }
}

/// Tell every webhook of the account that wants `event`, in the background.
pub fn send(state: &AppState, account_id: &str, event: &'static str, data: Value) {
    let state = state.clone();
    let account_id = account_id.to_string();
    tokio::spawn(async move {
        let hooks = match webhooks::for_event(&state.pool, &account_id, event).await {
            Ok(h) if !h.is_empty() => h,
            _ => return,
        };
        let message = envelope(event, data);
        let client = client();
        for hook in hooks {
            let (state, client, message) = (state.clone(), client.clone(), message.clone());
            tokio::spawn(async move {
                let mut result = post_once(&client, &hook, &message).await;
                for gap in RETRIES {
                    if result.1.is_none() {
                        break;
                    }
                    tokio::time::sleep(gap).await;
                    result = post_once(&client, &hook, &message).await;
                }
                if let Some(e) = &result.1 {
                    tracing::warn!(webhook = %hook.id, event, error = %e, "webhook delivery failed");
                }
                let _ = webhooks::record(&state.pool, &hook.id, result.0, result.1.as_deref()).await;
            });
        }
    });
}

/// A finished job: `training.finished` for a training, `job.finished` for the rest.
pub async fn job_finished(state: &AppState, job_id: &str) {
    let Ok(job) = jobs::get_any(&state.pool, job_id).await else { return };
    let status = match job.status {
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
        _ => return,
    };
    if job.pack_id == crate::train::PACK_ID {
        let Ok(t) = training::training_for_job(&state.pool, &job.id).await else { return };
        let run = training::run_dir(&crate::train::root(state), &t.id);
        let checkpoints: Vec<u32> = outpost_engines::trainer::checkpoints(&run, t.steps as u32)
            .into_iter()
            .filter(|c| !c.file.is_empty())
            .map(|c| c.step)
            .collect();
        let name = training::get_dataset(&state.pool, &t.account_id, &t.dataset_id)
            .await
            .map(|d| d.name)
            .unwrap_or_default();
        send(
            state,
            &job.account_id,
            "training.finished",
            json!({ "training": {
                "id": t.id, "dataset_id": t.dataset_id, "name": name, "trigger_word": t.trigger_word,
                "steps": t.steps, "status": status, "error": job.error, "checkpoints": checkpoints,
            }}),
        );
        return;
    }
    let inputs: Value = serde_json::from_str(&job.inputs).unwrap_or(Value::Null);
    let items: Vec<Value> = media::for_job(&state.pool, &job.account_id, &job.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| json!({ "id": m.id, "kind": m.kind, "mime": m.mime, "url": format!("/api/media/{}", m.id) }))
        .collect();
    send(
        state,
        &job.account_id,
        "job.finished",
        json!({
            "job": {
                "id": job.id, "pack_id": job.pack_id, "status": status, "error": job.error,
                "prompt": inputs.get("prompt"), "seed": job.seed,
                "created_at": job.created_at, "finished_at": job.finished_at,
            },
            "media": items,
        }),
    );
}

/// A new character (trained here or brought in).
pub fn character_created(state: &AppState, c: &outpost_core::training::Character) {
    send(state, &c.account_id, "character.created", json!({ "character": c }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_carry_event_id_and_time() {
        let m = envelope("job.finished", json!({"job": {"id": "x"}}));
        assert_eq!(m["event"], "job.finished");
        assert_eq!(m["data"]["job"]["id"], "x");
        assert_eq!(m["id"].as_str().unwrap().len(), 36);
        assert!(m["at"].as_i64().unwrap() > 0);
    }
}
