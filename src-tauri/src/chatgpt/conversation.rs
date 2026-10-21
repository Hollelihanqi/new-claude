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

/// The desktop client persists its own UI actions as fake user messages, for
/// example `<external_codex_apps_open_page>{"page_id":null}</external_…>`.
/// They carry `content_item_kinds` such as `additional_content.codex_apps_open_page`.
/// The client never renders them as a question, so neither should the list,
/// the summary or the preview.
fn synthetic_record(value: &Value) -> bool {
    let Some(kinds) = value["payload"]["internal_chat_message_metadata_passthrough"]
        ["content_item_kinds"]
        .as_array()
    else {
        return false;
    };
    !kinds.is_empty()
        && kinds.iter().all(|kind| {
            kind.as_str().is_some_and(|kind| {
                [
                    "additional_content.",
                    "agents_md.",
                    "environments.",
                    "multi_agent.",
                    "images.",
                    "permissions.",
                ]
                .iter()
                .any(|prefix| kind.starts_with(prefix))
            })
        })
}

fn leading_tag(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('<')?;
    let end = rest.find(|c: char| c == '>' || c == '/' || c.is_whitespace())?;
    Some(&rest[..end])
}

fn synthetic_tag(tag: &str) -> bool {
    let tag = tag.to_ascii_lowercase();
    matches!(
        tag.as_str(),
        "environment_context"
            | "user_instructions"
            | "permissions"
            | "recommended_plugins"
            | "in-app-browser-context"
            | "image_resize_notice"
            | "turn_aborted"
    ) || tag.starts_with("external_")
        || tag.starts_with("codex_apps_")
        || tag.starts_with("multi_agent_")
        || tag.starts_with("subagent_notification")
}

/// Strips leading synthetic blocks; returns `None` when nothing but synthetic
/// content was left. An unclosed synthetic block swallows the whole message.
fn strip_leading_synthetic(text: &str) -> Option<String> {
    let mut rest = text.trim();
    loop {
        let Some(tag) = leading_tag(rest) else {
            break;
        };
        if !synthetic_tag(tag) {
            break;
        }
        let lower = rest.to_ascii_lowercase();
        let close = format!("</{}>", tag.to_ascii_lowercase());
        let Some(end) = lower.find(&close) else {
            return Some(String::new());
        };
        rest = rest[end + close.len()..].trim();
    }
    Some(rest.into())
}

/// Removes `<image name=… path=…>…</image>` markup that the client appends to
/// attachment messages. It is presentation metadata, never user prose.
fn strip_image_markup(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let open = "<image";
    let close = "</image>";
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(open) {
        let start = cursor + relative;
        let after = start + open.len();
        let Some(next) = lower[after..].chars().next() else {
            break;
        };
        if !(next == '>' || next == '/' || next.is_whitespace()) {
            output.push_str(&text[cursor..after]);
            cursor = after;
            continue;
        }
        let Some(open_end) = lower[after..].find('>').map(|index| after + index + 1) else {
            break;
        };
        // Only the client's attachment markup carries attributes; a bare
        // `<image>` discussed in prose stays untouched.
        let attributes = &lower[after..open_end];
        if !attributes.contains("path=") && !attributes.contains("name=") {
            output.push_str(&text[cursor..open_end]);
            cursor = open_end;
            continue;
        }
        output.push_str(&text[cursor..start]);
        if lower[after..open_end].trim_end().ends_with('/') {
            cursor = open_end;
            continue;
        }
        cursor = lower[open_end..]
            .find(close)
            .map(|index| open_end + index + close.len())
            .unwrap_or(text.len());
    }
    output.push_str(&text[cursor..]);
    output
}

/// Approval transcripts are numbered `[12] tool exec result: …` / `[7] assistant: …`.
/// They are internal reviewer traffic, not user prose.
fn internal_transcript_line(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('[') else {
        return false;
    };
    let Some(end) = rest.find(']') else {
        return false;
    };
    if end == 0 || !rest[..end].bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let after = rest[end + 1..].trim_start();
    after.starts_with("assistant:")
        || (after.starts_with("tool ") && (after.contains(" call:") || after.contains(" result:")))
}

