//! Read-only presentation of local history. Tool calls, reasoning, environment
//! context and injected instructions are never used as conversation headlines.
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub text: String,
    pub timestamp: Option<String>,
}

fn content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.into();
    }
    value
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn message_content(value: &Value, role: &str) -> String {
    if role != "user" || !value.is_array() {
        return content(value);
    }
    value
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str).and_then(user_text))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn user_text(text: &str) -> Option<String> {
    let mut text = text.trim();
    for marker in ["## My request:", "# My request:"] {
        if let Some((_, request)) = text.split_once(marker) {
            text = request.trim();
            break;
        }
    }
    for tag in [
        "environment_context",
        "user_instructions",
        "permissions instructions",
    ] {
        if text.starts_with(&format!("<{tag}>")) {
            let (_, rest) = text.split_once(&format!("</{tag}>"))?;
            text = rest.trim();
        }
    }
    let lower = text.to_ascii_lowercase();
    if text.is_empty()
        || lower.starts_with("# agents.md instructions for")
        || lower.starts_with("<environment_context>")
        || lower.starts_with("<instructions>")
        || lower.starts_with("<system")
        || lower.starts_with("<developer")
        || lower.starts_with("<turn_aborted>")
        || lower.starts_with("<subagent_notification")
    {
        return None;
    }
    Some(text.into())
}

pub fn title(text: &str) -> Option<String> {
    let text = user_text(text)?;
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if uuid::Uuid::parse_str(&normalized).is_ok() {
        return None;
    }
    Some(normalized.chars().take(80).collect())
}

/// Prefer completed UI items, then legacy response messages, then legacy events.
/// This prevents the same message appearing twice in the two persisted forms.
pub fn messages(bytes: &[u8], limit: usize) -> (Vec<Message>, bool) {
    let mut channels = [Vec::new(), Vec::new(), Vec::new()];
    let mut counts = [0usize; 3];
    let mut shortened = [false; 3];
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let payload = &value["payload"];
        let (channel, role, text) = match (value["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("item_completed")) => {
                let item = &payload["item"];
                let role = match item["type"].as_str() {
                    Some("UserMessage") => "user",
                    Some("AgentMessage")
                        if item["channel"].is_null() || item["channel"] == "final" =>
                    {
                        "assistant"
                    }
                    _ => continue,
                };
                let text = if item["text"].is_string() {
                    content(&item["text"])
                } else {
                    message_content(&item["content"], role)
                };
                (0, role, text)
            }
            (Some("response_item"), Some("message")) => {
                let role = match payload["role"].as_str() {
                    Some("user") => "user",
                    Some("assistant")
                        if payload["channel"].is_null() || payload["channel"] == "final" =>
                    {
                        "assistant"
                    }
                    _ => continue,
                };
                (1, role, message_content(&payload["content"], role))
            }
            (Some("event_msg"), Some("user_message")) => (2, "user", content(&payload["message"])),
            (Some("event_msg"), Some("agent_message")) => {
                (2, "assistant", content(&payload["message"]))
            }
            _ => continue,
        };
        let text = if role == "user" {
            user_text(&text)
        } else {
            Some(text.trim().into()).filter(|s: &String| !s.is_empty())
        };
        let Some(text) = text else { continue };
        counts[channel] += 1;
        if channels[channel].len() >= limit {
            continue;
        }
        shortened[channel] |= text.chars().count() > 6000;
        channels[channel].push(Message {
            role: role.into(),
            text: text.chars().take(6000).collect(),
            timestamp: value["timestamp"].as_str().map(str::to_string),
        });
    }
    let channel = counts.iter().position(|n| *n > 0).unwrap_or(0);
    (
        std::mem::take(&mut channels[channel]),
        counts[channel] > limit || shortened[channel],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn lines(values: Vec<Value>) -> Vec<u8> {
        values
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>()
            .into_bytes()
    }
    fn legacy(role: &str, text: &str) -> Value {
        json!({"type":"response_item", "payload":{"type":"message", "role":role, "content":[{"type":"text","text":text}]}})
    }

    #[test]
    fn removes_injected_context_but_keeps_the_real_question_on_both_platforms() {
        for path in ["E:\\代码中心\\cods-app", "/Users/dev/cods-app"] {
            let raw = lines(vec![legacy("user", &format!("# AGENTS.md instructions for {path}\n<INSTRUCTIONS>rules</INSTRUCTIONS>")),
                legacy("user", "<environment_context>private settings</environment_context>"),
                legacy("user", "# Files mentioned by the user:\nprivate.txt\n## My request:\n四要素核验通过后，展示凭证编号。"), legacy("assistant", "可以展示凭证编号及验证时间。")]);
            let (messages, truncated) = messages(&raw, 100);
            assert_eq!(messages.len(), 2);
            assert!(messages[0].text.starts_with("四要素"));
            assert_eq!(messages[1].role, "assistant");
            assert!(!truncated);
        }
        assert!(title("01a1429f-d4a6-7302-bda4-c7512877f92c").is_none());
        assert_eq!(
            title("请检查 AGENTS.md 的规则").as_deref(),
            Some("请检查 AGENTS.md 的规则")
        );
    }

    #[test]
    fn prefers_completed_visible_items_and_excludes_reasoning_tools_and_duplicates() {
        let raw = lines(vec![
            legacy("user", "duplicate"),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"text":"hidden reasoning"}]}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"text":"实际问题"}]}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","text":"实际回答"}}}),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","text":"private tool output"}}}),
        ]);
        let (items, _) = messages(&raw, 100);
        assert_eq!(
            items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["实际问题", "实际回答"]
        );
        let (items, truncated) = messages(&raw, 1);
        assert_eq!(items.len(), 1);
        assert!(truncated);
    }

    #[test]
    fn retains_real_request_when_it_is_a_separate_part_after_injected_instructions() {
        let raw = lines(vec![
            json!({"type":"response_item", "payload":{"type":"message", "role":"user", "content":[
                {"text":"# AGENTS.md instructions for /project\n<INSTRUCTIONS>rules</INSTRUCTIONS>"},
                {"text":"请修改验证码的交互"}
            ]}}),
        ]);
        let (items, _) = messages(&raw, 100);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "请修改验证码的交互");
    }
}
