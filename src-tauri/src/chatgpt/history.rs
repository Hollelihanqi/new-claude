use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};

const MAX_ROLLOUT: u64 = 32 * 1024 * 1024;
const MAX_SCAN: usize = 10_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    key: String,
    thread_id: String,
    title: String,
    revision: String,
    bytes: u64,
    modified_at: u64,
    transferable: bool,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryList {
    items: Vec<HistoryItem>,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub source_id: String,
    pub target_id: String,
    pub key: String,
    pub revision: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferResult {
    pub target_thread_id: String,
    pub duplicate: bool,
    pub detail: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferJournal {
    source_id: String,
    source_thread_id: String,
    snapshot_hash: String,
    target_thread_id: Option<String>,
    state: String,
}

fn source_home(root: &Path, r: &Registry, id: &str) -> Result<PathBuf, String> {
    if id == "default" {
        let path = dirs::home_dir()
            .ok_or("无法定位默认工作记录")?
            .join(".codex");
        storage::plain(&path)?;
        return Ok(path);
    }
    selected(r, id)?;
    Ok(storage::profile_dir(root, id)?.join("codex"))
}

fn paths(home: &Path) -> Result<(Vec<PathBuf>, bool), String> {
    let mut pending: Vec<(PathBuf, usize)> = ["sessions", "archived_sessions"]
        .into_iter()
        .map(|p| (home.join(p), 0))
        .collect();
    let mut files = Vec::new();
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        if !dir.exists() {
            continue;
        }
        storage::plain(&dir)?;
        if depth > 8 {
            return Err("工作记录目录层级异常".into());
        }
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            visited += 1;
            if visited > MAX_SCAN {
                return Ok((files, true));
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if storage::linked(&meta) {
                continue;
            }
            if meta.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if meta.is_file() && entry.file_name().to_string_lossy().starts_with("rollout-")
            {
                files.push(entry.path());
            }
        }
    }
    Ok((files, false))
}

fn revision(meta: &fs::Metadata) -> String {
    format!(
        "{}-{}",
        meta.len(),
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

fn inspect(home: &Path, path: &Path) -> Result<HistoryItem, String> {
    storage::plain(path)?;
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let relative = path
        .strip_prefix(home)
        .map_err(|_| "会话路径不属于来源实例")?;
    let mut item = HistoryItem {
        key: hex::encode(Sha256::digest(relative.to_string_lossy().as_bytes())),
        thread_id: String::new(),
        title: "本地工作记录".into(),
        revision: revision(&meta),
        bytes: meta.len(),
        modified_at: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0),
        transferable: false,
        detail: "当前格式尚未通过迁移验证".into(),
    };
    if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        item.detail = "压缩记录暂不支持迁移".into();
        return Ok(item);
    }
    if meta.len() > MAX_ROLLOUT {
        item.detail = "记录超过 32 MB，暂不支持迁移".into();
        return Ok(item);
    }
    let mut first = String::new();
    BufReader::new(fs::File::open(path).map_err(|e| e.to_string())?)
        .take(1024 * 1024)
        .read_line(&mut first)
        .map_err(|e| e.to_string())?;
    let line: Value = serde_json::from_str(&first).map_err(|_| "会话头格式无法识别")?;
    if line["type"] != "session_meta" {
        return Ok(item);
    }
    let payload = &line["payload"];
    item.thread_id = payload["id"].as_str().unwrap_or("").into();
    if uuid::Uuid::parse_str(&item.thread_id).is_err() {
        item.detail = "会话标识无法识别".into();
        return Ok(item);
    }
    item.title = payload
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&item.thread_id)
        .chars()
        .take(100)
        .collect();
    if payload
        .get("history_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode != "legacy")
    {
        item.detail = "分页历史依赖额外数据，目前禁止不完整复制".into();
        return Ok(item);
    }
    if payload.get("forked_from_id").is_some_and(|v| !v.is_null()) {
        item.detail = "分支记录可能依赖父会话，暂不支持独立迁移".into();
        return Ok(item);
    }
    item.transferable = true;
    item.detail = "可尝试本地历史分叉；附件与内部路径将在导入前再次检查".into();
    Ok(item)
}

pub fn list(root: &Path, r: &Registry, source_id: &str) -> Result<HistoryList, String> {
    let home = source_home(root, r, source_id)?;
    let (paths, truncated) = paths(&home)?;
    let mut warnings = vec![
        "仅支持可独立读取的本地历史。分页历史、云端记录和依赖父会话的分支尚不支持完整接续。".into(),
    ];
    let mut items = Vec::new();
    let mut invalid = 0;
    for p in paths {
        match inspect(&home, &p) {
            Ok(item) => items.push(item),
            Err(_) => invalid += 1,
        }
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.modified_at));
    if truncated {
        warnings.push("扫描达到安全上限，列表可能不完整。".into());
    }
    if invalid > 0 {
        warnings.push(format!("有 {invalid} 条记录无法读取，已跳过。"));
    }
    if items.len() > 300 {
        items.truncate(300);
        warnings.push("当前显示最近 300 条记录。".into());
    }
    Ok(HistoryList { items, warnings })
}

