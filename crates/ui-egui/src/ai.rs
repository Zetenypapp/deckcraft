//! File › Settings › AI (#732): bring your own model. Any server that speaks the OpenAI chat
//! completions API (Ollama, LM Studio, LocalAI, vLLM, OpenAI itself…) at an address the user
//! picks, with their own key and model. DeckCraft only builds the request and reads the reply;
//! the host sends it (`Services::ai_send`), so the web build and the engine stay offline.
//!
//! Draft slides from a prompt asks the model for a Markdown outline and builds it with
//! `slide.fromOutline`, so the slides are ordinary, undoable slides.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The address, model and key size limits: longer is a paste gone wrong.
const MAX_FIELD: usize = 2048;
/// The longest prompt `ai.draftSlides` sends.
pub const MAX_PROMPT: usize = 8000;

/// Where requests go, saved with the other settings. The key is not here: it is kept apart from
/// the settings and their backups (`SlideApp::ai_key`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiSettings {
    /// The API's base address, up to `/v1` (`/chat/completions` is added).
    pub endpoint: String,
    pub model: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        // Ollama on this computer: free, private, and needs no key.
        AiSettings { endpoint: PRESETS[0].1.into(), model: PRESETS[0].2.into() }
    }
}

/// Ready-made servers for the AI tab: (name, address, a model to start with).
pub const PRESETS: [(&str, &str, &str); 3] = [
    ("Ollama", "http://localhost:11434/v1", "llama3.2"),
    ("LM Studio", "http://localhost:1234/v1", ""),
    ("OpenAI", "https://api.openai.com/v1", "gpt-4o-mini"),
];

/// A request for the host to send: POST `body` to `url`, with `key` as a bearer token when
/// there is one. The reply is the response's JSON, or a sentence saying what went wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct AiCall {
    pub url: String,
    pub key: String,
    pub body: Value,
}

/// What a request in flight is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Test,
    Draft,
}

/// AI requests for the Settings tab: the one in flight, the last outcome and the prompt box.
#[derive(Default)]
pub struct State {
    pub pending: Option<(Job, std::sync::mpsc::Receiver<Result<Value, String>>)>,
    /// The last result to show under the buttons: `Ok` for good news, `Err` for a problem.
    pub outcome: Option<Result<String, String>>,
    pub prompt: String,
}

/// Check and tidy a field typed by the user.
pub fn field(name: &str, v: &str) -> Result<String, String> {
    let v = v.trim();
    if v.len() > MAX_FIELD {
        return Err(format!("`{name}` is too long"));
    }
    if v.chars().any(char::is_control) {
        return Err(format!("`{name}` can't contain line breaks or control characters"));
    }
    Ok(v.to_string())
}

/// The chat completions address for a base address. Accepts the base (`…/v1`) or the full
/// address someone pasted (`…/v1/chat/completions`).
pub fn chat_url(endpoint: &str) -> Result<String, String> {
    let e = endpoint.trim().trim_end_matches('/');
    if !(e.starts_with("http://") || e.starts_with("https://")) || e.len() <= "https://".len() {
        return Err("The AI address must start with http:// or https://, like http://localhost:11434/v1".into());
    }
    if e.ends_with("/chat/completions") { Ok(e.to_string()) } else { Ok(format!("{e}/chat/completions")) }
}

/// Is the key about to travel unencrypted to another computer? (http:// to anything but this one.)
pub fn key_in_the_clear(endpoint: &str, key: &str) -> bool {
    let e = endpoint.trim().to_ascii_lowercase();
    let Some(rest) = e.strip_prefix("http://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| h.trim_start_matches('[')).unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let local = host == "localhost" || host == "::1" || host.starts_with("127.");
    !key.trim().is_empty() && !local
}

const SYSTEM: &str = "You write presentation outlines in Markdown. Reply with the outline only: no introduction, \
no closing remarks and no code fences. Each slide is a level-2 heading (## Slide title) followed by \
3 to 6 short bullet points (- point); indent a bullet by two spaces to make it a sub-point. Keep \
bullets under 12 words. Write in the language of the request.";

fn request(settings: &AiSettings, key: &str, messages: Value, max_tokens: u32) -> Result<AiCall, String> {
    let url = chat_url(&settings.endpoint)?;
    let model = settings.model.trim();
    if model.is_empty() {
        return Err("Pick a model first: type its name in File › Settings › AI.".into());
    }
    Ok(AiCall { url, key: key.trim().to_string(), body: json!({"model": model, "messages": messages, "max_tokens": max_tokens, "stream": false}) })
}

/// A tiny request that shows whether the server, key and model work.
pub fn test_call(settings: &AiSettings, key: &str) -> Result<AiCall, String> {
    request(settings, key, json!([{"role": "user", "content": "Reply with the word OK."}]), 16)
}

/// The request for slides about `prompt`.
pub fn draft_call(settings: &AiSettings, key: &str, prompt: &str) -> Result<AiCall, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Say what the slides should be about.".into());
    }
    if prompt.len() > MAX_PROMPT {
        return Err(format!("That request is too long: keep it under {MAX_PROMPT} characters."));
    }
    let messages = json!([{"role": "system", "content": SYSTEM}, {"role": "user", "content": prompt}]);
    request(settings, key, messages, 2048)
}

