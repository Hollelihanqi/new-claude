// 配置校验、transport 推断、敏感字段识别、脱敏、基础可达性测试。
// 纯函数为主，便于单元测试；网络可达性用 std + url crate，不引入 reqwest/MCP SDK。

use super::*;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub const REDACTED: &str = "__CC_MANAGER_REDACTED__";

// 保留名：与 Claude Code 内建工具/特殊服务冲突，禁止新建。
const RESERVED_NAMES: [&str; 5] = [
    "workspace",
    "claude-in-chrome",
    "computer-use",
    "Claude Preview",
    "Claude Browser",
];

// 大小写不敏感精确匹配的敏感 key（已小写）。
const SENSITIVE_KEYS: [&str; 14] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "token",
    "access_token",
    "refresh_token",
    "secret",
    "client_secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "credential",
];

// env/任意 key 中出现这些子串（大写匹配）也视为敏感。
const SENSITIVE_SUBSTRINGS: [&str; 6] = [
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "API_KEY",
    "APIKEY",
    "CREDENTIAL",
];

// ---------------- 名称 ----------------

pub(crate) fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    let len = trimmed.chars().count();
    if !(1..=64).contains(&len) {
        return Err("服务名称长度需为 1-64 个字符".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("名称只能包含字母、数字、点、下划线、短横线".into());
    }
    if RESERVED_NAMES.contains(&trimmed) {
        return Err(format!("名称「{trimmed}」为保留名，请换一个"));
    }
    Ok(trimmed.to_string())
}

/// 读取既有配置时旧名称可能非法；只标记不拦截（重命名以外的保存允许保留旧名）。
pub(crate) fn name_is_valid(name: &str) -> bool {
    validate_name(name).is_ok()
}

// ---------------- transport ----------------

pub(crate) fn infer_transport(config: &Map<String, Value>) -> McpTransport {
    match config.get("type").and_then(|v| v.as_str()) {
        Some("http") | Some("streamable-http") => McpTransport::Http,
        Some("sse") => McpTransport::Sse,
        Some("ws") => McpTransport::Ws,
        Some("stdio") => McpTransport::Stdio,
        _ => {
            if config
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                McpTransport::Stdio
            } else {
                McpTransport::Unknown
            }
        }
    }
}

/// 保留原始 type 字符串：streamable-http、未知类型等原样保存，不在保存时丢失。
pub(crate) fn raw_transport(config: &Map<String, Value>) -> Option<String> {
    match config.get("type").and_then(|v| v.as_str()) {
        Some(s) if !matches!(s, "http" | "sse" | "ws" | "stdio") => Some(s.to_string()),
        _ => None,
    }
}

// ---------------- 运行时变量检测 ----------------

/// 配置含 ${VAR} / ${VAR:-default} 时无法静态展开，跳过可达性检查。
pub(crate) fn contains_runtime_var(config: &Map<String, Value>) -> bool {
    let cmd = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let url = config.get("url").and_then(|v| v.as_str()).unwrap_or("");
    cmd.contains("${") || url.contains("${")
}

// ---------------- 配置结构校验 ----------------

/// 列表展示用的结构警告（不阻断保存，仅提示用户）。
pub(crate) fn config_warnings(config: &Map<String, Value>) -> Vec<String> {
    let mut warns = vec![];
    let transport = infer_transport(config);
    match transport {
        McpTransport::Stdio => {
            let cmd = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if cmd.is_empty() {
                warns.push("stdio 配置缺少非空 command".into());
            }
            if let Some(args) = config.get("args") {
                if !args_is_string_array(args) {
                    warns.push("args 必须是字符串数组".into());
                }
            }
            if let Some(env) = config.get("env") {
                if !env_is_string_map(env) {
                    warns.push("env 必须是字符串值对象".into());
                }
            }
        }
        McpTransport::Http | McpTransport::Sse | McpTransport::Ws => {
            let url = config.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if !valid_scheme(url, transport) {
                warns.push("url 协议与 transport 不匹配".into());
            }
            if transport == McpTransport::Sse {
                warns.push("SSE 已弃用，服务支持时请迁移到 HTTP".into());
            }
        }
        McpTransport::Unknown => {
            warns.push("无法识别 transport 类型，仅可经原始 JSON 编辑".into());
        }
    }
    if let Some(t) = config.get("timeout") {
        match t {
            Value::Number(n) if n.as_i64().map(|i| i >= 1000).unwrap_or(false) => {}
            _ => warns.push("timeout 存在时必须是 >= 1000 的整数".into()),
        }
    }
    if let Some(v) = config.get("alwaysLoad") {
        if !v.is_boolean() {
            warns.push("alwaysLoad 必须是布尔值".into());
        }
    }
    warns
}

