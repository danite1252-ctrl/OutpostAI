//! Tools on the user's PC: MCP servers the Outpost desktop app runs there (web search,
//! GitHub, a browser...). The app sends the tools of the servers this chat uses with each
//! message; each call the model makes goes back to that app (a `file_request` event with
//! `op: "mcp"`, like PC folders), which runs it and answers. Phones don't send any.

use std::time::Duration;

use axum::response::sse::Event;
use outpost_core::chat_files::FileAction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use utoipa::ToSchema;

use crate::workspace::{fail, Outcome, Relay};

/// Most tool servers one message may bring, and most tools in all (each costs context).
pub const MAX_SERVERS: usize = 8;
pub const MAX_TOOLS: usize = 80;
/// Longest a tool on the PC may take (a browser loading pages, a search).
const CALL_WITHIN: Duration = Duration::from_secs(180);
/// Most of a tool's answer the model reads.
const ANSWER_LIMIT: usize = 16_000;

/// One MCP server on the PC, with its tools (as MCP's `tools/list` gives them).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PcToolServer {
    /// The app's id for it.
    pub id: String,
    /// Its name, e.g. "GitHub".
    pub name: String,
    #[serde(default)]
    pub tools: Vec<PcTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PcTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema for its arguments.
    #[serde(default, rename = "inputSchema", alias = "input_schema")]
    #[schema(value_type = Object)]
    pub input_schema: Value,
}

struct Entry {
    /// The name the model calls it by: `<server>__<tool>`.
    call: String,
    server: usize,
    tool: String,
}

/// A message's PC tools.
pub struct PcTools {
    relay: Relay,
    chat_id: String,
    events: mpsc::Sender<Event>,
    servers: Vec<PcToolServer>,
    entries: Vec<Entry>,
}

/// Letters, digits, `_` and `-` only (what models and Ollama accept in a tool's name).
fn slug(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "tool".into()
    } else {
        out
    }
}

impl PcTools {
    /// `None` when there are no tools to offer.
    pub fn new(relay: Relay, chat_id: &str, events: mpsc::Sender<Event>, servers: Vec<PcToolServer>) -> Option<Self> {
        let servers: Vec<PcToolServer> = servers.into_iter().filter(|s| !s.tools.is_empty()).take(MAX_SERVERS).collect();
        let mut entries: Vec<Entry> = Vec::new();
        for (i, s) in servers.iter().enumerate() {
            let prefix: String = slug(&s.name).chars().take(20).collect();
            for t in &s.tools {
                if entries.len() >= MAX_TOOLS {
                    break;
                }
                let base: String = format!("{prefix}__{}", slug(&t.name)).chars().take(60).collect();
                let mut call = base.clone();
                let mut n = 2;
                while entries.iter().any(|e| e.call == call) {
                    call = format!("{base}{n}");
                    n += 1;
                }
                entries.push(Entry { call, server: i, tool: t.name.clone() });
            }
        }
        (!entries.is_empty()).then(|| Self { relay, chat_id: chat_id.into(), events, servers, entries })
    }

    /// The system prompt's part about them.
    pub fn instructions(&self) -> String {
        let names: Vec<&str> = self.servers.iter().map(|s| s.name.as_str()).collect();
        format!(
            "## Tools\nYou can use tools from: {}. Use them when they help answer (to look things up, check facts or \
             act for the user), and say what you found or did. Don't make up a tool's results; if one fails, say so.",
            names.join(", ")
        )
    }

    /// The tool definitions for Ollama.
    pub fn tools(&self) -> Vec<Value> {
        self.entries
            .iter()
            .map(|e| {
                let s = &self.servers[e.server];
                let t = s.tools.iter().find(|t| t.name == e.tool).expect("entry's tool");
                let mut params = t.input_schema.clone();
                if !params.is_object() {
                    params = json!({ "type": "object", "properties": {} });
                }
                if params.get("type").is_none() {
                    params["type"] = json!("object");
                }
                let description: String = format!("({}) {}", s.name, t.description.trim()).chars().take(1200).collect();
                json!({ "type": "function", "function": { "name": e.call, "description": description, "parameters": params } })
            })
            .collect()
    }

