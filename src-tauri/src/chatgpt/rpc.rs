//! Bounded, short-lived official app-server connection. Never starts a model turn.
use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub struct Client {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Result<Value, String>>,
    next_id: u64,
}

impl Drop for Client {
    fn drop(&mut self) {
        // EOF lets the server flush pending rollout writes before exit.
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Client {
    pub fn start(cli: &str, dir: &Path) -> Result<Self, String> {
        let mut command = Command::new(cli);
        process::configure(&mut command, dir);
        let mut child = process::quiet(&mut command)
            .args([
                "--config",
                &format!(
                    "sqlite_home={}",
                    serde_json::to_string(&dir.join("codex/db").to_string_lossy()).unwrap()
                ),
            ])
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("无法启动官方会话服务：{e}"))?;
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let result = reader
                    .by_ref()
                    .take(16 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut bytes);
                if matches!(result, Ok(0)) {
                    break;
                }
                let value = if result.is_err() || bytes.len() > 16 * 1024 * 1024 {
                    Err("官方会话服务输出异常或超出限制".into())
                } else {
                    serde_json::from_slice::<Value>(&bytes)
                        .map_err(|_| "官方会话服务返回了无法识别的数据".into())
                };
                let stop = value.is_err();
                if tx.send(value).is_err() || stop {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input: Some(input),
            messages,
            next_id: 1,
        };
        let initialized = client.call(
            "initialize",
            json!({
                "clientInfo":{"name":"pathmux_history","version":env!("CARGO_PKG_VERSION")},
                "capabilities":{"experimentalApi":true}
            }),
        )?;
        let home = initialized
            .get("codexHome")
            .and_then(Value::as_str)
            .ok_or("客户端未返回数据目录，无法确认隔离")?;
        if !process::path_eq(Path::new(home), &dir.join("codex"), cfg!(windows)) {
            return Err("官方会话服务使用了其他数据目录，已停止迁移".into());
        }
        client.send(json!({"method":"initialized"}))?;
        Ok(client)
    }

    fn send(&mut self, value: Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        let input = self.input.as_mut().ok_or("会话服务已关闭")?;
        input
            .write_all(&bytes)
            .and_then(|_| input.write_all(b"\n"))
            .and_then(|_| input.flush())
            .map_err(|e| e.to_string())
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or("官方会话服务响应超时")?;
            let value = self
                .messages
                .recv_timeout(remaining)
                .map_err(|_| "官方会话服务已退出或响应超时")??;
            if value.get("method").is_some() {
                if let Some(request_id) = value.get("id") {
                    self.send(json!({"id":request_id,"error":{"code":-32000,"message":"PathMux migration does not approve or execute tools"}}))?;
                }
                continue;
            }
            if value.get("id") == Some(&json!(id)) {
                // Avoid returning raw server errors that can contain conversation content.
                if value.get("error").is_some() {
                    return Err(format!("官方客户端拒绝 {method}，该记录或客户端版本可能不支持迁移。原始记录保持完整。"));
                }
                return value
                    .get("result")
                    .cloned()
                    .ok_or("官方客户端没有返回操作结果".into());
            }
        }
    }
}