fn args_is_string_array(v: &Value) -> bool {
    v.as_array()
        .map(|a| a.iter().all(|x| x.is_string()))
        .unwrap_or(false)
}

fn env_is_string_map(v: &Value) -> bool {
    v.as_object()
        .map(|m| m.values().all(|x| x.is_string()))
        .unwrap_or(false)
}

fn valid_scheme(url: &str, transport: McpTransport) -> bool {
    let scheme = url::Url::parse(url).ok().map(|u| u.scheme().to_lowercase());
    match transport {
        McpTransport::Http | McpTransport::Sse => {
            scheme.as_deref() == Some("http") || scheme.as_deref() == Some("https")
        }
        McpTransport::Ws => scheme.as_deref() == Some("ws") || scheme.as_deref() == Some("wss"),
        _ => false,
    }
}

/// 测试面板用的结构校验：返回 schema 阶段（可能附带 fail/warn 明细）。
pub(crate) fn validate_config(config: &Map<String, Value>) -> Vec<McpTestStage> {
    let warns = config_warnings(config);
    let transport = infer_transport(config);
    // pretty 序列化体积上限
    let size = serde_json::to_string_pretty(&Value::Object(config.clone()))
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    let mut detail = String::new();
    let mut status = McpTestStatus::Ok;
    if size > 256 * 1024 {
        detail.push_str("配置序列化后超过 256 KiB；");
        status = McpTestStatus::Fail;
    }
    if transport == McpTransport::Unknown {
        // URL 存在但 type 缺失时不能误判为 stdio
        let has_url = config
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if has_url {
            detail.push_str("存在 url 但缺少有效的 type；");
            status = McpTestStatus::Fail;
        } else {
            detail.push_str("未知 transport；");
            if status != McpTestStatus::Fail {
                status = McpTestStatus::Warn;
            }
        }
    }
    if !warns.is_empty() {
        detail.push_str(&warns.join("；"));
        if status != McpTestStatus::Fail {
            status = McpTestStatus::Warn;
        }
    }
    if detail.is_empty() {
        detail = "配置结构通过校验".into();
    }
    vec![McpTestStage {
        id: McpTestStageId::Schema,
        status,
        detail,
    }]
}

/// apply/preview 硬校验：阻断错误 schema（空 command、协议不匹配、url 缺 type、
/// args/env/headers 类型错、timeout/alwaysLoad 非法、体积超限）。SSE 弃用等仅作为 warning，不在此拒绝。
pub(crate) fn validate_config_strict(config: &Map<String, Value>) -> Result<(), String> {
    let size = serde_json::to_string_pretty(&Value::Object(config.clone()))
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if size > 256 * 1024 {
        return Err("配置序列化后超过 256 KiB".into());
    }
    let transport = infer_transport(config);
    let url_str = config.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let has_url = !url_str.is_empty();
    match transport {
        McpTransport::Stdio => {
            let cmd = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if cmd.is_empty() {
                return Err("stdio 配置缺少非空 command".into());
            }
        }
        McpTransport::Http | McpTransport::Sse => {
            if !valid_scheme(url_str, transport) {
                return Err("http/sse 的 url 必须是 http/https".into());
            }
        }
        McpTransport::Ws => {
            if !valid_scheme(url_str, transport) {
                return Err("ws 的 url 必须是 ws/wss".into());
            }
        }
        McpTransport::Unknown => {
            if has_url {
                return Err("存在 url 但缺少有效的 type".into());
            }
            // 纯未知 type（无 url）：允许保留，结构无法进一步校验
        }
    }
    if let Some(args) = config.get("args") {
        if !args_is_string_array(args) {
            return Err("args 必须是字符串数组".into());
        }
    }
    if let Some(env) = config.get("env") {
        if !env_is_string_map(env) {
            return Err("env 必须是字符串值对象".into());
        }
    }
    if let Some(headers) = config.get("headers") {
        if !env_is_string_map(headers) {
            return Err("headers 必须是字符串值对象".into());
        }
    }
    if let Some(t) = config.get("timeout") {
        match t {
            Value::Number(n) if n.as_i64().map(|i| i >= 1000).unwrap_or(false) => {}
            _ => return Err("timeout 必须是 >= 1000 的整数".into()),
        }
    }
    if let Some(v) = config.get("alwaysLoad") {
        if !v.is_boolean() {
            return Err("alwaysLoad 必须是布尔值".into());
        }
    }
    Ok(())
}

