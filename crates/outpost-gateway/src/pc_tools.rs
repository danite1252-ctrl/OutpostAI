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
/// Longest to wait for a yes or no.
const CONFIRM_WITHIN: Duration = Duration::from_secs(60);
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
    /// It only reads (changes nothing), as its server says.
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// Where a tool server runs: on the app streaming the reply (desktop), on your PC that offers
/// it for chats from elsewhere (see [`crate::device_tools`]), or on this server (see
/// [`crate::server_tools`]).
#[derive(Clone)]
pub enum Via {
    App,
    Device { tools: crate::device_tools::DeviceTools, session_id: String },
    Server(crate::AppState),
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
    /// Where each of `servers` runs.
    routes: Vec<Via>,
    entries: Vec<Entry>,
    /// Ask the user before each call (hands-free: speech can be misheard).
    confirm: bool,
}

/// A tool's name as words: `HassTurnOff` → "turn off", `search_files` → "search files".
pub fn tool_words(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let boundary = c == '_' || c == '-' || c == '.' || c == ' ';
        let upper_start = c.is_uppercase() && i > 0 && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if boundary || upper_start {
            if !cur.is_empty() {
                words.push(cur.to_lowercase());
                cur.clear();
            }
            if boundary {
                continue;
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur.to_lowercase());
    }
    // Home Assistant's tools all start "Hass".
    if words.first().is_some_and(|w| w == "hass") && words.len() > 1 {
        words.remove(0);
    }
    words.join(" ")
}

/// The arguments, briefly, for saying aloud ("kitchen lamp, 50").
pub fn args_words(args: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(o) = args.as_object() {
        for v in o.values() {
            match v {
                Value::String(s) if !s.trim().is_empty() => parts.push(s.trim().chars().take(60).collect()),
                Value::Number(n) => parts.push(n.to_string()),
                Value::Bool(true) => {}
                Value::Array(a) => parts.extend(a.iter().filter_map(Value::as_str).take(3).map(str::to_string)),
                _ => {}
            }
        }
    }
    let s = parts.join(", ");
    if s.chars().count() > 140 {
        format!("{}…", s.chars().take(140).collect::<String>())
    } else {
        s
    }
}