pub fn user_text(text: &str) -> Option<String> {
    let mut text = text.trim();
    // Internal auto-approval payloads persist the reviewed transcript, which can
    // itself contain a `## My request:` marker. Reject them before extracting
    // anything, otherwise the extraction would surface the transcript instead.
    let original = text.to_ascii_lowercase();
    if original.starts_with("the following is the codex agent history")
        || original.starts_with(">>> transcript delta")
        || internal_transcript_line(text)
    {
        return None;
    }
    for marker in ["## My request:", "# My request:"] {
        if let Some((_, request)) = text.split_once(marker) {
            text = request.trim();
            break;
        }
    }
    let without_images = strip_image_markup(text);
    let stripped = strip_leading_synthetic(&without_images)?;
    let text = stripped.trim();
    let lower = text.to_ascii_lowercase();
    if text.is_empty()
        || lower.starts_with("# agents.md instructions for")
        || lower.starts_with("# files mentioned by the user:")
        || lower.starts_with("<instructions>")
        || lower.starts_with("<system")
        || lower.starts_with("<developer")
        // Internal auto-approval turns are persisted as user messages but are
        // never rendered as questions by the client.
        || lower.starts_with("the following is the codex agent history")
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
        if synthetic_record(&value) {
            continue;
        }
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
    fn ui_actions_are_not_mistaken_for_questions_or_titles() {
        let raw = lines(vec![
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"<recommended_plugins>\nHere is a list of plugins.\n</recommended_plugins>"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"<in-app-browser-context source=\"ambient-ui-state\">\nCurrent URL: https://example.com\n</in-app-browser-context>\n\n## My request:\n真实问题"}]}}),
        ]);
        assert!(title("The following is the Codex agent history whose request action you are assessing.\n>>> START").is_none());
        assert!(title("The following is the Codex agent history added since your last approval assessment.\n>>> TRANSCRIPT DELTA START\n## My request:\n这是被审核记录里的旧请求").is_none());
        assert!(title("[10] tool exec result: Script completed\nWall time 12.1 seconds").is_none());
        assert!(title("[176] tool js call: {\"code\":\"x\"}").is_none());
        assert!(title("[215] assistant: Agent message from /root").is_none());
        // Ordinary numbered prose must survive.
        assert_eq!(
            title("[1] 第一步：打开设置").as_deref(),
            Some("[1] 第一步：打开设置")
        );
        let (items, _) = messages(&raw, 100);
        assert_eq!(
            items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            vec!["真实问题"]
        );
        assert_eq!(title("真实问题").as_deref(), Some("真实问题"));
    }

    #[test]
    fn metadata_marked_additional_content_is_ignored() {
        let raw = lines(vec![
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"客户端动作"}],
                "internal_chat_message_metadata_passthrough":{"content_item_kinds":["additional_content.codex_apps_open_page"]}}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"真正的提问"}],
                "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}}),
        ]);
        let (items, _) = messages(&raw, 100);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "真正的提问");
    }

    #[test]
    fn attachment_lists_only_count_when_they_carry_a_request() {
        assert!(title("# Files mentioned by the user:\n\n## shot.png: C:\\tmp\\shot.png\nImage attachment: true").is_none());
        assert_eq!(
            title("# Files mentioned by the user:\n\n## shot.png: C:\\tmp\\shot.png\nImage attachment: true\n\n## My request:\n把间距增大 4px").as_deref(),
            Some("把间距增大 4px")
        );
    }

    #[test]
    fn image_attachment_markup_does_not_leak_into_the_title() {
        assert_eq!(
            title("红色区域的文字不需要\n\n<image name=[Image #1] path=\"C:\\tmp\\shot.png\">\n\n</image>").as_deref(),
            Some("红色区域的文字不需要")
        );
        // A bare <image> discussed in prose must survive verbatim.
        assert_eq!(
            title("怎么在 HTML 里写 <image> 标签？").as_deref(),
            Some("怎么在 HTML 里写 <image> 标签？")
        );
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
