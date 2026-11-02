//! Google tools built into the desktop app: Drive, Docs, Sheets, Gmail and Calendar, through
//! Google's ordinary APIs (so personal Google accounts work), signed in with the user's own
//! OAuth client (see [`crate::mcp_oauth::google_state`]). The chat sees them like any other
//! tool server's; the app runs them with the signed-in token.
//!
//! Which parts are on, and whether it may change things, is the server's `args`: any of
//! `drive`, `docs`, `sheets`, `gmail`, `calendar`, plus `write`. Mail is never sent: the
//! model writes drafts for you to send.

use base64::Engine;
use serde_json::{json, Value};

use crate::mcp_client::{CallResult, ToolInfo};

pub const AREAS: [&str; 5] = ["drive", "docs", "sheets", "gmail", "calendar"];

/// Most of a file, document or message the model reads at once.
const READ_LIMIT: usize = 20_000;

/// The Google scopes these settings need.
pub fn scopes(args: &[String]) -> String {
    let on = |a: &str| args.iter().any(|x| x == a);
    let write = on("write");
    let mut s: Vec<&str> = vec!["openid", "email"];
    if on("drive") {
        s.push("https://www.googleapis.com/auth/drive.readonly");
    }
    if on("drive") && write {
        s.push("https://www.googleapis.com/auth/drive.file");
    }
    if on("docs") {
        s.push(if write { "https://www.googleapis.com/auth/documents" } else { "https://www.googleapis.com/auth/documents.readonly" });
    }
    if on("sheets") {
        s.push(if write { "https://www.googleapis.com/auth/spreadsheets" } else { "https://www.googleapis.com/auth/spreadsheets.readonly" });
    }
    if on("gmail") {
        s.push("https://www.googleapis.com/auth/gmail.readonly");
        if write {
            s.push("https://www.googleapis.com/auth/gmail.compose");
        }
    }
    if on("calendar") {
        s.push(if write { "https://www.googleapis.com/auth/calendar.events" } else { "https://www.googleapis.com/auth/calendar.readonly" });
    }
    s.join(" ")
}

