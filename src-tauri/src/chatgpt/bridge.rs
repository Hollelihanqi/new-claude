//! Opt-in loopback bridge for API gateways that expose Anthropic Messages but
//! not OpenAI Responses. A separate process keeps an open desktop instance
//! usable when the PathMux window itself is closed.
use super::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::time::{Duration, Instant};

const MARKER: &str = "--chatgpt-api-bridge";
const MAX_REQUEST: usize = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Runtime {
    pid: u32,
    port: u16,
}

fn runtime_path(dir: &Path) -> PathBuf {
    dir.join("api-bridge-runtime.json")
}

fn healthy(runtime: &Runtime) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", runtime.port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = [0; 64];
    stream
        .read(&mut response)
        .is_ok_and(|n| response[..n].starts_with(b"HTTP/1.1 200"))
}

pub fn start(dir: &Path, executable: &Path) -> Result<u16, String> {
    storage::plain(&runtime_path(dir))?;
    if let Ok(runtime) = storage::read_json::<Runtime>(&runtime_path(dir)) {
        if healthy(&runtime) {
            return Ok(runtime.port);
        }
    }
    let own_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(own_exe);
    command
        .arg(MARKER)
        .arg(dir)
        .arg(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = process::quiet(&mut command)
        .spawn()
        .map_err(|e| format!("无法启动 API 兼容服务：{e}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(runtime) = storage::read_json::<Runtime>(&runtime_path(dir)) {
            if runtime.pid == child.id() && healthy(&runtime) {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(runtime.port);
            }
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("API 兼容服务启动失败，请检查实例配置".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("API 兼容服务启动超时".into())
}

pub fn serve(dir: &Path, executable: &Path) -> Result<(), String> {
    storage::plain(dir)?;
    let Some((_, _, true)) = api_config::compatibility(dir)? else {
        return Err("实例没有启用 API 兼容模式".into());
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let runtime = Runtime {
        pid: std::process::id(),
        port: listener.local_addr().map_err(|e| e.to_string())?.port(),
    };
    storage::write_json(&runtime_path(dir), &runtime)?;
    let started = Instant::now();
    let mut last_live = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let dir = dir.to_path_buf();
                std::thread::spawn(move || {
                    let _ = handle(stream, &dir);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.to_string()),
        }
        if started.elapsed() > Duration::from_secs(30)
            && last_live.elapsed() > Duration::from_secs(8)
        {
            let live = process::snapshot()
                .ok()
                .and_then(|all| process::main_process(&all, dir, executable).map(|_| ()));
            if live.is_some() {
                last_live = Instant::now();
            } else if last_live.elapsed() > Duration::from_secs(12) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if storage::read_json::<Runtime>(&runtime_path(dir)).is_ok_and(|saved| saved.pid == runtime.pid)
    {
        let _ = fs::remove_file(runtime_path(dir));
    }
    Ok(())
}

fn response(stream: &mut TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        422 => "Unprocessable Content",
        502 => "Bad Gateway",
        _ => "Error",
    };
    write!(stream, "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

fn error(stream: &mut TcpStream, code: u16, message: &str) -> std::io::Result<()> {
    response(
        stream,
        code,
        &json!({"error":{"message":message,"type":"pathmux_compatibility_error"}}).to_string(),
    )
}

fn handle(mut stream: TcpStream, dir: &Path) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut head = String::new();
    reader.read_line(&mut head).map_err(|e| e.to_string())?;
    if head.len() > 4096 {
        let _ = error(&mut stream, 413, "请求过大");
        return Ok(());
    }
    let mut length = None;
    let mut authorization = String::new();
    let mut header_bytes = head.len();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        header_bytes += n;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if header_bytes > 32 * 1024 {
            let _ = error(&mut stream, 413, "请求头过大");
            return Ok(());
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().ok();
            }
            if name.eq_ignore_ascii_case("authorization") {
                authorization = value.trim().to_owned();
            }
        }
    }
    if head.starts_with("GET /health ") {
        return response(&mut stream, 200, "{}").map_err(|e| e.to_string());
    }
    let Some((base_url, key, true)) = api_config::compatibility(dir)? else {
        return error(&mut stream, 502, "实例未启用兼容模式").map_err(|e| e.to_string());
    };
    if !secure_equal(
        authorization
            .strip_prefix("Bearer ")
            .unwrap_or("")
            .as_bytes(),
        key.as_bytes(),
    ) {
        return error(&mut stream, 401, "本地兼容服务认证失败").map_err(|e| e.to_string());
    }
    if head.starts_with("GET /v1/models ") {
        return models(&mut stream, &base_url, &key);
    }
    if !head.starts_with("POST /v1/responses ") {
        return error(&mut stream, 404, "兼容模式暂不支持此接口").map_err(|e| e.to_string());
    }
    let Some(length) = length else {
        return error(&mut stream, 400, "缺少请求长度").map_err(|e| e.to_string());
    };
    if length > MAX_REQUEST {
        return error(&mut stream, 413, "请求过大").map_err(|e| e.to_string());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    let request: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error(&mut stream, 400, "请求 JSON 无效").map_err(|e| e.to_string()),
    };
    responses(&mut stream, &base_url, &key, &request)
}

fn secure_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn client(base_url: &str) -> Result<reqwest::blocking::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none());
    if api_config::is_private_gateway_url(base_url) {
        builder = builder.no_proxy();
    }
    if let Ok(bundle) = fs::read(crate::union_ca_bundle_path()) {
        if !bundle.is_empty() {
            let certs =
                reqwest::Certificate::from_pem_bundle(&bundle).map_err(|_| "导入的 CA 证书无效")?;
            if api_config::is_private_gateway_url(base_url) {
                builder = builder.tls_certs_only(certs);
            } else {
                for cert in certs {
                    builder = builder.add_root_certificate(cert);
                }
            }
        }
    }
    builder.build().map_err(|e| e.to_string())
}

fn models(stream: &mut TcpStream, base_url: &str, key: &str) -> Result<(), String> {
    let upstream = match client(base_url).and_then(|client| {
        client
            .get(format!("{base_url}/models"))
            .bearer_auth(key)
            .send()
            .map_err(|e| e.to_string())
    }) {
        Ok(upstream) => upstream,
        Err(message) => {
            return error(
                stream,
                502,
                &format!("连接模型网关失败：{}", message.replace(key, "[REDACTED]")),
            )
            .map_err(|e| e.to_string())
        }
    };
    let code = upstream.status().as_u16();
    let body = upstream.text().map_err(|e| e.to_string())?;
    response(stream, code, &body).map_err(|e| e.to_string())
}

fn text_content(content: &Value) -> Result<Vec<Value>, String> {
    if let Some(text) = content.as_str() {
        return Ok(vec![json!({"type":"text","text":text})]);
    }
    let Some(parts) = content.as_array() else {
        return Err("消息内容格式不受支持".into());
    };
    let mut converted = Vec::new();
    for part in parts {
        let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "input_text" | "output_text" | "text" => {
                let text = part
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("消息缺少文字内容")?;
                converted.push(json!({"type":"text","text":text}));
            }
            _ => return Err(format!("兼容模式暂不支持 {kind} 类型的消息内容")),
        }
    }
    Ok(converted)
}

#[derive(Debug)]
struct Translation {
    body: Value,
    to_original: HashMap<String, String>,
}

fn anthropic_request(request: &Value) -> Result<Translation, String> {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .ok_or("请求缺少模型名称")?;
    let mut mapped_tools = Vec::new();
    let mut to_original = HashMap::new();
    let mut to_upstream = HashMap::new();
    if let Some(tools) = request.get("tools").and_then(Value::as_array) {
        for tool in tools {
            let kind = tool
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let entries: Vec<(&Value, String)> = match kind {
                "function" => vec![(
                    tool,
                    tool.get("name")
                        .and_then(Value::as_str)
                        .ok_or("工具缺少名称")?
                        .to_owned(),
                )],
                "namespace" => {
                    let namespace = tool
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("工具命名空间缺少名称")?;
                    let nested = tool
                        .get("tools")
                        .and_then(Value::as_array)
                        .ok_or("工具命名空间缺少工具列表")?;
                    nested
                        .iter()
                        .map(|entry| {
                            let name = entry
                                .get("name")
                                .and_then(Value::as_str)
                                .ok_or("命名空间工具缺少名称")?;
                            Ok((entry, format!("{namespace}.{name}")))
                        })
                        .collect::<Result<Vec<_>, String>>()?
                }
                // Responses exposes hosted search as a tool. Anthropic Messages
                // gateways cannot execute the OpenAI hosted tool, so omit its
                // declaration; normal conversation remains available.
                "web_search" | "web_search_preview" => Vec::new(),
                _ => return Err(format!("兼容模式暂不支持 {kind} 类型的工具")),
            };
            for (entry, original) in entries {
                let upstream = format!("pathmux_tool_{}", mapped_tools.len());
                let schema = entry
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}}));
                mapped_tools.push(json!({"name":upstream,"description":entry.get("description").and_then(Value::as_str).unwrap_or(""),"input_schema":schema}));
                to_original.insert(upstream.clone(), original.clone());
                to_upstream.insert(original, upstream);
            }
        }
    }
    let mut system = request
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut messages: Vec<Value> = Vec::new();
    let input = request.get("input").ok_or("请求缺少 input")?;
    let items: Vec<Value> = if let Some(text) = input.as_str() {
        vec![json!({"type":"message","role":"user","content":text})]
    } else {
        input.as_array().ok_or("input 格式不受支持")?.clone()
    };
    for item in items {
        match item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message")
        {
            "message" => {
                let role = item
                    .get("role")
                    .and_then(Value::as_str)
                    .ok_or("消息缺少角色")?;
                let content = text_content(item.get("content").ok_or("消息缺少内容")?)?;
                if role == "system" || role == "developer" {
                    for part in content {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            if !system.is_empty() {
                                system.push_str("\n\n");
                            }
                            system.push_str(text);
                        }
                    }
                } else if role == "user" || role == "assistant" {
                    messages.push(json!({"role":role,"content":content}));
                } else {
                    return Err(format!("兼容模式暂不支持 {role} 角色"));
                }
            }
            "function_call" => {
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or("工具调用缺少 call_id")?;
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("工具调用缺少名称")?;
                let args = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let parsed: Value =
                    serde_json::from_str(args).map_err(|_| "工具调用参数不是有效 JSON")?;
                let upstream_name = to_upstream.get(name).map(String::as_str).unwrap_or(name);
                messages.push(json!({"role":"assistant","content":[{"type":"tool_use","id":id,"name":upstream_name,"input":parsed}]}));
            }
            "function_call_output" => {
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or("工具结果缺少 call_id")?;
                let output = item
                    .get("output")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        item.get("output").map(Value::to_string).unwrap_or_default()
                    });
                messages.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":output}]}));
            }
            "reasoning" => {}
            kind => return Err(format!("兼容模式暂不支持 {kind} 类型的历史记录")),
        }
    }
    // Anthropic requires alternating turns; the Responses input may contain
    // adjacent assistant tool calls or adjacent tool results.
    let mut merged: Vec<Value> = Vec::new();
    for message in messages {
        if let Some(last) = merged.last_mut() {
            if last["role"] == message["role"] {
                if let (Some(dst), Some(src)) = (
                    last.get_mut("content").and_then(Value::as_array_mut),
                    message.get("content").and_then(Value::as_array),
                ) {
                    dst.extend(src.iter().cloned());
                    continue;
                }
            }
        }
        merged.push(message);
    }
    if merged.is_empty() {
        return Err("没有可发送的消息".into());
    }
    let max_tokens = request
        .get("max_output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(8192)
        .clamp(1, 16384);
    let mut result = json!({"model":model,"max_tokens":max_tokens,"messages":merged,"stream":request.get("stream").and_then(Value::as_bool).unwrap_or(false)});
    if !system.is_empty() {
        result["system"] = Value::String(system);
    }
    if !mapped_tools.is_empty() {
        result["tools"] = Value::Array(mapped_tools);
    }
    if let Some(choice) = request.get("tool_choice") {
        let kind = choice
            .as_str()
            .or_else(|| choice.get("type").and_then(Value::as_str))
            .unwrap_or("auto");
        match kind {
            "none" => {
                result.as_object_mut().unwrap().remove("tools");
            }
            "required" => result["tool_choice"] = json!({"type":"any"}),
            "function" => {
                let name = choice
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("工具选择缺少名称")?;
                result["tool_choice"] = json!({"type":"tool","name":to_upstream.get(name).map(String::as_str).unwrap_or(name)});
            }
            _ => {}
        }
    }
    Ok(Translation {
        body: result,
        to_original,
    })
}