/// The question asked before a call: "Would you like me to check Google Calendar?".
pub fn confirm_question(server: &str, tool: &str, args: &Value, read_only: Option<bool>) -> String {
    let what = args_words(args);
    let words = tool_words(tool);
    let reads = read_only == Some(true)
        || ["get", "list", "search", "read", "find", "fetch", "query", "lookup", "events"].iter().any(|w| words.split(' ').next() == Some(*w) || words.ends_with(w));
    if reads {
        if what.is_empty() {
            format!("Would you like me to check {server}?")
        } else {
            format!("Would you like me to check {server} for {what}?")
        }
    } else if what.is_empty() {
        format!("Would you like me to {words} with {server}?")
    } else {
        format!("Would you like me to {words}: {what}, with {server}?")
    }
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

/// Smart-home calls go wrong when a model guesses a name or sends only a room (which acts on
/// everything in it: the lamp *and* the air conditioner).
const HOME_ASSISTANT_NOTE: &str = "\n\nHome Assistant: to switch or change a device, pass its exact name as Home \
    Assistant lists it (call GetLiveContext first to see the devices, their names and areas, unless this chat already \
    did). Pass the device's name, not just a room or area: a room alone acts on every device in it. Only send an area \
    (with no name) when the user asked for everything in that room. If two devices could match (\"kitchen light\" and \
    \"kitchen lamp\"), ask which.";

impl PcTools {
    /// `None` when there are no tools to offer.
    pub fn new(relay: Relay, chat_id: &str, events: mpsc::Sender<Event>, servers: Vec<(PcToolServer, Via)>, confirm: bool) -> Option<Self> {
        let (servers, routes): (Vec<PcToolServer>, Vec<Via>) =
            servers.into_iter().filter(|(s, _)| !s.tools.is_empty()).take(MAX_SERVERS).unzip();
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
        (!entries.is_empty()).then(|| Self { relay, chat_id: chat_id.into(), events, servers, routes, entries, confirm })
    }

    /// The system prompt's part about them.
    pub fn instructions(&self) -> String {
        let names: Vec<&str> = self.servers.iter().map(|s| s.name.as_str()).collect();
        format!(
            "## Tools\nYou can use tools from: {}. Use them when they help answer (to look things up, check facts or \
             act for the user), and say what you found or did. Don't make up a tool's results; if one fails, say so. \
             Each time the user asks for something to be done (even the same thing again, or undoing it: \"turn it back \
             on\"), call the tool again in this reply: earlier calls don't carry over, and nothing happens unless you call \
             a tool now. Never say something was done unless a tool call in this reply did it. When acting on \
             something (a device, a file, a message), name exactly that one thing; if you aren't sure which one the user \
             means, look it up with a tool or ask, rather than guessing.{}",
            names.join(", "),
            if self.has_home_assistant() { HOME_ASSISTANT_NOTE } else { "" }
        )
    }

    /// Home Assistant's tools are here (their names start "Hass", plus GetLiveContext).
    fn has_home_assistant(&self) -> bool {
        self.servers.iter().any(|s| s.tools.iter().any(|t| t.name.starts_with("Hass") || t.name == "GetLiveContext"))
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
        if self.confirm {
            let read_only = server.tools.iter().find(|t| t.name == e.tool).and_then(|t| t.read_only);
            let question = confirm_question(&server.name, &e.tool, &args, read_only);
            let asked = json!({ "op": "confirm", "question": question, "server": server.name, "tool": e.tool, "arguments": args });
            let ok = match self.relay.ask_within(&self.events, &self.chat_id, asked, CONFIRM_WITHIN, "confirm").await {
                Ok(v) => v.get("ok").and_then(Value::as_bool).unwrap_or(false),
                Err(_) => false,
            };
            if !ok {
                return Outcome {
                    reply: "The user said no to this (or didn't answer), so it wasn't done. Speech can be misheard: briefly \
                            ask what they'd like instead."
                        .into(),
                    action: FileAction { op: "mcp".into(), path: shown, note: Some("Not confirmed".into()), change_id: None, failed: true },
                    more: Vec::new(),
                };
            }
        }
        let request = json!({ "op": "mcp", "server": server.id, "tool": e.tool, "arguments": args.clone() });
        let answer = match &self.routes[e.server] {
            Via::App => self.relay.ask_within(&self.events, &self.chat_id, request, CALL_WITHIN, "file_request").await,
            Via::Device { tools, session_id } => tools.call(session_id, request, CALL_WITHIN).await,
            Via::Server(state) => match tokio::time::timeout(CALL_WITHIN, crate::server_tools::call(state, &server.id, &e.tool, args)).await {
                Ok(Ok(r)) => Ok(json!({ "text": r.text, "is_error": r.is_error })),
                Ok(Err(err)) => Err(err.trim_start_matches("needs sign-in: ").to_string()),
                Err(_) => Err("The tool took too long.".into()),
            },
        };
        match answer {
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
                    more: Vec::new(),
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
                .map(|t| PcTool { name: (*t).into(), description: format!("Does {t}"), input_schema: Value::Null, read_only: None })
                .collect(),
        }
    }

    #[test]
    fn pc_tools_get_safe_unique_names_and_valid_schemas() {
        let (tx, _rx) = mpsc::channel(4);
        let none = PcTools::new(Relay::default(), "c", tx.clone(), vec![(server("Empty", &[]), Via::App)], false);
        assert!(none.is_none());
        let t = PcTools::new(
            Relay::default(),
            "c",
            tx,
            vec![(server("Brave Search", &["brave_web_search", "brave.local search"]), Via::App), (server("GitHub", &["search", "search"]), Via::App)],
            false,
        )
        .unwrap();
        let tools = t.tools();
        let names: Vec<&str> = tools.iter().map(|v| v["function"]["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["brave_search__brave_web_search", "brave_search__brave_local_search", "github__search", "github__search2"]);
        assert!(t.has("github__search2"));
        assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
        assert!(tools[2]["function"]["description"].as_str().unwrap().starts_with("(GitHub) "));
        assert!(t.instructions().contains("Brave Search, GitHub"));
        assert!(!t.instructions().contains("GetLiveContext"));
        // Home Assistant gets told to use exact device names, not just a room.
        let (tx, _rx) = mpsc::channel(1);
        let ha = PcTools::new(Relay::default(), "c", tx, vec![(server("Home Assistant", &["HassTurnOff", "GetLiveContext"]), Via::App)], false).unwrap();
        assert!(ha.instructions().contains("call GetLiveContext first"));
    }

    #[tokio::test]
    async fn a_call_goes_to_the_app_and_its_answer_back_to_the_model() {
        let (tx, mut rx) = mpsc::channel(4);
        let relay = Relay::default();
        let t = PcTools::new(relay.clone(), "chat1", tx, vec![(server("Fetch", &["fetch"]), Via::App)], false).unwrap();
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

    #[test]
    fn questions_read_naturally() {
        assert_eq!(tool_words("HassTurnOff"), "turn off");
        assert_eq!(tool_words("calendar_events"), "calendar events");
        assert_eq!(tool_words("GetHTMLPage"), "get html page");
        assert_eq!(confirm_question("Google", "calendar_events", &json!({}), None), "Would you like me to check Google?");
        assert_eq!(
            confirm_question("Home Assistant", "HassTurnOff", &json!({ "name": "kitchen lamp" }), None),
            "Would you like me to turn off: kitchen lamp, with Home Assistant?"
        );
        assert_eq!(
            confirm_question("DuckDuckGo Search", "search", &json!({ "query": "weather Sydney" }), Some(true)),
            "Would you like me to check DuckDuckGo Search for weather Sydney?"
        );
    }

    #[tokio::test]
    async fn hands_free_asks_first_and_a_no_stops_it() {
        let (tx, mut rx) = mpsc::channel(4);
        let relay = Relay::default();
        let t = PcTools::new(relay.clone(), "chat1", tx, vec![(server("Home Assistant", &["HassTurnOff"]), Via::App)], true).unwrap();
        let app = tokio::spawn(async move {
            // The question comes first; the user says no.
            let ev = rx.recv().await.unwrap();
            let _ = ev;
            for _ in 0..50 {
                if relay.pending_ids().len() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let id = relay.pending_ids().pop().unwrap();
            assert!(relay.answer("chat1", &id, json!({ "ok": false })));
            // Nothing else is asked of the app.
            assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.is_err());
        });
        let out = t.run("home_assistant__hassturnoff", &json!({ "name": "lamp" })).await;
        app.await.unwrap();
        assert!(out.action.failed);
        assert_eq!(out.action.note.as_deref(), Some("Not confirmed"));
        assert!(out.reply.contains("said no"));
    }
}