// ---------------- 敏感字段 / 脱敏 ----------------

fn is_sensitive_key(k: &str) -> bool {
    let lower = k.to_lowercase();
    if SENSITIVE_KEYS.contains(&lower.as_str()) {
        return true;
    }
    let upper = k.to_uppercase();
    SENSITIVE_SUBSTRINGS.iter().any(|s| upper.contains(s))
}

fn escape_pointer_token(k: &str) -> String {
    k.replace('~', "~0").replace('/', "~1")
}

fn walk_sensitive(value: &Value, pointer: &mut String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let prev = pointer.len();
                pointer.push('/');
                pointer.push_str(&escape_pointer_token(k));
                if is_sensitive_key(k) {
                    out.push(pointer.clone());
                    // 整棵子树脱敏，不再下钻
                } else {
                    walk_sensitive(v, pointer, out);
                }
                pointer.truncate(prev);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let prev = pointer.len();
                pointer.push('/');
                pointer.push_str(&i.to_string());
                walk_sensitive(v, pointer, out);
                pointer.truncate(prev);
            }
        }
        _ => {}
    }
}

/// 返回 RFC 6901 JSON Pointer 列表（相对 config 根）。
pub(crate) fn sensitive_paths(config: &Value) -> Vec<String> {
    let mut pointer = String::new();
    let mut out = vec![];
    walk_sensitive(config, &mut pointer, &mut out);
    out
}

fn pointer_get_mut<'a>(root: &'a mut Value, pointer: &str) -> Option<&'a mut Value> {
    if pointer.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for raw in pointer.split('/') {
        if raw.is_empty() {
            continue;
        }
        let token = raw.replace("~1", "/").replace("~0", "~");
        match cur {
            Value::Object(map) => cur = map.get_mut(&token)?,
            Value::Array(arr) => {
                let idx: usize = token.parse().ok()?;
                cur = arr.get_mut(idx)?;
            }
            _ => return None,
        }
    }
    Some(cur)
}

/// 深拷贝并把敏感位置替换为 REDACTED；不修改原对象。
pub(crate) fn redact(config: &Value, paths: &[String]) -> Value {
    let mut out = config.clone();
    for p in paths {
        if let Some(target) = pointer_get_mut(&mut out, p) {
            *target = Value::String(REDACTED.to_string());
        }
    }
    out
}

// ---------------- 基础可达性测试 ----------------

enum CommandStatus {
    Found,
    NotFound,
    NpxWarn,
}

/// command 是绝对路径：检查存在且是文件；否则按 PATH 搜索，Windows 补 .exe/.cmd/.bat。
fn resolve_command(command: &str) -> CommandStatus {
    let pb = PathBuf::from(command);
    if pb.is_absolute() {
        if pb.is_file() {
            return CommandStatus::Found;
        }
        return CommandStatus::NotFound;
    }
    if cfg!(target_os = "windows") && command.eq_ignore_ascii_case("npx") {
        return CommandStatus::NpxWarn;
    }
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for ext in exts {
                let mut candidate = dir.clone();
                candidate.push(format!("{command}{ext}"));
                if candidate.is_file() {
                    return CommandStatus::Found;
                }
            }
        }
    }
    CommandStatus::NotFound
}

fn stage(id: McpTestStageId, status: McpTestStatus, detail: impl Into<String>) -> McpTestStage {
    McpTestStage {
        id,
        status,
        detail: detail.into(),
    }
}