fn output_items(message: &Value, names: &HashMap<String, String>) -> Vec<Value> {
    let mut output = Vec::new();
    let mut text = String::new();
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    text.push_str(block.get("text").and_then(Value::as_str).unwrap_or(""))
                }
                Some("tool_use") => {
                    if !text.is_empty() {
                        output.push(json!({"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]}));
                        text = String::new();
                    }
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    output.push(json!({"id":format!("fc_{}",uuid::Uuid::new_v4().simple()),"type":"function_call","status":"completed","call_id":block.get("id"),"name":names.get(name).map(String::as_str).unwrap_or(name),"arguments":block.get("input").map(Value::to_string).unwrap_or_else(|| "{}".into())}));
                }
                _ => {}
            }
        }
    }
    if !text.is_empty() || output.is_empty() {
        output.push(json!({"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]}));
    }
    output
}

fn response_object(message: &Value, model: &str, names: &HashMap<String, String>) -> Value {
    json!({
        "id":format!("resp_{}",uuid::Uuid::new_v4().simple()),
        "object":"response", "created_at":now(), "status":"completed", "model":model,
        "output":output_items(message, names),
        "usage":{"input_tokens":message.pointer("/usage/input_tokens").and_then(Value::as_u64).unwrap_or(0),
                 "output_tokens":message.pointer("/usage/output_tokens").and_then(Value::as_u64).unwrap_or(0),
                 "total_tokens":message.pointer("/usage/input_tokens").and_then(Value::as_u64).unwrap_or(0)+message.pointer("/usage/output_tokens").and_then(Value::as_u64).unwrap_or(0)}
    })
}

fn responses(
    stream: &mut TcpStream,
    base_url: &str,
    key: &str,
    request: &Value,
) -> Result<(), String> {
    let converted = match anthropic_request(request) {
        Ok(value) => value,
        Err(message) => return error(stream, 422, &message).map_err(|e| e.to_string()),
    };
    let model = request.get("model").and_then(Value::as_str).unwrap_or("");
    let upstream = match client(base_url).and_then(|client| {
        client
            .post(format!("{base_url}/messages"))
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&converted.body)
            .send()
            .map_err(|e| e.to_string())
    }) {
        Ok(upstream) => upstream,
        Err(message) => {
            return error(
                stream,
                502,
                &format!("连接对话网关失败：{}", message.replace(key, "[REDACTED]")),
            )
            .map_err(|e| e.to_string())
        }
    };
    if !upstream.status().is_success() {
        let code = upstream.status().as_u16();
        let body = upstream
            .text()
            .unwrap_or_default()
            .replace(key, "[REDACTED]");
        return response(stream, code, &body).map_err(|e| e.to_string());
    }
    if converted.body["stream"] == false {
        let message: Value = upstream.json().map_err(|e| e.to_string())?;
        return response(
            stream,
            200,
            &response_object(&message, model, &converted.to_original).to_string(),
        )
        .map_err(|e| e.to_string());
    }
    stream_response(stream, upstream, model, converted.to_original)
}

fn event(
    stream: &mut TcpStream,
    kind: &str,
    mut value: Value,
    sequence: &mut u64,
) -> Result<(), String> {
    *sequence += 1;
    value["type"] = Value::String(kind.to_owned());
    value["sequence_number"] = json!(*sequence);
    write!(stream, "event: {kind}\ndata: {value}\n\n").map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

struct StreamState {
    id: String,
    model: String,
    output: Vec<Value>,
    text_index: Option<usize>,
    text: String,
    tool_blocks: std::collections::HashMap<usize, usize>,
    input_tokens: u64,
    output_tokens: u64,
    sequence: u64,
    completed: bool,
    names: HashMap<String, String>,
}

impl StreamState {
    fn new(model: &str, names: HashMap<String, String>) -> Self {
        Self {
            id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
            model: model.into(),
            output: Vec::new(),
            text_index: None,
            text: String::new(),
            tool_blocks: Default::default(),
            input_tokens: 0,
            output_tokens: 0,
            sequence: 0,
            completed: false,
            names,
        }
    }

    fn start(&mut self, stream: &mut TcpStream) -> Result<(), String> {
        let response = json!({"id":self.id,"object":"response","created_at":now(),"status":"in_progress","model":self.model,"output":[]});
        event(
            stream,
            "response.created",
            json!({"response":response}),
            &mut self.sequence,
        )?;
        event(
            stream,
            "response.in_progress",
            json!({"response":response}),
            &mut self.sequence,
        )
    }

    fn text_item(&mut self, stream: &mut TcpStream) -> Result<usize, String> {
        if let Some(index) = self.text_index {
            return Ok(index);
        }
        let index = self.output.len();
        let item = json!({"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant","status":"in_progress","content":[]});
        event(
            stream,
            "response.output_item.added",
            json!({"output_index":index,"item":item}),
            &mut self.sequence,
        )?;
        event(
            stream,
            "response.content_part.added",
            json!({"item_id":item["id"],"output_index":index,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            &mut self.sequence,
        )?;
        self.output.push(item);
        self.text_index = Some(index);
        Ok(index)
    }

    fn consume(&mut self, stream: &mut TcpStream, value: &Value) -> Result<(), String> {
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                self.input_tokens = value
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
            }
            "content_block_start" => {
                let block = &value["content_block"];
                if block["type"] == "text" {
                    self.text_item(stream)?;
                    if let Some(text) = block
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|x| !x.is_empty())
                    {
                        self.text_delta(stream, text)?;
                    }
                } else if block["type"] == "tool_use" {
                    let index = self.output.len();
                    let id = value["index"].as_u64().ok_or("工具事件缺少序号")? as usize;
                    let upstream_name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    let name = self
                        .names
                        .get(upstream_name)
                        .map(String::as_str)
                        .unwrap_or(upstream_name);
                    let item = json!({"id":format!("fc_{}",uuid::Uuid::new_v4().simple()),"type":"function_call","status":"in_progress","call_id":block["id"],"name":name,"arguments":""});
                    event(
                        stream,
                        "response.output_item.added",
                        json!({"output_index":index,"item":item}),
                        &mut self.sequence,
                    )?;
                    self.output.push(item);
                    self.tool_blocks.insert(id, index);
                }
            }
            "content_block_delta" => {
                let delta = &value["delta"];
                if delta["type"] == "text_delta" {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        self.text_delta(stream, text)?;
                    }
                } else if delta["type"] == "input_json_delta" {
                    let block_index =
                        value["index"].as_u64().ok_or("工具参数事件缺少序号")? as usize;
                    if let Some(&output_index) = self.tool_blocks.get(&block_index) {
                        let text = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let current = self.output[output_index]["arguments"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned();
                        self.output[output_index]["arguments"] = Value::String(current + text);
                        event(
                            stream,
                            "response.function_call_arguments.delta",
                            json!({"item_id":self.output[output_index]["id"],"output_index":output_index,"delta":text}),
                            &mut self.sequence,
                        )?;
                    }
                }
            }
            "content_block_stop" => {
                if let Some(block_index) = value["index"].as_u64().map(|x| x as usize) {
                    if let Some(index) = self.tool_blocks.remove(&block_index) {
                        let item = &mut self.output[index];
                        if item["arguments"] == "" {
                            item["arguments"] = Value::String("{}".into());
                        }
                        item["status"] = Value::String("completed".into());
                        event(
                            stream,
                            "response.function_call_arguments.done",
                            json!({"item_id":item["id"],"output_index":index,"name":item["name"],"arguments":item["arguments"]}),
                            &mut self.sequence,
                        )?;
                        event(
                            stream,
                            "response.output_item.done",
                            json!({"output_index":index,"item":item}),
                            &mut self.sequence,
                        )?;
                    }
                }
            }
            "message_delta" => {
                self.output_tokens = value
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.output_tokens);
            }
            "message_stop" => self.finish(stream)?,
            "error" => {
                let message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("网关返回流式错误");
                event(
                    stream,
                    "response.failed",
                    json!({"response":{"id":self.id,"object":"response","status":"failed","model":self.model,"error":{"message":message,"type":"upstream_error"}}}),
                    &mut self.sequence,
                )?;
                self.completed = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn text_delta(&mut self, stream: &mut TcpStream, delta: &str) -> Result<(), String> {
        let index = self.text_item(stream)?;
        self.text.push_str(delta);
        event(
            stream,
            "response.output_text.delta",
            json!({"item_id":self.output[index]["id"],"output_index":index,"content_index":0,"delta":delta}),
            &mut self.sequence,
        )
    }

    fn finish(&mut self, stream: &mut TcpStream) -> Result<(), String> {
        if self.completed {
            return Ok(());
        }
        if let Some(index) = self.text_index {
            let item = &mut self.output[index];
            item["status"] = Value::String("completed".into());
            item["content"] = json!([{"type":"output_text","text":self.text,"annotations":[]}]);
            event(
                stream,
                "response.output_text.done",
                json!({"item_id":item["id"],"output_index":index,"content_index":0,"text":self.text}),
                &mut self.sequence,
            )?;
            event(
                stream,
                "response.content_part.done",
                json!({"item_id":item["id"],"output_index":index,"content_index":0,"part":item["content"][0]}),
                &mut self.sequence,
            )?;
            event(
                stream,
                "response.output_item.done",
                json!({"output_index":index,"item":item}),
                &mut self.sequence,
            )?;
        }
        let response = json!({"id":self.id,"object":"response","created_at":now(),"status":"completed","model":self.model,"output":self.output,
            "usage":{"input_tokens":self.input_tokens,"output_tokens":self.output_tokens,"total_tokens":self.input_tokens+self.output_tokens}});
        event(
            stream,
            "response.completed",
            json!({"response":response}),
            &mut self.sequence,
        )?;
        self.completed = true;
        Ok(())
    }
}

fn stream_response(
    stream: &mut TcpStream,
    upstream: reqwest::blocking::Response,
    model: &str,
    names: HashMap<String, String>,
) -> Result<(), String> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut state = StreamState::new(model, names);
    state.start(stream)?;
    let mut reader = BufReader::new(upstream);
    let mut data = String::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if let Some(chunk) = line.strip_prefix("data:") {
            data.push_str(chunk.trim());
        } else if line == "\r\n" || line == "\n" {
            if !data.is_empty() {
                if let Ok(value) = serde_json::from_str::<Value>(&data) {
                    state.consume(stream, &value)?;
                }
                data.clear();
            }
        }
    }
    if !data.is_empty() {
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            state.consume(stream, &value)?;
        }
    }
    if !state.completed {
        state.finish(stream)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_text_and_tool_round_trip_without_losing_call_ids() {
        let request = json!({"model":"claude-opus-4-8","stream":true,"instructions":"Help with code", "input":[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"List files"}]},
            {"type":"function_call","name":"shell_command","call_id":"call_7","arguments":"{\"cmd\":\"ls\"}"},
            {"type":"function_call_output","call_id":"call_7","output":"a.txt"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"Summarize"}]}
        ],"tools":[{"type":"function","name":"shell_command","description":"Run shell","parameters":{"type":"object","properties":{"cmd":{"type":"string"}}}}]});
        let mapped = anthropic_request(&request).unwrap();
        assert_eq!(mapped.body["system"], "Help with code");
        assert_eq!(mapped.body["messages"][1]["content"][0]["id"], "call_7");
        assert_eq!(
            mapped.body["messages"][2]["content"][0]["tool_use_id"],
            "call_7"
        );
        assert_eq!(
            mapped.body["tools"][0]["input_schema"]["properties"]["cmd"]["type"],
            "string"
        );
        let result = response_object(
            &json!({"content":[{"type":"text","text":"Done"},{"type":"tool_use","id":"toolu_5","name":"pathmux_tool_0","input":{"cmd":"pwd"}}],"usage":{"input_tokens":2,"output_tokens":3}}),
            "claude-opus-4-8",
            &mapped.to_original,
        );
        assert_eq!(result["output"][0]["content"][0]["text"], "Done");
        assert_eq!(result["output"][1]["call_id"], "toolu_5");
        assert_eq!(result["output"][1]["name"], "shell_command");
        assert_eq!(result["output"][1]["arguments"], "{\"cmd\":\"pwd\"}");
    }

    #[test]
    fn flattens_namespace_tools_and_restores_original_names() {
        let request = json!({"model":"m","input":"hi","tools":[{"type":"namespace","name":"functions","tools":[{"type":"function","name":"exec","parameters":{"type":"object","properties":{}}}]}]});
        let mapped = anthropic_request(&request).unwrap();
        assert_eq!(mapped.body["tools"][0]["name"], "pathmux_tool_0");
        assert_eq!(mapped.to_original["pathmux_tool_0"], "functions.exec");
    }

    #[test]
    fn rejects_unsupported_input_instead_of_silently_dropping_it() {
        let request = json!({"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AA=="}]}]});
        assert!(anthropic_request(&request)
            .unwrap_err()
            .contains("input_image"));
    }
}
