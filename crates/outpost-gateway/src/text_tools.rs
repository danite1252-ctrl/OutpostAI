//! Built-in text tools (a tool you add from the library, "Count words"): exact counts that
//! models can't do reliably themselves. A persona reads a photo of a page, writes the text
//! out, and hands it here for the numbers.

use serde_json::{json, Value};

use crate::mcp_client::{CallResult, ToolInfo};

pub fn tools() -> Vec<ToolInfo> {
    vec![ToolInfo {
        name: "count_words".into(),
        description: "Count the words, characters, lines and paragraphs in a piece of text exactly, the way a word processor \
                      does. Always use this for a word count (for example of text you've read from a picture) instead of \
                      counting yourself, and pass the text exactly as written."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": { "text": { "type": "string", "description": "The text to count, exactly as written" } },
            "required": ["text"],
        }),
        read_only: Some(true),
    }]
}

/// The counts, as a word processor gives them.
#[derive(Debug, PartialEq)]
pub struct Counts {
    /// Runs of non-space characters with at least one letter or digit.
    pub words: usize,
    /// Every run of non-space characters, stand-alone dashes and ampersands too (Word counts these).
    pub words_with_symbols: usize,
    pub characters: usize,
    pub characters_no_spaces: usize,
    pub lines: usize,
    pub paragraphs: usize,
}

pub fn count(text: &str) -> Counts {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    Counts {
        words: tokens.iter().filter(|t| t.chars().any(char::is_alphanumeric)).count(),
        words_with_symbols: tokens.len(),
        characters: text.chars().count(),
        characters_no_spaces: text.chars().filter(|c| !c.is_whitespace()).count(),
        lines: text.lines().filter(|l| !l.trim().is_empty()).count(),
        paragraphs: text.split("\n\n").filter(|p| !p.trim().is_empty()).count(),
    }
}

pub fn call(name: &str, args: &Value) -> Result<CallResult, String> {
    match name {
        "count_words" => {
            let text = args["text"].as_str().unwrap_or("");
            if text.trim().is_empty() {
                return Ok(CallResult { text: "There's no text to count.".into(), is_error: true });
            }
            let c = count(text);
            let mut out = format!("Words: {}", c.words);
            if c.words_with_symbols != c.words {
                out.push_str(&format!(
                    " ({} counting stand-alone symbols like – or &, as Microsoft Word does)",
                    c.words_with_symbols
                ));
            }
            out.push_str(&format!(
                "\nCharacters: {} ({} without spaces)\nLines: {}\nParagraphs: {}",
                c.characters, c.characters_no_spaces, c.lines, c.paragraphs
            ));
            Ok(CallResult { text: out, is_error: false })
        }
        other => Err(format!("There's no text tool called {other}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_counted_like_a_word_processor() {
        let c = count("Dear Sam,\n\nI'm applying for the role – it's a great fit & I'm keen.\nThanks!\n");
        assert_eq!(c.words, 14);
        assert_eq!(c.words_with_symbols, 16);
        assert_eq!(c.lines, 3);
        assert_eq!(c.paragraphs, 2);
        assert_eq!(count("well-known e-mail 2026").words, 3);
        assert!(call("count_words", &json!({"text": "  "})).unwrap().is_error);
        assert!(call("count_words", &json!({"text": "one two"})).unwrap().text.starts_with("Words: 2\n"));
    }
}