/// 仅做配置结构校验和基础可达性检查：不启动进程、不发 HTTP/握手、不发送凭据。
pub(crate) fn test_basic(_name: &str, config: &Map<String, Value>) -> McpTestResult {
    let transport = infer_transport(config);
    let mut stages = validate_config(config);
    let has_var = contains_runtime_var(config);

    match transport {
        McpTransport::Stdio => {
            let cmd = config.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if has_var {
                stages.push(stage(
                    McpTestStageId::Command,
                    McpTestStatus::Skipped,
                    "包含运行时环境变量，未展开，跳过基础可达性检查",
                ));
            } else if cmd.is_empty() {
                stages.push(stage(
                    McpTestStageId::Command,
                    McpTestStatus::Fail,
                    "缺少非空 command",
                ));
            } else {
                match resolve_command(cmd) {
                    CommandStatus::Found => stages.push(stage(
                        McpTestStageId::Command,
                        McpTestStatus::Ok,
                        "命令可在 PATH 中找到",
                    )),
                    CommandStatus::NotFound => stages.push(stage(
                        McpTestStageId::Command,
                        McpTestStatus::Fail,
                        format!("未在 PATH 中找到命令「{cmd}」"),
                    )),
                    CommandStatus::NpxWarn => stages.push(stage(
                        McpTestStageId::Command,
                        McpTestStatus::Warn,
                        "Windows 原生环境请把 command 改为 cmd，并把 /c、npx 放在 args 最前面",
                    )),
                }
            }
        }
        McpTransport::Http | McpTransport::Sse | McpTransport::Ws => {
            let url_str = config.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let parsed = url::Url::parse(url_str).ok();
            let scheme_ok = parsed
                .as_ref()
                .map(|_| valid_scheme(url_str, transport))
                .unwrap_or(false);
            if scheme_ok {
                stages.push(stage(
                    McpTestStageId::Url,
                    McpTestStatus::Ok,
                    "URL 格式与协议匹配",
                ));
            } else {
                stages.push(stage(
                    McpTestStageId::Url,
                    McpTestStatus::Fail,
                    "URL 缺失或协议不匹配",
                ));
            }
            if scheme_ok {
                let host = parsed.as_ref().and_then(|u| u.host_str()).unwrap_or("");
                let port = parsed
                    .as_ref()
                    .and_then(|u| u.port_or_known_default())
                    .unwrap_or(0);
                if has_var || host.is_empty() || port == 0 {
                    stages.push(stage(
                        McpTestStageId::Endpoint,
                        McpTestStatus::Skipped,
                        "包含运行时环境变量或缺少主机/端口，跳过基础可达性检查",
                    ));
                } else {
                    match tcp_reachable(host, port) {
                        Ok(()) => stages.push(stage(
                            McpTestStageId::Endpoint,
                            McpTestStatus::Ok,
                            "端点可达，尚未执行 MCP 握手",
                        )),
                        Err(e) => stages.push(stage(
                            McpTestStageId::Endpoint,
                            McpTestStatus::Fail,
                            format!("无法连接 {host}:{port}（{e}）"),
                        )),
                    }
                }
            }
        }
        McpTransport::Unknown => {
            // schema 阶段已说明
        }
    }

    let ok = stages.iter().all(|s| s.status != McpTestStatus::Fail);
    let sanitized_detail = stages
        .iter()
        .map(|s| format!("{:?}: {}", s.status, s.detail))
        .collect::<Vec<_>>()
        .join("\n");
    McpTestResult {
        ok,
        transport,
        stages,
        sanitized_detail,
    }
}