/// Whether a sign-in's scopes cover these settings (else it must sign in again).
pub fn covers(granted: &str, args: &[String]) -> bool {
    let have: Vec<&str> = granted.split_whitespace().collect();
    scopes(args).split_whitespace().filter(|s| *s != "openid" && *s != "email").all(|s| have.contains(&s))
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> ToolInfo {
    ToolInfo {
        name: name.into(),
        description: description.into(),
        input_schema: json!({ "type": "object", "properties": props, "required": required }),
        read_only: None,
    }
}

/// The tools these settings give.
pub fn tools(args: &[String]) -> Vec<ToolInfo> {
    let on = |a: &str| args.iter().any(|x| x == a);
    let write = on("write");
    let id = |what: &str| json!({ "type": "string", "description": format!("The {what}'s ID (from a search, or the part of its address after /d/)") });
    let mut out = Vec::new();
    if on("drive") {
        out.push(tool(
            "drive_search",
            "Find files in Google Drive by words in their name or content. Leave the query empty for recent files.",
            json!({ "query": { "type": "string" }, "max": { "type": "integer", "description": "Most results (default 10)" } }),
            &[],
        ));
        out.push(tool(
            "drive_read",
            "Read a Drive file as text: Google Docs, Sheets (as CSV) and Slides, and text files.",
            json!({ "file_id": id("file") }),
            &["file_id"],
        ));
        if write {
            out.push(tool(
                "drive_create",
                "Create a new Google Doc (from text), Google Sheet (from CSV) or plain text file in Drive.",
                json!({
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["doc", "sheet", "text"] },
                    "content": { "type": "string", "description": "The text, or for a sheet CSV rows" },
                    "folder_id": { "type": "string", "description": "Optional folder to put it in" }
                }),
                &["name", "kind", "content"],
            ));
        }
    }
    if on("docs") {
        out.push(tool("docs_read", "Read a Google Doc's text.", json!({ "document_id": id("document") }), &["document_id"]));
        if write {
            out.push(tool(
                "docs_append",
                "Add text to the end of a Google Doc.",
                json!({ "document_id": id("document"), "text": { "type": "string" } }),
                &["document_id", "text"],
            ));
            out.push(tool(
                "docs_replace",
                "Replace every place an exact piece of text appears in a Google Doc.",
                json!({ "document_id": id("document"), "find": { "type": "string" }, "replace": { "type": "string" } }),
                &["document_id", "find", "replace"],
            ));
        }
    }
    if on("sheets") {
        out.push(tool(
            "sheets_read",
            "Read a Google Sheet: its tabs, and the values in a range (A1 notation like 'Budget'!A1:F50; the first tab if left out).",
            json!({ "spreadsheet_id": id("spreadsheet"), "range": { "type": "string" } }),
            &["spreadsheet_id"],
        ));
        if write {
            let rows = json!({ "type": "array", "items": { "type": "array", "items": {} }, "description": "Rows of cell values; formulas like =SUM(A1:A3) work" });
            out.push(tool(
                "sheets_write",
                "Write values into a range of a Google Sheet (overwriting what's there).",
                json!({ "spreadsheet_id": id("spreadsheet"), "range": { "type": "string" }, "values": rows.clone() }),
                &["spreadsheet_id", "range", "values"],
            ));
            out.push(tool(
                "sheets_append",
                "Add rows after the last row of a table in a Google Sheet.",
                json!({ "spreadsheet_id": id("spreadsheet"), "range": { "type": "string", "description": "The table, e.g. 'Expenses'!A:E" }, "values": rows }),
                &["spreadsheet_id", "range", "values"],
            ));
        }
    }
    if on("gmail") {
        out.push(tool(
            "gmail_search",
            "Search Gmail like the Gmail search box (from:, subject:, newer_than:7d, is:unread...).",
            json!({ "query": { "type": "string" }, "max": { "type": "integer", "description": "Most results (default 10)" } }),
            &["query"],
        ));
        out.push(tool("gmail_read", "Read one email (from a search).", json!({ "message_id": { "type": "string" } }), &["message_id"]));
        if write {
            out.push(tool(
                "gmail_draft",
                "Write an email as a draft in Gmail, for the user to check and send (it is never sent from here). Give reply_to to answer an email.",
                json!({
                    "to": { "type": "string" },
                    "subject": { "type": "string" },
                    "body": { "type": "string" },
                    "reply_to": { "type": "string", "description": "Optional: the message ID being answered" }
                }),
                &["body"],
            ));
        }
    }
    if on("calendar") {
        out.push(tool(
            "calendar_events",
            "List events on the user's Google Calendar (the next 7 days unless from/to are given, as dates or RFC 3339 times).",
            json!({ "from": { "type": "string" }, "to": { "type": "string" }, "query": { "type": "string" }, "max": { "type": "integer" } }),
            &[],
        ));
        if write {
            out.push(tool(
                "calendar_create",
                "Add an event to the user's Google Calendar. start/end are times with the user's UTC offset (2026-11-03T10:00:00+11:00), or dates for all-day events (end = the day after).",
                json!({
                    "summary": { "type": "string" },
                    "start": { "type": "string" },
                    "end": { "type": "string" },
                    "description": { "type": "string" },
                    "location": { "type": "string" }
                }),
                &["summary", "start", "end"],
            ));
        }
    }
    // Which only read (hands-free asks "check..." for those).
    const CHANGES: [&str; 7] = ["drive_create", "docs_append", "docs_replace", "sheets_write", "sheets_append", "gmail_draft", "calendar_create"];
    for t in &mut out {
        t.read_only = Some(!CHANGES.contains(&t.name.as_str()));
    }
    out
}

/// Google's answer as an error for the model (or the app: sign in again).
fn google_error(status: u16, body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or_default();
    let msg = v["error"]["message"].as_str().or(v["error_description"].as_str()).unwrap_or(body).chars().take(300).collect::<String>();
    match status {
        401 => "needs sign-in: Google wants you to sign in again (Tools › Google).".into(),
        403 if msg.contains("insufficient") || msg.contains("scope") => {
            "Google didn't allow that: sign in again from Tools so Outpost can ask for that part of your account.".into()
        }
        403 if msg.contains("has not been used") || msg.contains("disabled") => format!(
            "That Google API isn't switched on in your Google Cloud project: {msg}"
        ),
        404 => "Google couldn't find that (check the ID, or that it's shared with you).".into(),
        _ => format!("Google said {status}: {msg}"),
    }
}