    pub fn has(&self, call: &str) -> bool {
        self.entries.iter().any(|e| e.call == call)
    }

    /// Run one call on the PC.
    pub async fn run(&self, call: &str, args: &Value) -> Outcome {
        let Some(e) = self.entries.iter().find(|e| e.call == call) else {
            return fail("mcp", call, format!("There's no tool called {call}."));
        };
        let server = &self.servers[e.server];
        let shown = format!("{} › {}", server.name, e.tool);
        let args = if args.is_object() { args.clone() } else { json!({}) };
        let request = json!({ "op": "mcp", "server": server.id, "tool": e.tool, "arguments": args });
        match self.relay.ask_within(&self.events, &self.chat_id, request, CALL_WITHIN).await {
            Ok(v) => {
                let text = v.get("text").and_then(Value::as_str).unwrap_or_default();
                let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let mut reply: String = text.chars().take(ANSWER_LIMIT).collect();
                if text.chars().count() > ANSWER_LIMIT {
                    reply.push_str("\n[... the rest was cut off]");
                }
                if reply.trim().is_empty() {
                    reply = "(no output)".into();
                }
                let note = if is_error { Some(text.chars().take(200).collect::<String>()) } else { None };
                Outcome {
                    reply: if is_error { format!("The tool reported an error: {reply}") } else { reply },
                    action: FileAction { op: "mcp".into(), path: shown, note, change_id: None, failed: is_error },
                }
            }
            Err(why) => fail("mcp", &shown, why),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, tools: &[&str]) -> PcToolServer {
        PcToolServer {
            id: format!("id-{name}"),
            name: name.into(),
            tools: tools
                .iter()
                .map(|t| PcTool { name: (*t).into(), description: format!("Does {t}"), input_schema: Value::Null })
                .collect(),
        }
    }

    #[test]
    fn pc_tools_get_safe_unique_names_and_valid_schemas() {
        let (tx, _rx) = mpsc::channel(4);
        let none = PcTools::new(Relay::default(), "c", tx.clone(), vec![server("Empty", &[])]);
        assert!(none.is_none());
        let t = PcTools::new(
            Relay::default(),
            "c",
            tx,
            vec![server("Brave Search", &["brave_web_search", "brave.local search"]), server("GitHub", &["search", "search"])],
        )
        .unwrap();
        let tools = t.tools();
        let names: Vec<&str> = tools.iter().map(|v| v["function"]["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["brave_search__brave_web_search", "brave_search__brave_local_search", "github__search", "github__search2"]);
        assert!(t.has("github__search2"));
        assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
        assert!(tools[2]["function"]["description"].as_str().unwrap().starts_with("(GitHub) "));
        assert!(t.instructions().contains("Brave Search, GitHub"));
    }

    #[tokio::test]
    async fn a_call_goes_to_the_app_and_its_answer_back_to_the_model() {
        let (tx, mut rx) = mpsc::channel(4);
        let relay = Relay::default();
        let t = PcTools::new(relay.clone(), "chat1", tx, vec![server("Fetch", &["fetch"])]).unwrap();
        let app = tokio::spawn(async move {
            // The app sees the request (as an SSE event) and answers it.
            let _event = rx.recv().await.unwrap();
            for _ in 0..50 {
                if relay.pending_ids().len() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let id = relay.pending_ids().pop().unwrap();
            assert!(!relay.answer("someone-else", &id, json!({ "text": "x" })));
            assert!(relay.answer("chat1", &id, json!({ "text": "Page text", "is_error": false })));
        });
        let out = t.run("fetch__fetch", &json!({ "url": "https://example.com" })).await;
        app.await.unwrap();
        assert_eq!(out.reply, "Page text");
        assert_eq!(out.action.path, "Fetch › fetch");
        assert!(!out.action.failed);
        let missing = t.run("nope", &json!({})).await;
        assert!(missing.action.failed);
    }
}