fn validate_snapshot(bytes: &[u8], home: &Path) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "会话内容编码不受支持")?;
    if !text.ends_with('\n') {
        return Err("会话仍在写入或数据不完整，请关闭来源实例后重试".into());
    }
    let source = home.to_string_lossy();
    // Internal paths need an attachment/path migration adapter. Do not silently
    // leave the new thread dependent on a profile that may later be deleted.
    if text.contains(source.as_ref()) || text.contains(&source.replace('\\', "\\\\")) {
        return Err("记录引用来源实例内部文件，当前不能保证独立附件接续".into());
    }
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).map_err(|_| "会话包含不完整记录")?;
        if has_external_attachment(&v) {
            return Err("记录包含外部附件或云端文件引用，当前不支持完整迁移".into());
        }
    }
    Ok(())
}

fn has_external_attachment(v: &Value) -> bool {
    match v {
        Value::Object(map) => map.iter().any(|(k, value)| {
            (matches!(
                k.as_str(),
                "file_id" | "fileId" | "local_image" | "localImage" | "attachment_id"
            ) && !value.is_null())
                || (k == "image_url" && value.as_str().is_none_or(|s| !s.starts_with("data:")))
                || (matches!(k.as_str(), "attachments" | "attachmentPaths")
                    && value.as_array().is_some_and(|a| !a.is_empty()))
                || has_external_attachment(value)
        }),
        Value::Array(a) => a.iter().any(has_external_attachment),
        _ => false,
    }
}

fn verify_copied_context(source: &[u8], target: &[u8]) -> Result<(), String> {
    fn context(bytes: &[u8]) -> Result<Vec<Value>, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "历史编码无法识别")?;
        let mut result = vec![];
        for line in text.lines() {
            let value: Value = serde_json::from_str(line).map_err(|_| "历史内容不完整")?;
            if matches!(value["type"].as_str(), Some("response_item" | "compacted")) {
                let mut payload = value["payload"].clone();
                // The official reader assigns IDs to legacy messages lacking one.
                // IDs identify items; their text, role and tool references must match.
                if payload["type"] == "message" {
                    if let Some(object) = payload.as_object_mut() {
                        object.remove("id");
                    }
                }
                result.push(payload);
            }
        }
        Ok(result)
    }
    let source = context(source)?;
    let target = context(target)?;
    if source.is_empty() {
        return Err("记录中没有可验证的对话上下文".into());
    }
    let mut next = 0;
    for value in &target {
        if source.get(next) == Some(value) {
            next += 1;
        }
    }
    if next != source.len() {
        return Err("目标副本未保留完整的模型上下文，已停止迁移".into());
    }
    Ok(())
}