/// The text of a chat completions reply, or the server's own error message.
pub fn reply_text(reply: &Value) -> Result<String, String> {
    if let Some(e) = reply.get("error") {
        let msg = e.get("message").and_then(Value::as_str).or(e.as_str()).unwrap_or("the server reported an error");
        return Err(format!("The AI server said: {msg}"));
    }
    let content = reply.pointer("/choices/0/message/content").and_then(Value::as_str);
    content.map(str::to_string).ok_or_else(|| "The AI server's reply has no text in it. Is the address an OpenAI-compatible API?".into())
}

/// The Markdown outline in a model's reply: thinking (`<think>…</think>`) and code fences
/// around the outline are dropped.
pub fn outline(reply: &str) -> String {
    let mut text = reply.to_string();
    while let Some(start) = text.find("<think>") {
        match text[start..].find("</think>") {
            Some(end) => text.replace_range(start..start + end + "</think>".len(), ""),
            // Thinking that never ended: there is no outline after it.
            None => text.truncate(start),
        }
    }
    text.lines().filter(|l| !l.trim_start().starts_with("```")).collect::<Vec<_>>().join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ollama() -> AiSettings {
        AiSettings::default()
    }

    #[test]
    fn the_chat_address_is_built_from_the_base_or_kept_when_whole() {
        assert_eq!(chat_url("http://localhost:11434/v1").unwrap(), "http://localhost:11434/v1/chat/completions");
        assert_eq!(chat_url(" https://api.openai.com/v1/ ").unwrap(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(chat_url("http://h/v1/chat/completions").unwrap(), "http://h/v1/chat/completions");
        assert!(chat_url("localhost:11434").is_err());
        assert!(chat_url("ftp://x").is_err());
        assert!(chat_url("https://").is_err());
    }

    #[test]
    fn a_key_sent_over_http_to_another_computer_is_flagged() {
        assert!(!key_in_the_clear("http://localhost:11434/v1", "sk-1"));
        assert!(!key_in_the_clear("http://127.0.0.1:1234/v1", "sk-1"));
        assert!(!key_in_the_clear("http://[::1]:1234/v1", "sk-1"));
        assert!(!key_in_the_clear("https://api.openai.com/v1", "sk-1"));
        assert!(!key_in_the_clear("http://192.168.1.5/v1", ""));
        assert!(key_in_the_clear("http://192.168.1.5:8000/v1", "sk-1"));
        assert!(key_in_the_clear("HTTP://ai.example.com/v1", "sk-1"));
        assert!(key_in_the_clear("http://localhost@evil.example/v1", "sk-1"));
    }

    #[test]
    fn a_draft_asks_the_model_for_a_markdown_outline() {
        let call = draft_call(&ollama(), " sk-1 ", "  The solar system  ").unwrap();
        assert_eq!(call.url, "http://localhost:11434/v1/chat/completions");
        assert_eq!(call.key, "sk-1");
        assert_eq!(call.body["model"], "llama3.2");
        assert_eq!(call.body["stream"], false);
        assert_eq!(call.body["messages"][0]["role"], "system");
        assert_eq!(call.body["messages"][1]["content"], "The solar system");
    }

    #[test]
    fn a_draft_needs_a_prompt_a_model_and_a_good_address() {
        assert!(draft_call(&ollama(), "", "  ").is_err());
        assert!(draft_call(&ollama(), "", &"x".repeat(MAX_PROMPT + 1)).is_err());
        let no_model = AiSettings { model: " ".into(), ..ollama() };
        assert!(draft_call(&no_model, "", "Bees").unwrap_err().contains("model"));
        let bad = AiSettings { endpoint: "localhost".into(), ..ollama() };
        assert!(test_call(&bad, "").unwrap_err().contains("http://"));
    }

    #[test]
    fn fields_are_trimmed_and_must_be_one_short_line() {
        assert_eq!(field("model", "  llama3  ").unwrap(), "llama3");
        assert!(field("model", "a\nb").is_err());
        assert!(field("key", &"k".repeat(MAX_FIELD + 1)).is_err());
    }

    #[test]
    fn the_reply_text_or_the_servers_error_is_read() {
        let ok = json!({"choices": [{"message": {"role": "assistant", "content": "## A\n- b"}}]});
        assert_eq!(reply_text(&ok).unwrap(), "## A\n- b");
        let err = json!({"error": {"message": "Incorrect API key provided"}});
        assert!(reply_text(&err).unwrap_err().contains("Incorrect API key"));
        assert!(reply_text(&json!({"error": "model not found"})).unwrap_err().contains("model not found"));
        assert!(reply_text(&json!({"choices": []})).is_err());
        assert!(reply_text(&json!("hello")).is_err());
    }

    #[test]
    fn thinking_and_code_fences_are_left_out_of_the_outline() {
        let reply = "<think>The user wants planets.</think>\n```markdown\n## Planets\n- Eight of them\n```\n";
        assert_eq!(outline(reply), "## Planets\n- Eight of them");
        assert_eq!(outline("<think>never ends"), "");
        assert_eq!(outline("<think>a</think>## One\n<think>b</think>- two"), "## One\n- two");
    }
}
