//! Chatting with documents: reading them, and finding what's relevant to a message.
//!
//! - **Reading** (in the background when a document is added): the text comes out of the file
//!   (PDF pages, a Word file's paragraphs, text as it is), is split into passages of a few
//!   paragraphs that never cross a page (so a reply can say which page), and each passage
//!   gets an embedding from the same model memory uses, when one is installed.
//! - **Using**: when a chat (or its persona) has documents, each message gets the passages
//!   most like it (by embedding, else by its words), numbered, in the system prompt, and the
//!   reply cites them ("[2]"). Small documents go in whole. The reply keeps which passages it
//!   cited, so the apps can show "Handbook.pdf, page 3" under it.

use std::io::Read;

use outpost_core::documents::{self, Chunk, NewChunk, Source};
use outpost_core::personas::Persona;

use crate::AppState;

/// Aim for passages about this long (characters)...
const TARGET: usize = 900;
/// ...and never longer than this.
const MAX_CHUNK: usize = 1400;
/// Text carried over from the end of one passage to the start of the next.
const OVERLAP: usize = 150;
/// Documents this short (all together) go into the prompt whole.
const WHOLE: usize = 12_000;
/// Passages given to the model for a message.
const PASSAGES: usize = 6;
/// Passages embedded per batch.
const BATCH: usize = 16;
/// Passages embedded while answering a message (the rest wait for the next time).
const EMBED_WHILE_ANSWERING: usize = 256;

/// The text of a document, page by page (one part for files without pages).
#[derive(Debug, Default)]
pub struct Extracted {
    pub pages: Option<u32>,
    /// (page, text)
    pub parts: Vec<(Option<u32>, String)>,
}

impl Extracted {
    fn chars(&self) -> usize {
        self.parts.iter().map(|(_, t)| t.trim().chars().count()).sum()
    }
}

/// Take the text out of a document.
pub fn extract(mime: &str, data: &[u8]) -> Result<Extracted, String> {
    let out = match mime {
        "application/pdf" => {
            // A damaged PDF can make the reader panic: that's a failed document, not a crash.
            let pages = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text_from_mem_by_pages(data)))
                .map_err(|_| "This PDF couldn't be read (it may be damaged).".to_string())?
                .map_err(|e| format!("This PDF couldn't be read: {e}"))?;
            let n = pages.len() as u32;
            let doc = Extracted {
                pages: Some(n),
                parts: pages.into_iter().enumerate().map(|(i, t)| (Some(i as u32 + 1), t)).collect(),
            };
            if n > 0 && doc.chars() < 20 * n as usize {
                return Err("This PDF has almost no text in it: it's probably scanned pages (pictures of text), which Outpost can't read yet.".into());
            }
            doc
        }
        m if m.ends_with("wordprocessingml.document") => Extracted { pages: None, parts: vec![(None, docx_text(data)?)] },
        "text/html" => Extracted { pages: None, parts: vec![(None, html_text(&text_of(data)))] },
        _ => Extracted { pages: None, parts: vec![(None, text_of(data))] },
    };
    if out.chars() == 0 {
        return Err("No text was found in this document.".into());
    }
    if out.chars() > documents::MAX_CHARS {
        return Err(format!(
            "This document has more text than Outpost keeps for one document (about {} pages).",
            documents::MAX_CHARS / 3000
        ));
    }
    Ok(out)
}