struct Api<'a> {
    client: &'a reqwest::Client,
    token: &'a str,
}

impl Api<'_> {
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<(String, String), String> {
        let r = req.bearer_auth(self.token).send().await.map_err(|e| format!("Can't reach Google: {e}"))?;
        let status = r.status().as_u16();
        let kind = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let body = r.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(google_error(status, &body));
        }
        Ok((kind, body))
    }
    async fn json(&self, req: reqwest::RequestBuilder) -> Result<Value, String> {
        let (_, body) = self.send(req).await?;
        serde_json::from_str(&body).map_err(|e| format!("Google's answer was garbled: {e}"))
    }
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.get(url)
    }
}

fn cut(s: &str) -> String {
    if s.chars().count() <= READ_LIMIT {
        return s.to_string();
    }
    let mut out: String = s.chars().take(READ_LIMIT).collect();
    out.push_str("\n[... cut off here]");
    out
}

fn arg<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

/// Quote a value for a Drive query.
fn drive_quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// A Doc's text: its paragraphs and table cells, in order.
pub fn doc_text(doc: &Value) -> String {
    fn walk(content: &Value, out: &mut String) {
        for el in content.as_array().into_iter().flatten() {
            if let Some(p) = el.get("paragraph") {
                for e in p["elements"].as_array().into_iter().flatten() {
                    if let Some(t) = e["textRun"]["content"].as_str() {
                        out.push_str(t);
                    }
                }
            } else if let Some(table) = el.get("table") {
                for row in table["tableRows"].as_array().into_iter().flatten() {
                    let mut cells = Vec::new();
                    for cell in row["tableCells"].as_array().into_iter().flatten() {
                        let mut s = String::new();
                        walk(&cell["content"], &mut s);
                        cells.push(s.trim().replace('\n', " "));
                    }
                    out.push_str(&format!("| {} |\n", cells.join(" | ")));
                }
            }
        }
    }
    let mut out = String::new();
    walk(&doc["body"]["content"], &mut out);
    out
}

/// An email's text: its plain part, else its HTML without tags.
pub fn mail_text(payload: &Value) -> String {
    fn find(p: &Value, mime: &str) -> Option<String> {
        if p["mimeType"].as_str() == Some(mime) {
            if let Some(data) = p["body"]["data"].as_str() {
                let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(data.trim_end_matches('='))
                    .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(data))
                    .ok()?;
                return Some(String::from_utf8_lossy(&bytes).to_string());
            }
        }
        p["parts"].as_array().into_iter().flatten().find_map(|c| find(c, mime))
    }
    if let Some(t) = find(payload, "text/plain") {
        return t;
    }
    let html = find(payload, "text/html").unwrap_or_default();
    // Tags out, a line break per block.
    let mut out = String::new();
    let mut in_tag = false;
    let mut skip_until: Option<&str> = None;
    let mut i = 0;
    let bytes: Vec<char> = html.chars().collect();
    // (ASCII only, so it lines up with `bytes` character for character.)
    let lower: Vec<char> = html.chars().map(|c| c.to_ascii_lowercase()).collect();
    while i < bytes.len() {
        let rest: String = lower[i..lower.len().min(i + 8)].iter().collect();
        if let Some(end) = skip_until {
            if rest.starts_with(end) {
                skip_until = None;
            }
            i += 1;
            continue;
        }
        if rest.starts_with("<style") {
            skip_until = Some("</style");
        } else if rest.starts_with("<script") {
            skip_until = Some("</script");
        }
        let c = bytes[i];
        if c == '<' {
            in_tag = true;
            if rest.starts_with("<br") || rest.starts_with("<p") || rest.starts_with("</p") || rest.starts_with("<div") || rest.starts_with("<tr") || rest.starts_with("<li") {
                out.push('\n');
            }
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
        i += 1;
    }
    let out = out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    let lines: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.join("\n")
}