pub fn transfer(
    root: &Path,
    r: &Registry,
    request: TransferRequest,
) -> Result<TransferResult, String> {
    if request.source_id == request.target_id {
        return Err("请选择另一个目标实例".into());
    }
    selected(r, &request.target_id)?;
    let target = storage::profile_dir(root, &request.target_id)?;
    storage::verify_config(&target)?;
    let all = process::snapshot()?;
    process::require_stopped(&all, &target)?;
    let home = source_home(root, r, &request.source_id)?;
    let app = discovery::discover(r.installation.as_deref())?;
    if !app.compatible {
        return Err(app.detail);
    }
    if request.source_id == "default" {
        // Conservatively require all official main instances of this installation
        // to be closed before snapshotting the default profile.
        if all
            .iter()
            .any(|p| process::path_eq(&p.exe, Path::new(&app.executable), cfg!(windows)))
        {
            return Err("复制默认实例记录前，请先退出该客户端的所有窗口".into());
        }
    } else {
        process::require_stopped(&all, &storage::profile_dir(root, &request.source_id)?)?;
    }
    let cli = app
        .cli
        .as_deref()
        .ok_or("未找到该客户端内置会话服务，暂不能迁移")?;
    let mut source = None;
    for p in paths(&home)?.0 {
        if let Ok(item) = inspect(&home, &p) {
            if item.key == request.key {
                source = Some((p, item));
                break;
            }
        }
    }
    let (path, item) = source.ok_or("记录已移动或不存在，请刷新列表")?;
    if !item.transferable {
        return Err(item.detail);
    }
    if item.revision != request.revision {
        return Err("来源记录已变化，请刷新后重新选择".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(MAX_ROLLOUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ROLLOUT
        || revision(&fs::metadata(&path).map_err(|e| e.to_string())?) != item.revision
    {
        return Err("读取期间来源记录发生变化，已取消迁移".into());
    }
    validate_snapshot(&bytes, &home)?;
    let key = hex::encode(Sha256::digest(
        [request.source_id.as_bytes(), request.key.as_bytes(), &bytes].concat(),
    ));
    let transfers = target.join("transfers");
    storage::private_dir(&transfers)?;
    let record = transfers.join(format!("{key}.json"));
    let journal_path = transfers.join(format!("{key}.pending.json"));
    if record.exists() {
        let mut result: TransferResult = storage::read_json(&record)?;
        // The official client may have deleted the earlier imported thread.
        // Do not report an existing copy unless the server can still read it.
        let mut client = rpc::Client::start(cli, &target)?;
        client.call(
            "thread/read",
            json!({"threadId":result.target_thread_id,"includeTurns":false}),
        )?;
        result.duplicate = true;
        result.detail = "这份历史快照已导入，目标后续进展保持原样。".into();
        return Ok(result);
    }
    if journal_path.exists() {
        return Err(format!(
            "上次迁移尚未确认完成，已阻止重复导入。请先检查目标会话及迁移登记：{}",
            journal_path.display()
        ));
    }
    let stage = transfers.join(format!("{key}.jsonl"));
    storage::plain(&stage)?;
    crate::sync::write_bytes_atomic(&stage, &bytes).map_err(|e| e.to_string())?;
    let mut journal = TransferJournal {
        source_id: request.source_id.clone(),
        source_thread_id: item.thread_id.clone(),
        snapshot_hash: key.clone(),
        target_thread_id: None,
        state: "pending".into(),
    };
    let result = (|| {
        let mut client = rpc::Client::start(cli, &target)?;
        storage::write_json(&journal_path, &journal)?;
        let result = client.call(
            "thread/fork",
            json!({"threadId": item.thread_id, "path":stage, "excludeTurns":true,
            "approvalPolicy":"on-request", "sandbox":"read-only", "deferGoalContinuation":true}),
        )?;
        let id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or("客户端没有返回新会话标识")?
            .to_owned();
        if id == item.thread_id || uuid::Uuid::parse_str(&id).is_err() {
            return Err("客户端未创建独立会话，已停止操作".into());
        }
        journal.target_thread_id = Some(id.clone());
        let finalize: Result<TransferResult, String> = (|| {
            storage::write_json(&journal_path, &journal)?;
            client.call("thread/read", json!({"threadId":id,"includeTurns":false}))?;
            let fork_path = result
                .pointer("/thread/path")
                .and_then(Value::as_str)
                .ok_or("客户端没有返回独立历史路径")?;
            storage::plain(Path::new(fork_path))?;
            let canonical = fs::canonicalize(fork_path).map_err(|_| "目标历史文件尚未写入")?;
            if !canonical.starts_with(
                target
                    .join("codex")
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
            ) {
                return Err("目标会话没有写入目标实例目录".into());
            }
            let mut copied = Vec::new();
            fs::File::open(&canonical)
                .map_err(|e| e.to_string())?
                .take(MAX_ROLLOUT + 1)
                .read_to_end(&mut copied)
                .map_err(|e| e.to_string())?;
            if copied.len() as u64 > MAX_ROLLOUT {
                return Err("目标历史大小超出验证上限".into());
            }
            verify_copied_context(&bytes, &copied)?;
            let name = format!("接续 · {}", item.title);
            client.call("thread/name/set", json!({"threadId":id,"name":name}))?;
            let result = TransferResult { target_thread_id: id.clone(), duplicate: false,
                detail: "官方客户端已创建并读取独立会话副本。请启动目标实例核对上下文并续聊；尚未验证真实账号续聊。项目文件仍使用原工作目录。".into() };
            storage::write_json(&record, &result)?;
            Ok(result)
        })();
        match finalize {
            Ok(result) => {
                let _ = fs::remove_file(&journal_path);
                Ok(result)
            }
            Err(error) => {
                // Roll back only the newly returned target ID, never the source.
                if client.call("thread/delete", json!({"threadId":id})).is_ok() {
                    let _ = fs::remove_file(&journal_path);
                    Err(format!("导入未完成，已撤回新建副本：{error}"))
                } else {
                    journal.state = "needsReview".into();
                    let _ = storage::write_json(&journal_path, &journal);
                    Err(format!(
                        "导入未完成且无法确认回滚。已保留登记并阻止重复导入，请检查目标会话 {id}。"
                    ))
                }
            }
        }
    })();
    let _ = fs::remove_file(stage);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_incomplete_and_external_history() {
        let home = Path::new("/profiles/source/codex");
        assert!(validate_snapshot(b"{}", home).is_err());
        assert!(validate_snapshot(b"{\"file_id\":\"file-secret\"}\n", home).is_err());
        assert!(validate_snapshot(
            b"{\"path\":\"/profiles/source/codex/attachments/x\"}\n",
            home
        )
        .is_err());
        assert!(
            validate_snapshot(b"{\"image_url\":\"data:image/png;base64,test\"}\n", home).is_ok()
        );
    }
    #[test]
    fn paginated_and_parent_dependent_threads_are_not_advertised_as_transferable() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().canonicalize().unwrap();
        let path = home.join("rollout-test.jsonl");
        for mode in ["paginated", "unknown"] {
            fs::write(&path, format!("{}\n", json!({"type":"session_meta","payload":{"id":uuid::Uuid::new_v4(),"history_mode":mode}}))).unwrap();
            assert!(!inspect(&home, &path).unwrap().transferable);
        }
        fs::write(&path, format!("{}\n", json!({"type":"session_meta","payload":{"id":uuid::Uuid::new_v4(),"history_mode":"legacy"}}))).unwrap();
        assert!(inspect(&home, &path).unwrap().transferable);
    }

    #[test]
    fn context_validation_detects_missing_or_reordered_messages() {
        let a = b"{\"type\":\"response_item\",\"payload\":1}\n{\"type\":\"response_item\",\"payload\":2}\n";
        assert!(verify_copied_context(a, a).is_ok());
        assert!(verify_copied_context(a, b"{\"type\":\"response_item\",\"payload\":1}\n").is_err());
        assert!(verify_copied_context(a, b"{\"type\":\"response_item\",\"payload\":2}\n{\"type\":\"response_item\",\"payload\":1}\n").is_err());
    }

    #[test]
    #[ignore = "requires installed official ChatGPT; uses only temporary synthetic history, no login or inference"]
    fn official_client_migrates_legacy_context_without_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        registry.installation = std::env::var("PATHMUX_TEST_CHATGPT_APP").ok();
        let app = discovery::discover(registry.installation.as_deref()).unwrap();
        let a = storage::create(&root, &mut registry, "Source").unwrap();
        let b = storage::create(&root, &mut registry, "Target").unwrap();
        let adir = storage::profile_dir(&root, &a.id).unwrap();
        let bdir = storage::profile_dir(&root, &b.id).unwrap();
        storage::private_dir(&adir.join("codex/sessions")).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let fixture = [
            json!({"type":"session_meta","payload":{"id":id,"timestamp":"2026-09-28T00:00:00Z","cwd":root,"originator":"codex_cli_rs","cli_version":"0.100.0","source":"cli","model_provider":"openai"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Remember test color: cyan."}]}}),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"Remember test color: cyan.","images":[],"local_images":[],"text_elements":[]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"The test color is cyan."}]}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"The test color is cyan."}}),
        ].into_iter().map(|mut v| { v["timestamp"] = json!("2026-09-28T00:00:00Z"); format!("{v}\n") }).collect::<String>();
        let source = adir
            .join("codex/sessions")
            .join(format!("rollout-{id}.jsonl"));
        fs::write(&source, &fixture).unwrap();
        fs::write(adir.join("codex/auth.json"), "test-only-source-secret").unwrap();
        let entries = list(&root, &registry, &a.id).unwrap();
        let item = &entries.items[0];
        assert!(item.transferable);
        let request = || TransferRequest {
            source_id: a.id.clone(),
            target_id: b.id.clone(),
            key: item.key.clone(),
            revision: item.revision.clone(),
        };
        let result = transfer(&root, &registry, request()).unwrap();
        assert_ne!(result.target_thread_id, id);
        assert!(!result.duplicate);
        let duplicate = transfer(&root, &registry, request()).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.target_thread_id, result.target_thread_id);
        assert_eq!(fs::read_to_string(&source).unwrap(), fixture);
        assert!(!bdir.join("codex/auth.json").exists());
        fs::remove_dir_all(&adir).unwrap();
        let mut client = rpc::Client::start(app.cli.as_deref().unwrap(), &bdir).unwrap();
        let read = client
            .call(
                "thread/read",
                json!({"threadId":result.target_thread_id,"includeTurns":true}),
            )
            .unwrap();
        assert!(
            read.to_string().contains("cyan"),
            "Imported dialogue must remain readable after deleting the source"
        );
    }
}