fn tcp_reachable(host: &str, port: u16) -> Result<(), String> {
    use std::net::ToSocketAddrs;
    let addr = format!("{host}:{port}");
    let mut iter = addr
        .to_socket_addrs()
        .map_err(|e| format!("DNS 解析失败：{e}"))?;
    let sock = iter.next().ok_or_else(|| "无可用地址".to_string())?;
    std::net::TcpStream::connect_timeout(&sock, std::time::Duration::from_secs(3))
        .map_err(|e| format!("连接超时或被拒：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn name_validation_rules() {
        assert_eq!(validate_name("github").unwrap(), "github");
        assert_eq!(validate_name("  my-srv.v2  ").unwrap(), "my-srv.v2");
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("bad/name").is_err());
        assert!(validate_name("workspace").is_err());
        assert!(validate_name("computer-use").is_err());
    }

    #[test]
    fn infer_transport_branches() {
        assert_eq!(
            infer_transport(&map(json!({"type":"http","url":"https://a"}))),
            McpTransport::Http
        );
        assert_eq!(
            infer_transport(&map(json!({"type":"streamable-http","url":"https://a"}))),
            McpTransport::Http
        );
        assert_eq!(
            infer_transport(&map(json!({"type":"sse","url":"https://a"}))),
            McpTransport::Sse
        );
        assert_eq!(
            infer_transport(&map(json!({"command":"node"}))),
            McpTransport::Stdio
        );
        assert_eq!(
            infer_transport(&map(json!({"type":"stdio","command":"node"}))),
            McpTransport::Stdio
        );
        assert_eq!(
            infer_transport(&map(json!({"weird":true}))),
            McpTransport::Unknown
        );
        // URL 存在但 type 缺失不能误判为 stdio
        assert_eq!(
            infer_transport(&map(json!({"url":"https://a"}))),
            McpTransport::Unknown
        );
    }

    #[test]
    fn raw_transport_preserves_streamable_http() {
        assert_eq!(
            raw_transport(&map(json!({"type":"streamable-http"}))),
            Some("streamable-http".into())
        );
        assert_eq!(raw_transport(&map(json!({"type":"http"}))), None);
        assert_eq!(
            raw_transport(&map(json!({"type":"weird"}))),
            Some("weird".into())
        );
    }

    #[test]
    fn sensitive_paths_and_redaction() {
        let cfg = json!({
            "command": "node",
            "env": { "API_TOKEN": "abc", "HARMLESS": "x" },
            "headers": { "Authorization": "Bearer s3cr3t", "X-Public": "p" },
            "url": "https://example.com"
        });
        let paths = sensitive_paths(&cfg);
        assert!(paths.iter().any(|p| p == "/env/API_TOKEN"));
        assert!(paths.iter().any(|p| p == "/headers/Authorization"));
        assert!(!paths.iter().any(|p| p.contains("HARMLESS")));
        assert!(!paths.iter().any(|p| p.contains("X-Public")));

        let redacted = redact(&cfg, &paths);
        assert_eq!(redacted["env"]["API_TOKEN"], REDACTED);
        assert_eq!(redacted["headers"]["Authorization"], REDACTED);
        assert_eq!(redacted["env"]["HARMLESS"], "x");
        assert_eq!(redacted["url"], "https://example.com");
        // 原对象未被修改
        assert_eq!(cfg["headers"]["Authorization"], "Bearer s3cr3t");
    }

    #[test]
    fn stdio_config_validation() {
        let stages = validate_config(&map(json!({"command":"node","args":["a.js"]})));
        assert_eq!(stages[0].status, McpTestStatus::Ok);
        let bad = validate_config(&map(json!({"command":"node","args":"not-array"})));
        assert_eq!(bad[0].status, McpTestStatus::Warn);
    }

    #[test]
    fn http_scheme_mismatch_flags_warning() {
        let stages = validate_config(&map(json!({"type":"http","url":"ftp://x"})));
        assert_eq!(stages[0].status, McpTestStatus::Warn);
        let ws = validate_config(&map(json!({"type":"ws","url":"https://x"})));
        assert_eq!(ws[0].status, McpTestStatus::Warn);
    }

    #[test]
    fn url_without_type_fails_schema() {
        let stages = validate_config(&map(json!({"url":"https://example.com"})));
        assert_eq!(stages[0].status, McpTestStatus::Fail);
    }

    #[test]
    fn runtime_var_skips_command_stage() {
        let result = test_basic("s", &map(json!({"command":"${BIN}"})));
        let cmd_stage = result
            .stages
            .iter()
            .find(|s| s.id == McpTestStageId::Command)
            .unwrap();
        assert_eq!(cmd_stage.status, McpTestStatus::Skipped);
    }

    #[test]
    fn unknown_command_fails() {
        let result = test_basic(
            "s",
            &map(json!({"command":"this-command-does-not-exist-xyz"})),
        );
        let cmd_stage = result
            .stages
            .iter()
            .find(|s| s.id == McpTestStageId::Command)
            .unwrap();
        assert_eq!(cmd_stage.status, McpTestStatus::Fail);
    }
}