/// A header value, encoded if it isn't plain ASCII.
fn header_value(s: &str) -> String {
    let s = s.replace(['\r', '\n'], " ");
    if s.is_ascii() {
        s
    } else {
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(s.as_bytes()))
    }
}

/// An email as Gmail takes it (RFC 822, URL-safe base64).
pub fn raw_mail(to: &str, subject: &str, body: &str, in_reply_to: Option<&str>) -> String {
    let mut m = String::new();
    if !to.is_empty() {
        m.push_str(&format!("To: {}\r\n", to.replace(['\r', '\n'], " ")));
    }
    m.push_str(&format!("Subject: {}\r\n", header_value(subject)));
    if let Some(id) = in_reply_to {
        m.push_str(&format!("In-Reply-To: {id}\r\nReferences: {id}\r\n"));
    }
    m.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n");
    m.push_str(&body.replace("\r\n", "\n").replace('\n', "\r\n"));
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(m.as_bytes())
}

/// The signed-in account's email address.
pub async fn account_email(client: &reqwest::Client, token: &str) -> Option<String> {
    let r = client.get("https://openidconnect.googleapis.com/v1/userinfo").bearer_auth(token).send().await.ok()?;
    r.json::<Value>().await.ok()?["email"].as_str().map(str::to_string)
}

fn now_rfc3339(offset_days: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    crate::diagnostics::rfc3339(now + offset_days * 86_400)
}

/// A date (all day) or a time, as Calendar takes it; a bare date gets midnight UTC for ranges.
fn when(s: &str) -> Value {
    if s.len() == 10 && s.as_bytes().get(4) == Some(&b'-') {
        json!({ "date": s })
    } else {
        json!({ "dateTime": s })
    }
}

fn range_time(s: &str) -> String {
    if s.len() == 10 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    }
}