/// Text in UTF-8 (or UTF-16 with a byte-order mark), without a BOM.
fn text_of(data: &[u8]) -> String {
    if data.starts_with(&[0xFF, 0xFE]) || data.starts_with(&[0xFE, 0xFF]) {
        let le = data[0] == 0xFF;
        let units: Vec<u16> = data[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let text = String::from_utf8_lossy(data);
    text.trim_start_matches('\u{feff}').to_string()
}

/// A Word file's text: its paragraphs (from `word/document.xml`), one per line.
fn docx_text(data: &[u8]) -> Result<String, String> {
    let bad = || "This Word file couldn't be read (only .docx files are supported, not old .doc).".to_string();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|_| bad())?;
    let mut xml = String::new();
    {
        let mut part = zip.by_name("word/document.xml").map_err(|_| bad())?;
        // Refuse a "zip bomb": no real document.xml is this big.
        part.by_ref().take(200 * 1024 * 1024).read_to_string(&mut xml).map_err(|_| bad())?;
    }
    Ok(word_xml_text(&xml))
}

/// The text in WordprocessingML: `<w:t>` runs, with paragraphs, breaks and tabs.
pub fn word_xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let tag = &rest[start + 1..start + end];
        let after = &rest[start + end + 1..];
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        match name {
            "w:t" if !tag.starts_with('/') && !tag.ends_with('/') => {
                let close = after.find("</w:t>").unwrap_or(after.len());
                out.push_str(&unescape(&after[..close]));
                rest = &after[close..];
                continue;
            }
            "w:p" if tag.starts_with('/') => out.push('\n'),
            "w:br" | "w:cr" => out.push('\n'),
            "w:tab" if !tag.starts_with('/') => out.push('\t'),
            _ => {}
        }
        rest = after;
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Readable text from HTML: no scripts or styles, block tags as line breaks.
pub fn html_text(html: &str) -> String {
    // ASCII only, so byte positions match the original.
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while i < html.len() {
        let Some(off) = html[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        out.push_str(&html[i..i + off]);
        let start = i + off;
        let Some(end) = html[start..].find('>').map(|e| start + e) else { break };
        let tag = lower[start + 1..end].trim_start_matches('/');
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        i = end + 1;
        if (name == "script" || name == "style") && !lower[start + 1..].starts_with('/') {
            // Skip to the closing tag.
            let close = format!("</{name}");
            i = lower[i..].find(&close).map(|c| i + c).unwrap_or(html.len());
            continue;
        }
        if matches!(name, "p" | "br" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article") {
            out.push('\n');
        }
    }
    unescape(&out)
}

/// Split a document into passages: a few paragraphs each, never across a page, long
/// paragraphs split at sentences, each starting with a little of the one before.
pub fn chunk(doc: &Extracted) -> Vec<NewChunk> {
    let mut out = Vec::new();
    for (page, text) in &doc.parts {
        let mut current = String::new();
        let mut carry = String::new();
        let flush = |current: &mut String, carry: &mut String, out: &mut Vec<NewChunk>| {
            let body = current.trim();
            if body.chars().count() >= 2 {
                let text = if carry.is_empty() { body.to_string() } else { format!("…{carry} {body}") };
                *carry = tail(body, OVERLAP);
                out.push(NewChunk { page: *page, text });
            }
            current.clear();
        };
        for para in paragraphs(text) {
            for piece in split_long(&para) {
                if !current.is_empty() && current.chars().count() + piece.chars().count() > TARGET {
                    flush(&mut current, &mut carry, &mut out);
                }
                if !current.is_empty() {
                    current.push_str("\n\n");
                }
                current.push_str(&piece);
            }
        }
        flush(&mut current, &mut carry, &mut out);
    }
    out
}

/// Paragraphs: blank-line separated, with spacing tidied.
fn paragraphs(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    let mut para = String::new();
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !para.is_empty() {
                out.push(std::mem::take(&mut para));
            }
            continue;
        }
        if !para.is_empty() {
            para.push(' ');
        }
        para.push_str(&line);
        // A very long run of lines with no blank line (common in PDFs) is still split.
        if para.chars().count() > TARGET {
            out.push(std::mem::take(&mut para));
        }
    }
    if !para.is_empty() {
        out.push(para);
    }
    out
}