/// Run one tool. Errors starting `needs sign-in:` mean the token's no good (the app refreshes
/// it and tries again, or asks you to sign in).
pub async fn call(client: &reqwest::Client, token: &str, args_on: &[String], name: &str, args: &Value) -> Result<CallResult, String> {
    if !tools(args_on).iter().any(|t| t.name == name) {
        return Ok(CallResult { text: format!("{name} isn't switched on in Outpost's Google settings."), is_error: true });
    }
    let api = Api { client, token };
    let enc = |s: &str| reqwest::Url::parse(&format!("http://x/{}", s.replace('/', "%2F"))).map(|u| u.path()[1..].to_string()).unwrap_or_default();
    let result: Result<String, String> = async {
        match name {
            "drive_search" => {
                let q = arg(args, "query");
                let max = args.get("max").and_then(Value::as_u64).unwrap_or(10).clamp(1, 50);
                let mut params: Vec<(&str, String)> = vec![
                    ("pageSize", max.to_string()),
                    ("fields", "files(id,name,mimeType,modifiedTime,webViewLink,size)".into()),
                    ("supportsAllDrives", "true".into()),
                    ("includeItemsFromAllDrives", "true".into()),
                ];
                if q.is_empty() {
                    params.push(("q", "trashed = false".into()));
                    params.push(("orderBy", "modifiedTime desc".into()));
                } else {
                    params.push(("q", format!("(fullText contains {0} or name contains {0}) and trashed = false", drive_quote(q))));
                }
                let v = api.json(api.get("https://www.googleapis.com/drive/v3/files").query(&params)).await?;
                let files = v["files"].as_array().cloned().unwrap_or_default();
                if files.is_empty() {
                    return Ok("No files found.".into());
                }
                Ok(files
                    .iter()
                    .map(|f| {
                        format!(
                            "- {} (id {}, {}, modified {}){}",
                            f["name"].as_str().unwrap_or("?"),
                            f["id"].as_str().unwrap_or("?"),
                            f["mimeType"].as_str().unwrap_or("?").replace("application/vnd.google-apps.", "google "),
                            f["modifiedTime"].as_str().unwrap_or("?"),
                            f["webViewLink"].as_str().map(|l| format!(" {l}")).unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "drive_read" => {
                let id = arg(args, "file_id");
                let meta = api
                    .json(api.get(&format!("https://www.googleapis.com/drive/v3/files/{}", enc(id))).query(&[("fields", "id,name,mimeType,size"), ("supportsAllDrives", "true")]))
                    .await?;
                let mime = meta["mimeType"].as_str().unwrap_or("");
                let title = meta["name"].as_str().unwrap_or("file");
                let export = match mime {
                    "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => Some("text/plain"),
                    "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
                    _ => None,
                };
                let text = if let Some(to) = export {
                    api.send(api.get(&format!("https://www.googleapis.com/drive/v3/files/{}/export", enc(id))).query(&[("mimeType", to)])).await?.1
                } else if mime.starts_with("text/") || mime == "application/json" || mime.ends_with("+xml") {
                    let size: u64 = meta["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
                    if size > 5_000_000 {
                        return Ok(format!("{title} is too big to read ({} MB).", size / 1_000_000));
                    }
                    api.send(api.get(&format!("https://www.googleapis.com/drive/v3/files/{}", enc(id))).query(&[("alt", "media")])).await?.1
                } else {
                    return Ok(format!("{title} is a {mime} file, which can't be read as text here."));
                };
                Ok(format!("{title}{}\n\n{}", if export == Some("text/csv") { " (first tab, as CSV)" } else { "" }, cut(&text)))
            }
            "drive_create" => {
                let (target, upload) = match arg(args, "kind") {
                    "doc" => ("application/vnd.google-apps.document", "text/plain"),
                    "sheet" => ("application/vnd.google-apps.spreadsheet", "text/csv"),
                    _ => ("text/plain", "text/plain"),
                };
                let mut meta = json!({ "name": arg(args, "name"), "mimeType": target });
                if !arg(args, "folder_id").is_empty() {
                    meta["parents"] = json!([arg(args, "folder_id")]);
                }
                let boundary = format!("outpost{}", uuid::Uuid::new_v4().simple());
                let body = format!(
                    "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: {upload}; charset=UTF-8\r\n\r\n{}\r\n--{boundary}--\r\n",
                    arg(args, "content")
                );
                let v = api
                    .json(
                        client
                            .post("https://www.googleapis.com/upload/drive/v3/files")
                            .query(&[("uploadType", "multipart"), ("fields", "id,name,webViewLink"), ("supportsAllDrives", "true")])
                            .header("Content-Type", format!("multipart/related; boundary={boundary}"))
                            .body(body),
                    )
                    .await?;
                Ok(format!(
                    "Created {} (id {}) {}",
                    v["name"].as_str().unwrap_or("it"),
                    v["id"].as_str().unwrap_or("?"),
                    v["webViewLink"].as_str().unwrap_or("")
                ))
            }
            "docs_read" => {
                let v = api.json(api.get(&format!("https://docs.googleapis.com/v1/documents/{}", enc(arg(args, "document_id"))))).await?;
                Ok(format!("{}\n\n{}", v["title"].as_str().unwrap_or("Document"), cut(&doc_text(&v))))
            }
            "docs_append" => {
                let text = arg(args, "text");
                let body = json!({ "requests": [{ "insertText": { "endOfSegmentLocation": {}, "text": format!("\n{text}") } }] });
                api.json(client.post(format!("https://docs.googleapis.com/v1/documents/{}:batchUpdate", enc(arg(args, "document_id")))).json(&body)).await?;
                Ok("Added to the end of the document.".into())
            }
            "docs_replace" => {
                let find = args.get("find").and_then(Value::as_str).unwrap_or("");
                if find.is_empty() {
                    return Ok("Give the text to find.".into());
                }
                let body = json!({ "requests": [{ "replaceAllText": {
                    "containsText": { "text": find, "matchCase": true },
                    "replaceText": args.get("replace").and_then(Value::as_str).unwrap_or("")
                } }] });
                let v = api
                    .json(client.post(format!("https://docs.googleapis.com/v1/documents/{}:batchUpdate", enc(arg(args, "document_id")))).json(&body))
                    .await?;
                let n = v["replies"][0]["replaceAllText"]["occurrencesChanged"].as_u64().unwrap_or(0);
                Ok(if n == 0 { "That text wasn't found in the document; nothing changed.".into() } else { format!("Replaced {n} place(s).") })
            }
            "sheets_read" => {
                let id = enc(arg(args, "spreadsheet_id"));
                let meta = api
                    .json(api.get(&format!("https://sheets.googleapis.com/v4/spreadsheets/{id}")).query(&[("fields", "properties.title,sheets.properties(title,gridProperties)")]))
                    .await?;
                let tabs: Vec<String> = meta["sheets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| {
                        let p = &s["properties"];
                        format!(
                            "{} ({} rows x {} columns)",
                            p["title"].as_str().unwrap_or("?"),
                            p["gridProperties"]["rowCount"].as_u64().unwrap_or(0),
                            p["gridProperties"]["columnCount"].as_u64().unwrap_or(0)
                        )
                    })
                    .collect();
                let first = meta["sheets"][0]["properties"]["title"].as_str().unwrap_or("Sheet1").replace('\'', "''");
                let range = if arg(args, "range").is_empty() { format!("'{first}'!A1:Z200") } else { arg(args, "range").to_string() };
                let v = api
                    .json(api.get(&format!("https://sheets.googleapis.com/v4/spreadsheets/{id}/values/{}", enc(&range))).query(&[("valueRenderOption", "FORMATTED_VALUE")]))
                    .await?;
                let rows: Vec<String> = v["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| r.as_array().map(|c| c.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(" | ")).unwrap_or_default())
                    .collect();
                Ok(cut(&format!(
                    "{}\nTabs: {}\n\n{} ({} rows):\n{}",
                    meta["properties"]["title"].as_str().unwrap_or("Spreadsheet"),
                    tabs.join(", "),
                    v["range"].as_str().unwrap_or(&range),
                    rows.len(),
                    rows.join("\n")
                )))
            }
            "sheets_write" | "sheets_append" => {
                let id = enc(arg(args, "spreadsheet_id"));
                let range = enc(arg(args, "range"));
                let body = json!({ "values": args.get("values").cloned().unwrap_or(json!([])) });
                let v = if name == "sheets_write" {
                    api.json(client.put(format!("https://sheets.googleapis.com/v4/spreadsheets/{id}/values/{range}")).query(&[("valueInputOption", "USER_ENTERED")]).json(&body))
                        .await?
                } else {
                    api.json(
                        client
                            .post(format!("https://sheets.googleapis.com/v4/spreadsheets/{id}/values/{range}:append"))
                            .query(&[("valueInputOption", "USER_ENTERED"), ("insertDataOption", "INSERT_ROWS")])
                            .json(&body),
                    )
                    .await?
                };
                let u = if name == "sheets_write" { &v } else { &v["updates"] };
                Ok(format!("Wrote {} cell(s) in {}.", u["updatedCells"].as_u64().unwrap_or(0), u["updatedRange"].as_str().unwrap_or("the sheet")))
            }
            "gmail_search" => {
                let max = args.get("max").and_then(Value::as_u64).unwrap_or(10).clamp(1, 25);
                let v = api
                    .json(api.get("https://gmail.googleapis.com/gmail/v1/users/me/messages").query(&[("q", arg(args, "query").to_string()), ("maxResults", max.to_string())]))
                    .await?;
                let ids: Vec<String> = v["messages"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
                if ids.is_empty() {
                    return Ok("No emails found.".into());
                }
                let mut out = Vec::new();
                for id in ids {
                    let m = api
                        .json(api.get(&format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}")).query(&[
                            ("format", "metadata"),
                            ("metadataHeaders", "From"),
                            ("metadataHeaders", "Subject"),
                            ("metadataHeaders", "Date"),
                        ]))
                        .await?;
                    let h = |k: &str| {
                        m["payload"]["headers"].as_array().into_iter().flatten().find(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(k))).and_then(|x| x["value"].as_str()).unwrap_or("").to_string()
                    };
                    out.push(format!("- id {id} | {} | from {} | {}\n  {}", h("Date"), h("From"), h("Subject"), m["snippet"].as_str().unwrap_or("")));
                }
                Ok(out.join("\n"))
            }
            "gmail_read" => {
                let id = enc(arg(args, "message_id"));
                let m = api.json(api.get(&format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}")).query(&[("format", "full")])).await?;
                let h = |k: &str| {
                    m["payload"]["headers"].as_array().into_iter().flatten().find(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(k))).and_then(|x| x["value"].as_str()).unwrap_or("").to_string()
                };
                Ok(cut(&format!(
                    "From: {}\nTo: {}\nDate: {}\nSubject: {}\n\n{}",
                    h("From"),
                    h("To"),
                    h("Date"),
                    h("Subject"),
                    mail_text(&m["payload"])
                )))
            }
            "gmail_draft" => {
                let mut to = arg(args, "to").to_string();
                let mut subject = arg(args, "subject").to_string();
                let mut thread = None;
                let mut in_reply = None;
                if !arg(args, "reply_to").is_empty() {
                    let m = api
                        .json(api.get(&format!("https://gmail.googleapis.com/gmail/v1/users/me/messages/{}", enc(arg(args, "reply_to")))).query(&[
                            ("format", "metadata"),
                            ("metadataHeaders", "From"),
                            ("metadataHeaders", "Subject"),
                            ("metadataHeaders", "Message-ID"),
                        ]))
                        .await?;
                    let h = |k: &str| {
                        m["payload"]["headers"].as_array().into_iter().flatten().find(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(k))).and_then(|x| x["value"].as_str()).unwrap_or("").to_string()
                    };
                    if to.is_empty() {
                        to = h("From");
                    }
                    if subject.is_empty() {
                        let s = h("Subject");
                        subject = if s.to_lowercase().starts_with("re:") { s } else { format!("Re: {s}") };
                    }
                    thread = m["threadId"].as_str().map(str::to_string);
                    in_reply = Some(h("Message-ID")).filter(|s| !s.is_empty());
                }
                if to.is_empty() {
                    return Ok("Who is it to? Give an address in to.".into());
                }
                let mut message = json!({ "raw": raw_mail(&to, &subject, arg(args, "body"), in_reply.as_deref()) });
                if let Some(t) = thread {
                    message["threadId"] = json!(t);
                }
                let v = api.json(client.post("https://gmail.googleapis.com/gmail/v1/users/me/drafts").json(&json!({ "message": message }))).await?;
                Ok(format!("Saved as a draft in Gmail (draft {}), to {to}: \"{subject}\". It hasn't been sent; the user sends it from Gmail.", v["id"].as_str().unwrap_or("?")))
            }
            "calendar_events" => {
                let from = if arg(args, "from").is_empty() { now_rfc3339(0) } else { range_time(arg(args, "from")) };
                let to = if arg(args, "to").is_empty() { now_rfc3339(7) } else { range_time(arg(args, "to")) };
                let max = args.get("max").and_then(Value::as_u64).unwrap_or(25).clamp(1, 100);
                let mut params: Vec<(&str, String)> = vec![
                    ("timeMin", from),
                    ("timeMax", to),
                    ("singleEvents", "true".into()),
                    ("orderBy", "startTime".into()),
                    ("maxResults", max.to_string()),
                ];
                if !arg(args, "query").is_empty() {
                    params.push(("q", arg(args, "query").to_string()));
                }
                let v = api.json(api.get("https://www.googleapis.com/calendar/v3/calendars/primary/events").query(&params)).await?;
                let items = v["items"].as_array().cloned().unwrap_or_default();
                if items.is_empty() {
                    return Ok("No events in that time.".into());
                }
                let zone = v["timeZone"].as_str().unwrap_or("");
                Ok(format!(
                    "Calendar time zone: {zone}\n{}",
                    items
                        .iter()
                        .map(|e| {
                            let t = |k: &str| e[k]["dateTime"].as_str().or(e[k]["date"].as_str()).unwrap_or("?").to_string();
                            format!(
                                "- {} → {}: {}{}",
                                t("start"),
                                t("end"),
                                e["summary"].as_str().unwrap_or("(no title)"),
                                e["location"].as_str().map(|l| format!(" @ {l}")).unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            }
            "calendar_create" => {
                let mut ev = json!({ "summary": arg(args, "summary"), "start": when(arg(args, "start")), "end": when(arg(args, "end")) });
                for k in ["description", "location"] {
                    if !arg(args, k).is_empty() {
                        ev[k] = json!(arg(args, k));
                    }
                }
                let v = api.json(client.post("https://www.googleapis.com/calendar/v3/calendars/primary/events").json(&ev)).await?;
                Ok(format!("Added \"{}\" to the calendar. {}", arg(args, "summary"), v["htmlLink"].as_str().unwrap_or("")))
            }
            other => Ok(format!("There's no Google tool called {other}.")),
        }
    }
    .await;
    match result {
        Ok(text) => Ok(CallResult { text, is_error: false }),
        // The app handles a sign-in; anything else is the model's to explain.
        Err(e) if crate::mcp_client::needs_sign_in(&e) => Err(e),
        Err(e) => Ok(CallResult { text: e, is_error: true }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn settings_choose_tools_and_scopes() {
        let read = on(&["drive", "gmail"]);
        let names: Vec<String> = tools(&read).into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["drive_search", "drive_read", "gmail_search", "gmail_read"]);
        let s = scopes(&read);
        assert!(s.contains("drive.readonly") && s.contains("gmail.readonly"));
        assert!(!s.contains("gmail.compose") && !s.contains("drive.file"));
        let all = on(&["drive", "docs", "sheets", "gmail", "calendar", "write"]);
        assert_eq!(tools(&all).len(), 14);
        assert!(scopes(&all).contains("calendar.events") && scopes(&all).contains("gmail.compose"));
        // A read-only sign-in doesn't cover changes.
        assert!(covers(&scopes(&read), &read));
        assert!(!covers(&scopes(&read), &all));
        assert!(covers(&scopes(&all), &on(&["docs", "write"])));
    }

    #[test]
    fn docs_and_mail_read_as_text() {
        let doc = json!({ "body": { "content": [
            { "paragraph": { "elements": [{ "textRun": { "content": "Shopping\n" } }] } },
            { "table": { "tableRows": [{ "tableCells": [
                { "content": [{ "paragraph": { "elements": [{ "textRun": { "content": "Milk\n" } }] } }] },
                { "content": [{ "paragraph": { "elements": [{ "textRun": { "content": "2\n" } }] } }] }
            ] }] } }
        ] } });
        assert_eq!(doc_text(&doc), "Shopping\n| Milk | 2 |\n");
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        let plain = json!({ "mimeType": "multipart/alternative", "parts": [
            { "mimeType": "text/html", "body": { "data": b64("<p>Hi</p>") } },
            { "mimeType": "text/plain", "body": { "data": b64("Hi Dan,\nSee you Friday.") } }
        ] });
        assert_eq!(mail_text(&plain), "Hi Dan,\nSee you Friday.");
        let html = json!({ "mimeType": "text/html", "body": { "data": b64("<style>p{}</style><p>Hello&nbsp;there</p><div>Bye &amp; thanks</div>") } });
        assert_eq!(mail_text(&html), "Hello there\nBye & thanks");
    }

    #[test]
    fn drafts_are_proper_emails() {
        let raw = raw_mail("sj@example.com", "Dinner – Friday", "Line one\nLine two", Some("<abc@mail>"));
        let text = String::from_utf8(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw).unwrap()).unwrap();
        assert!(text.starts_with("To: sj@example.com\r\nSubject: =?UTF-8?B?"));
        assert!(text.contains("In-Reply-To: <abc@mail>\r\n"));
        assert!(text.ends_with("\r\n\r\nLine one\r\nLine two"));
        assert_eq!(drive_quote("Dan's notes"), "'Dan\\'s notes'");
        assert_eq!(when("2026-11-03"), json!({ "date": "2026-11-03" }));
        assert_eq!(when("2026-11-03T10:00:00+11:00"), json!({ "dateTime": "2026-11-03T10:00:00+11:00" }));
    }
}