/// A paragraph longer than a passage, split at sentence ends (or, failing that, words).
fn split_long(para: &str) -> Vec<String> {
    if para.chars().count() <= MAX_CHUNK {
        return vec![para.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in para.split(' ') {
        if !current.is_empty() && current.chars().count() + word.chars().count() + 1 > TARGET {
            let ends_sentence = current.ends_with(['.', '!', '?', ';', ':']);
            if ends_sentence || current.chars().count() + word.chars().count() + 1 > MAX_CHUNK {
                out.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// About the last `n` characters, starting at a word.
fn tail(text: &str, n: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= n {
        return String::new();
    }
    let s: String = chars[chars.len() - n..].iter().collect();
    match s.find(' ') {
        Some(i) => s[i + 1..].to_string(),
        None => s,
    }
}

// ------------------------------------------------------------------ in the background

/// Read a document that was just added (or left half-read by a restart): text, passages,
/// embeddings. Failures are recorded on the document.
pub async fn read(state: AppState, id: String) {
    let files = state.config.files_dir();
    let (doc, path) = match documents::file_of(&state.pool, &files, &id).await {
        Ok(found) => found,
        Err(_) => return,
    };
    let data = match tokio::fs::read(&path).await {
        Ok(d) => d,
        Err(e) => {
            let _ = documents::set_failed(&state.pool, &id, &format!("The file is missing: {e}")).await;
            return;
        }
    };
    let mime = doc.mime.clone();
    let extracted = tokio::task::spawn_blocking(move || extract(&mime, &data))
        .await
        .unwrap_or_else(|_| Err("Reading this document failed.".into()));
    let extracted = match extracted {
        Ok(x) => x,
        Err(e) => {
            tracing::info!(document = %doc.name, error = %e, "couldn't read a document");
            let _ = documents::set_failed(&state.pool, &id, &e).await;
            return;
        }
    };
    let passages = chunk(&extracted);
    if let Err(e) = documents::set_chunks(&state.pool, &id, extracted.pages, &passages).await {
        let _ = documents::set_failed(&state.pool, &id, &format!("Couldn't save its text: {e}")).await;
        return;
    }
    tracing::info!(document = %doc.name, passages = passages.len(), "read a document");
    if let Some(model) = crate::memory::embedding_model(&state).await {
        if let Ok(mut all) = documents::chunks(&state.pool, &[id.clone()], Some(&model)).await {
            embed_missing(&state, &model, &mut all, usize::MAX).await;
        }
    }
}

/// Read again what a restart interrupted.
pub async fn resume(state: AppState) {
    for (id, _) in documents::unread(&state.pool).await.unwrap_or_default() {
        read(state.clone(), id).await;
    }
}

/// Embed passages that have no embedding from `model` yet (up to `limit`), in place.
async fn embed_missing(state: &AppState, model: &str, chunks: &mut [Chunk], limit: usize) {
    let missing: Vec<usize> = chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.embedding.is_none())
        .map(|(i, _)| i)
        .take(limit)
        .collect();
    for batch in missing.chunks(BATCH) {
        let texts: Vec<String> = batch.iter().map(|&i| chunks[i].text.clone()).collect();
        let Some(vectors) = crate::memory::embed(state, model, &texts).await else { return };
        for (&i, v) in batch.iter().zip(vectors) {
            let c = &mut chunks[i];
            let _ = documents::set_chunk_embedding(&state.pool, &c.document_id, c.idx, model, &v).await;
            c.embedding = Some(outpost_core::memory::normalized(&v));
        }
    }
}

// ------------------------------------------------------------------ answering

/// What a message gets from its chat's documents: a section for the system prompt, and
/// the passages in it (numbered as the section numbers them).
#[derive(Debug, Clone)]
pub struct Context {
    pub section: String,
    pub sources: Vec<Source>,
}

/// Passages from the documents of this chat and its persona that fit `query` (the user's
/// message; `previous`, the message before it, helps with short follow-ups).
pub async fn context(
    state: &AppState,
    chat_id: &str,
    persona: Option<&Persona>,
    query: &str,
    previous: Option<&str>,
) -> Option<Context> {
    let mut docs = documents::for_chat(&state.pool, chat_id).await.unwrap_or_default();
    if let Some(p) = persona {
        for d in documents::for_persona(&state.pool, &p.id).await.unwrap_or_default() {
            if !docs.iter().any(|x| x.id == d.id) {
                docs.push(d);
            }
        }
    }
    let reading: Vec<String> = docs.iter().filter(|d| d.status == "reading").map(|d| d.name.clone()).collect();
    docs.retain(|d| d.status == "ready");
    if docs.is_empty() {
        return (!reading.is_empty()).then(|| Context {
            section: format!(
                "The user has shared documents that are still being read ({}). If they ask about them, say they'll be ready in a moment.",
                reading.join(", ")
            ),
            sources: Vec::new(),
        });
    }
    let ids: Vec<String> = docs.iter().map(|d| d.id.clone()).collect();
    let total: i64 = docs.iter().map(|d| d.chars).sum();
    let model = crate::memory::embedding_model(state).await;
    let mut all = documents::chunks(&state.pool, &ids, model.as_deref()).await.ok()?;
    if all.is_empty() {
        return None;
    }
    let picked: Vec<Chunk> = if total as usize <= WHOLE {
        all
    } else {
        // A short follow-up ("and the second one?") is searched with the message before it.
        let q = match previous {
            Some(p) if query.chars().count() < 60 => format!("{p}\n{query}"),
            _ => query.to_string(),
        };
        let mut qv = None;
        if let Some(m) = model.as_deref() {
            embed_missing(state, m, &mut all, EMBED_WHILE_ANSWERING).await;
            qv = crate::memory::embed(state, m, &[q.clone()]).await.and_then(|mut v| v.pop());
        }
        documents::best(&all, &q, qv.as_deref(), PASSAGES).into_iter().cloned().collect()
    };
    Some(section(&picked, docs.len(), &reading))
}

fn section(picked: &[Chunk], doc_count: usize, reading: &[String]) -> Context {
    let mut s = String::from(
        "The user has shared documents with you. Passages from them that may help with this message are below, numbered. \
         Use them to answer, and put the passage's number in square brackets after what you took from it, like [2]. \
         If they don't contain the answer, say so plainly instead of guessing.",
    );
    if doc_count > 1 {
        s.push_str(" Say which document something comes from when it matters.");
    }
    if !reading.is_empty() {
        s.push_str(&format!(" ({} still being read.)", reading.join(", ")));
    }
    let mut sources = Vec::new();
    for (i, c) in picked.iter().enumerate() {
        let n = i as u32 + 1;
        let place = match c.page {
            Some(p) => format!("{}, page {p}", c.document_name),
            None => c.document_name.clone(),
        };
        s.push_str(&format!("\n\n[{n}] {place}:\n{}", c.text));
        sources.push(Source { n, document_id: c.document_id.clone(), name: c.document_name.clone(), page: c.page });
    }
    Context { section: s, sources }
}

/// The sources a reply cites ("[2]"), or every one it was given if it cites none.
pub fn cited(reply: &str, sources: &[Source]) -> Vec<Source> {
    let used: Vec<Source> = sources.iter().filter(|s| reply.contains(&format!("[{}]", s.n))).cloned().collect();
    if used.is_empty() {
        sources.to_vec()
    } else {
        used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_files_keep_paragraphs_and_entities() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Fish &amp; chips</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve"> cost $5.</w:t></w:r></w:p><w:p><w:r><w:t>Open daily</w:t></w:r><w:r><w:br/><w:t>9 to 5</w:t></w:r></w:p></w:body></w:document>"#;
        assert_eq!(word_xml_text(xml), "Fish & chips\t cost $5.\nOpen daily\n9 to 5\n");
    }

    #[test]
    fn html_drops_scripts_and_keeps_blocks() {
        let html = "<html><head><style>p{}</style><script>var x = '<p>';</script></head><body><h1>Title</h1><p>One &amp; two</p><p>Three</p></body></html>";
        let t = html_text(html);
        assert!(!t.contains("var x"), "{t}");
        assert!(t.contains("Title") && t.contains("One & two") && t.contains("Three"), "{t}");
    }

    #[test]
    fn passages_stay_on_their_page_and_overlap() {
        let long: String = (0..60).map(|i| format!("Sentence number {i} is here.")).collect::<Vec<_>>().join(" ");
        let doc = Extracted {
            pages: Some(2),
            parts: vec![(Some(1), format!("Intro.\n\n{long}")), (Some(2), "Page two text.".into())],
        };
        let chunks = chunk(&doc);
        assert!(chunks.len() >= 3, "{chunks:?}");
        assert!(chunks.iter().all(|c| c.text.chars().count() <= MAX_CHUNK + OVERLAP + 4));
        assert_eq!(chunks.last().unwrap().page, Some(2));
        assert_eq!(chunks.last().unwrap().text, "Page two text.", "no carry across pages");
        assert!(chunks.iter().any(|c| c.page == Some(1) && c.text.starts_with('…')), "carries on from the passage before: {chunks:?}");
        assert!(chunks.iter().filter(|c| c.page == Some(1)).count() >= 2);
    }

    #[test]
    fn text_files_and_boms() {
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Hi".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(text_of(&utf16), "Hi");
        assert_eq!(text_of("\u{feff}Hello".as_bytes()), "Hello");
        assert!(extract("text/plain", b"   \n ").is_err(), "nothing to read");
        let e = extract("text/markdown", b"# Notes\n\nBuy milk.").unwrap();
        assert_eq!(chunk(&e).len(), 1);
    }

    #[test]
    fn only_cited_sources_are_kept() {
        let s = |n| Source { n, document_id: "d".into(), name: "a.pdf".into(), page: Some(n as i64) };
        let all = vec![s(1), s(2), s(3)];
        assert_eq!(cited("It's two years [2].", &all), vec![s(2)]);
        assert_eq!(cited("No citations here.", &all), all);
    }
}
