use super::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;

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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub source_id: String,
    pub target_id: String,
    pub key: String,
    pub revision: String,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    stage_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content_hash: String,
}

fn source_home(root: &Path, r: &Registry, id: &str) -> Result<PathBuf, String> {
    if id == "default" {
        let path = crate::home().join(".codex");
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

fn path_key(home: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(home)
        .map_err(|_| "会话路径不属于来源实例")?;
    Ok(hex::encode(Sha256::digest(
        relative.to_string_lossy().as_bytes(),
    )))
}

fn inspect(home: &Path, path: &Path) -> Result<HistoryItem, String> {
    storage::plain(path)?;
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;

    let mut item = HistoryItem {
        key: path_key(home, path)?,
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
    let bytes = snapshot::read(path)?;
    let first = bytes.split(|b| *b == b'\n').next().ok_or("记录为空")?;
    let line: Value = serde_json::from_slice(first).map_err(|_| "会话头格式无法识别")?;
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
        .is_some_and(|mode| !matches!(mode, "legacy" | "paginated"))
    {
        item.detail = "未知的历史格式，请更新适配器".into();
        return Ok(item);
    }
    if payload.get("title").and_then(Value::as_str).is_none() {
        for line in bytes.split(|b| *b == b'\n').take(300) {
            if let Ok(value) = serde_json::from_slice::<Value>(line) {
                if value["type"] == "event_msg" && value["payload"]["type"] == "user_message" {
                    if let Some(text) = value["payload"]["message"].as_str() {
                        item.title = text
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(100)
                            .collect();
                        break;
                    }
                }
            }
        }
    }
    item.transferable = true;
    item.detail = "支持独立复制；完整父链与附件将在复制前检查".into();
    Ok(item)
}

pub fn list(root: &Path, r: &Registry, source_id: &str) -> Result<HistoryList, String> {
    let home = source_home(root, r, source_id)?;
    let (mut paths, truncated) = paths(&home)?;
    paths.sort_by_key(|p| std::cmp::Reverse(fs::metadata(p).and_then(|m| m.modified()).ok()));
    let more = paths.len() > 300;
    paths.truncate(300);
    let mut warnings =
        vec!["支持本地旧版、分页、分支与 zstd 压缩记录。云端文件权限不能跨账号复制。".into()];
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
    if more {
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
                || (matches!(k.as_str(), "file_ids" | "local_audio")
                    && value.as_array().is_some_and(|a| !a.is_empty()))
                || (k == "audio" && value.as_array().is_some_and(|a| !a.is_empty()))
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

fn transfer_key(request: &TransferRequest, fingerprint: &str) -> String {
    hex::encode(Sha256::digest(format!(
        "{}:{}:{}",
        request.source_id, request.key, fingerprint
    )))
}

pub(super) fn has_pending(root: &Path, request: &TransferRequest) -> bool {
    request.fingerprint.as_ref().is_some_and(|fingerprint| {
        storage::profile_dir(root, &request.target_id)
            .map(|target| {
                target
                    .join("transfers")
                    .join(format!(
                        "{}.pending.json",
                        transfer_key(request, fingerprint)
                    ))
                    .exists()
            })
            .unwrap_or(false)
    })
}

fn existing_transfer(
    cli: &str,
    target: &Path,
    key: &str,
) -> Result<Option<TransferResult>, String> {
    let transfers = target.join("transfers");
    let record = transfers.join(format!("{key}.json"));
    if record.exists() {
        let mut result: TransferResult = storage::read_json(&record)?;
        let mut client = rpc::Client::start(cli, target)?;
        client.call(
            "thread/read",
            json!({"threadId":result.target_thread_id,"includeTurns":false}),
        )?;
        result.duplicate = true;
        result.detail = "这份快照已导入，目标后续进展保持原样。".into();
        return Ok(Some(result));
    }
    if transfers.join(format!("{key}.pending.json")).exists() {
        return resume_transfer(cli, target, key, false).map(Some);
    }
    Ok(None)
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
    let app = discovery::discover(r.installation.as_deref())?;
    if !app.compatible {
        return Err(app.detail);
    }
    let cli = app
        .cli
        .as_deref()
        .ok_or("未找到该客户端内置会话服务，暂不能迁移")?;
    if let Some(fingerprint) = &request.fingerprint {
        let key = transfer_key(&request, fingerprint);
        if let Some(result) = existing_transfer(cli, &target, &key)? {
            return Ok(result);
        }
    }
    let home = source_home(root, r, &request.source_id)?;
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
    let mut source = None;
    for p in paths(&home)?.0 {
        if path_key(&home, &p)? != request.key {
            continue;
        }
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
    let candidates = paths(&home)?.0;
    let raw = snapshot::materialize(&path, &candidates)?;
    let (bytes, _, _) = prepare(raw, &home, request.workspace.as_deref())?;
    if request
        .fingerprint
        .as_ref()
        .is_some_and(|hash| *hash != hex::encode(Sha256::digest(&bytes)))
    {
        return Err("预览后记录或附件已变化，请重新预览".into());
    }
    if revision(&fs::metadata(&path).map_err(|e| e.to_string())?) != item.revision {
        return Err("读取期间来源记录发生变化，请刷新后重试".into());
    }
    validate_snapshot(&bytes, &home)?;
    let key = transfer_key(&request, &hex::encode(Sha256::digest(&bytes)));
    let transfers = target.join("transfers");
    storage::private_dir(&transfers)?;
    let journal_path = transfers.join(format!("{key}.pending.json"));
    if let Some(result) = existing_transfer(cli, &target, &key)? {
        return Ok(result);
    }
    let stage = transfers.join(format!("{key}.jsonl"));
    let stage_id = uuid::Uuid::new_v4().to_string();
    let mut lines: Vec<Value> = std::str::from_utf8(&bytes)
        .map_err(|_| "记录编码无效")?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|_| "记录内容无效")?;
    lines[0]["payload"]["id"] = json!(stage_id);
    lines[0]["payload"]["session_id"] = json!(stage_id);
    let mut staged = Vec::new();
    for line in lines {
        serde_json::to_writer(&mut staged, &line).map_err(|e| e.to_string())?;
        staged.push(b'\n');
    }
    crate::sync::write_bytes_atomic(&stage, &staged).map_err(|e| e.to_string())?;
    let journal = TransferJournal {
        source_id: request.source_id,
        source_thread_id: item.thread_id,
        snapshot_hash: key.clone(),
        target_thread_id: None,
        state: "pending".into(),
        stage_id,
        title: item.title,
        content_hash: hex::encode(Sha256::digest(&staged)),
    };
    storage::write_json(&journal_path, &journal)?;
    resume_transfer(cli, &target, &key, false)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub key: String,
    pub title: String,
    pub bytes: usize,
    pub images: usize,
    pub workspace: String,
    pub fingerprint: String,
}

pub fn preview(root: &Path, r: &Registry, request: &TransferRequest) -> Result<Preview, String> {
    let home = source_home(root, r, &request.source_id)?;
    let candidates = paths(&home)?.0;
    let (path, item) = candidates
        .iter()
        .filter(|p| path_key(&home, p).ok().as_deref() == Some(&request.key))
        .find_map(|p| {
            inspect(&home, p)
                .ok()
                .filter(|i| i.key == request.key)
                .map(|i| (p, i))
        })
        .ok_or("记录不存在，请刷新")?;
    if item.revision != request.revision {
        return Err("记录已变化，请刷新".into());
    }
    let (bytes, images, workspace) = prepare(
        snapshot::materialize(path, &candidates)?,
        &home,
        request.workspace.as_deref(),
    )?;
    Ok(Preview {
        key: item.key,
        title: item.title,
        bytes: bytes.len(),
        images,
        workspace,
        fingerprint: hex::encode(Sha256::digest(bytes)),
    })
}

fn prepare(
    raw: Vec<u8>,
    home: &Path,
    workspace: Option<&str>,
) -> Result<(Vec<u8>, usize, String), String> {
    let mut records = std::str::from_utf8(&raw)
        .map_err(|_| "历史编码无效")?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "历史内容无效")?;
    let original = records[0]["payload"]["cwd"]
        .as_str()
        .ok_or("记录没有项目目录")?;
    let cwd = PathBuf::from(workspace.unwrap_or(original));
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("原项目目录不存在，请先选择接续使用的项目目录".into());
    }
    let cwd = cwd.canonicalize().map_err(|e| e.to_string())?;
    if cwd.starts_with(home.canonicalize().map_err(|e| e.to_string())?) {
        return Err("项目位于来源实例内部，请选择独立项目目录".into());
    }
    let mut images = 0;
    let mut output = vec![];
    for record in &mut records {
        if matches!(
            record["type"].as_str(),
            Some("session_meta" | "turn_context")
        ) && workspace.is_some()
        {
            record["payload"]["cwd"] = json!(cwd);
            if record["payload"].get("runtime_workspace_roots").is_some() {
                record["payload"]["runtime_workspace_roots"] = json!([cwd]);
            }
        }
        embed_images(record, &mut images)?;
        serde_json::to_writer(&mut output, record).map_err(|e| e.to_string())?;
        output.push(b'\n');
        if output.len() as u64 > MAX_ROLLOUT {
            return Err("历史和内嵌附件超过 32 MB 上限".into());
        }
    }
    validate_snapshot(&output, home)?;
    Ok((output, images, cwd.to_string_lossy().into()))
}

fn embed_images(value: &mut Value, count: &mut usize) -> Result<(), String> {
    use base64::Engine;
    fn data(path: &str) -> Result<String, String> {
        let p = Path::new(path);
        storage::plain(p)?;
        if !p.is_absolute() {
            return Err("附件必须使用绝对路径".into());
        }
        let mut bytes = vec![];
        fs::File::open(p)
            .map_err(|_| "本地图片附件缺失或无法读取")?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("单张图片超过 8 MB 上限".into());
        }
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            "image/gif"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else {
            return Err("附件不是受支持的 PNG/JPEG/GIF/WebP 图片".into());
        };
        Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
    match value {
        Value::Object(map) => {
            if let Some(local) = map.get("local_images").and_then(Value::as_array).cloned() {
                if !local.is_empty() {
                    let mut embedded = map
                        .get("images")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let original_count = embedded.len();
                    let mut details = map
                        .get("image_details")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let local_details = map
                        .get("local_image_details")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    while details.len() < original_count {
                        details.push(Value::Null);
                    }
                    for path in local {
                        embedded.push(json!(data(path.as_str().ok_or("图片路径无效")?)?));
                        *count += 1;
                    }
                    for index in 0..embedded.len() - original_count {
                        details.push(local_details.get(index).cloned().unwrap_or(Value::Null));
                    }
                    if !details.is_empty() {
                        map.insert("image_details".into(), json!(details));
                    }
                    map.insert("local_image_details".into(), json!([]));
                    if let Some(order) = map.get("image_order").and_then(Value::as_array) {
                        if !order.is_empty() {
                            let mut order = order.clone();
                            order.extend((original_count..embedded.len()).map(|_| json!("inline")));
                            map.insert("image_order".into(), json!(order));
                        }
                    }
                    map.insert("images".into(), json!(embedded));
                    map.insert("local_images".into(), json!([]));
                }
            }
            if map.get("type").and_then(Value::as_str) == Some("local_image") {
                let image = data(
                    map.get("path")
                        .and_then(Value::as_str)
                        .ok_or("图片路径缺失")?,
                )?;
                map.clear();
                map.insert("type".into(), json!("input_image"));
                map.insert("image_url".into(), json!(image));
                *count += 1;
            }
            if *count > 100 {
                return Err("单条记录图片超过 100 张上限".into());
            }
            for v in map.values_mut() {
                embed_images(v, count)?;
            }
        }
        Value::Array(array) => {
            for v in array {
                embed_images(v, count)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTransfer {
    key: String,
    title: String,
    state: String,
}

pub fn pending(root: &Path, r: &Registry, target_id: &str) -> Result<Vec<PendingTransfer>, String> {
    selected(r, target_id)?;
    let dir = storage::profile_dir(root, target_id)?.join("transfers");
    if !dir.exists() {
        return Ok(vec![]);
    }
    storage::plain(&dir)?;
    let mut result = vec![];
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.take(MAX_SCAN) {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(key) = name.strip_suffix(".jsonl") {
            if key.len() == 64
                && key.bytes().all(|b| b.is_ascii_hexdigit())
                && !entry
                    .path()
                    .with_file_name(format!("{key}.pending.json"))
                    .exists()
            {
                storage::plain(&entry.path())?;
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        if let Some(key) = name.strip_suffix(".pending.json") {
            if let Ok(journal) = storage::read_json::<TransferJournal>(&entry.path()) {
                result.push(PendingTransfer {
                    key: key.into(),
                    title: journal.title,
                    state: journal.state,
                });
            }
        }
    }
    Ok(result)
}

pub fn recover(
    root: &Path,
    r: &Registry,
    target_id: &str,
    key: &str,
    discard: bool,
) -> Result<TransferResult, String> {
    selected(r, target_id)?;
    let target = storage::profile_dir(root, target_id)?;
    storage::verify_config(&target)?;
    process::require_stopped(&process::snapshot()?, &target)?;
    let app = discovery::discover(r.installation.as_deref())?;
    if !app.compatible {
        return Err(app.detail);
    }
    resume_transfer(
        app.cli.as_deref().ok_or("客户端没有会话服务")?,
        &target,
        key,
        discard,
    )
}

fn resume_transfer(
    cli: &str,
    target: &Path,
    key: &str,
    discard: bool,
) -> Result<TransferResult, String> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("无效的复制标识".into());
    }
    let transfers = target.join("transfers");
    let journal_path = transfers.join(format!("{key}.pending.json"));
    let stage = transfers.join(format!("{key}.jsonl"));
    let record = transfers.join(format!("{key}.json"));
    let mut journal: TransferJournal = storage::read_json(&journal_path)?;
    storage::validate_id(&journal.stage_id)?;
    if record.exists() {
        let result = storage::read_json(&record)?;
        fs::remove_file(&journal_path).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(stage);
        return Ok(result);
    }
    // Recover an RPC response lost after the official client persisted a fork.
    // A fresh staging UUID uniquely identifies this import, including after a crash.
    let mut matches = vec![];
    for path in paths(&target.join("codex"))?.0 {
        // Other existing sessions may be damaged or much larger than this import.
        // Recovery only needs their bounded metadata header.
        let Ok(meta) = snapshot::header_file(&path) else {
            continue;
        };
        if meta["payload"]["forked_from_id"] == journal.stage_id {
            let id = meta["payload"]["id"]
                .as_str()
                .ok_or("目标会话标识缺失")?
                .to_owned();
            storage::validate_id(&id)?;
            matches.push(id);
        }
    }
    matches.sort();
    matches.dedup();
    if matches.len() > 1 {
        return Err("发现多个关联副本，已停止自动恢复以保留数据".into());
    }
    if let Some(id) = matches.pop() {
        journal.target_thread_id = Some(id);
    }
    let mut client = rpc::Client::start(cli, target)?;
    if discard {
        if let Some(id) = &journal.target_thread_id {
            client.call("thread/delete", json!({"threadId":id}))?;
        }
        fs::remove_file(&journal_path).map_err(|e| e.to_string())?;
        if stage.exists() {
            fs::remove_file(stage).map_err(|e| e.to_string())?;
        }
        return Ok(TransferResult {
            target_thread_id: String::new(),
            duplicate: false,
            detail: "未完成副本已撤回，来源及其他会话保留。".into(),
        });
    }
    let bytes = snapshot::read(&stage)?;
    if hex::encode(Sha256::digest(&bytes)) != journal.content_hash {
        return Err("恢复快照校验失败，请撤回后重新复制".into());
    }
    let id = match journal.target_thread_id.clone() {
        Some(id) => id,
        None => {
            let response = client.call("thread/fork", json!({"threadId":journal.stage_id,"path":stage,"excludeTurns":true,"approvalPolicy":"on-request","sandbox":"read-only","deferGoalContinuation":true}))?;
            let id = response
                .pointer("/thread/id")
                .and_then(Value::as_str)
                .ok_or("客户端没有返回新会话 ID")?
                .to_owned();
            storage::validate_id(&id)?;
            if id == journal.stage_id {
                return Err("客户端未创建独立副本".into());
            }
            journal.target_thread_id = Some(id.clone());
            storage::write_json(&journal_path, &journal)?;
            id
        }
    };
    let result: Result<TransferResult, String> = (|| {
        let read = client.call("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        let path = read
            .pointer("/thread/path")
            .and_then(Value::as_str)
            .ok_or("客户端未返回目标路径")?;
        storage::plain(Path::new(path))?;
        if !fs::canonicalize(path)
            .map_err(|e| e.to_string())?
            .starts_with(
                target
                    .join("codex")
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
            )
        {
            return Err("目标路径超出实例目录".into());
        }
        let copied = snapshot::read(Path::new(path))?;
        verify_copied_context(&bytes, &copied)?;
        client.call(
            "thread/name/set",
            json!({"threadId":id,"name":format!("接续 · {}", journal.title)}),
        )?;
        let result = TransferResult {
            target_thread_id: id,
            duplicate: false,
            detail: "已创建并校验独立会话副本。启动目标实例即可查看；项目文件仍使用原工作目录。"
                .into(),
        };
        storage::write_json(&record, &result)?;
        Ok(result)
    })();
    match result {
        Ok(result) => {
            fs::remove_file(journal_path).map_err(|e| e.to_string())?;
            fs::remove_file(stage).map_err(|e| e.to_string())?;
            Ok(result)
        }
        Err(error) => {
            journal.state = "needsReview".into();
            storage::write_json(&journal_path, &journal)?;
            Err(format!(
                "复制未完成：{error}。快照已保留，请从恢复列表继续或撤回。"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_incomplete_and_external_history() {
        let home = Path::new("/profiles/source/codex");
        assert!(validate_snapshot(b"{}", home).is_err());
        assert!(validate_snapshot(b"{\"file_id\":\"file-secret\"}\n", home).is_err());
        assert!(validate_snapshot(b"{\"file_ids\":[\"file-secret\"]}\n", home).is_err());
        assert!(validate_snapshot(b"{\"local_audio\":[\"/tmp/private.wav\"]}\n", home).is_err());
        assert!(validate_snapshot(b"{\"audio\":[\"https://example.com/a.wav\"]}\n", home).is_err());
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
    fn unknown_history_modes_are_not_advertised_as_transferable() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().canonicalize().unwrap();
        let path = home.join("rollout-test.jsonl");
        for mode in ["unknown"] {
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
            workspace: None,
            fingerprint: None,
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
        let account = client
            .call("account/read", json!({"refreshToken":false}))
            .unwrap();
        assert!(account["account"].is_null());
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
    #[test]
    fn preview_fingerprint_changes_when_local_attachment_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("codex");
        storage::private_dir(&home).unwrap();
        let image = home.join("attachment.png");
        let make = |text: &[u8]| {
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            bytes.extend_from_slice(text);
            fs::write(&image, bytes).unwrap();
        };
        let mut records: Vec<Value> = fixture(
            &uuid::Uuid::new_v4().to_string(),
            &root,
            "paginated",
            "image",
        )
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
        records[2]["payload"]["local_images"] = json!([image]);
        let raw = records
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>()
            .into_bytes();
        make(b"one");
        let first = prepare(raw.clone(), &home, None).unwrap();
        make(b"two");
        let second = prepare(raw.clone(), &home, None).unwrap();
        let event: Value = std::str::from_utf8(&first.0)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|v| v["type"] == "event_msg" && v["payload"]["type"] == "user_message")
            .unwrap();
        assert!(event["payload"]["images"][0]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert_eq!(event["payload"]["local_images"], json!([]));
        assert_eq!(first.1, 1);
        assert_ne!(first.0, second.0);
        assert!(!String::from_utf8(first.0)
            .unwrap()
            .contains(home.to_str().unwrap()));
        fs::remove_file(image).unwrap();
        assert!(prepare(raw, &home, None).is_err());
    }

    #[test]
    fn project_relocation_is_explicit_and_missing_workspace_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("codex");
        storage::private_dir(&home).unwrap();
        let raw = fixture(
            &uuid::Uuid::new_v4().to_string(),
            &root.join("missing"),
            "legacy",
            "text",
        )
        .into_bytes();
        assert!(prepare(raw.clone(), &home, None).is_err());
        let relocated = prepare(raw, &home, root.to_str()).unwrap();
        assert_eq!(
            snapshot::header(&relocated.0).unwrap()["payload"]["cwd"],
            json!(root)
        );
        assert!(prepare(
            fixture(&uuid::Uuid::new_v4().to_string(), &home, "legacy", "text").into_bytes(),
            &home,
            None
        )
        .is_err());
    }

    fn fixture(id: &str, cwd: &Path, mode: &str, text: &str) -> String {
        [
            json!({"type":"session_meta","payload":{"id":id,"session_id":id,"history_mode":mode,"timestamp":"2026-09-28T00:00:00Z","cwd":cwd,"originator":"codex_cli_rs","cli_version":"0.158.0","source":"cli","model_provider":"openai"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}),
            json!({"type":"event_msg","payload":{"type":"user_message","message":text,"images":[],"local_images":[],"text_elements":[]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Remembered."}]}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"Remembered."}}),
        ].into_iter().map(|mut v| { v["timestamp"] = json!("2026-09-28T00:00:00Z"); format!("{v}\n") }).collect()
    }

    #[test]
    #[ignore = "official client, synthetic compressed paginated branch and crash recovery; no real credentials"]
    fn official_client_pagination_batch_and_crash_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        registry.installation = std::env::var("PATHMUX_TEST_CHATGPT_APP").ok();
        let app = discovery::discover(registry.installation.as_deref()).unwrap();
        let cli = app.cli.as_deref().unwrap();
        let a = storage::create(&root, &mut registry, "Paged Source").unwrap();
        let b = storage::create(&root, &mut registry, "Paged Target").unwrap();
        let adir = storage::profile_dir(&root, &a.id).unwrap();
        let bdir = storage::profile_dir(&root, &b.id).unwrap();
        let sessions = adir.join("codex/sessions");
        storage::private_dir(&sessions).unwrap();
        let parent_id = uuid::Uuid::new_v4().to_string();
        let child_id = uuid::Uuid::new_v4().to_string();
        let prefix = fixture(&parent_id, &root, "paginated", "Inherited color is cyan.");
        let full = format!(
            "{prefix}{}\n",
            json!({"timestamp":"2026-09-28T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"DO_NOT_INHERIT_LATER_PARENT"}]}})
        );
        let parent = sessions.join(format!("rollout-{parent_id}.jsonl.zst"));
        fs::write(
            &parent,
            zstd::stream::encode_all(full.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let child = sessions.join(format!("rollout-{child_id}.jsonl"));
        let mut child_lines: Vec<Value> =
            fixture(&child_id, &root, "paginated", "Child value is amber.")
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
        child_lines[0]["payload"]["history_base"] =
            json!({"thread_id":parent_id,"end_ordinal_exclusive":5,"end_byte_offset":prefix.len()});
        let child_text = child_lines
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>();
        fs::write(&child, &child_text).unwrap();
        let item = inspect(&adir.join("codex"), &child).unwrap();
        let mut request = TransferRequest {
            source_id: a.id.clone(),
            target_id: b.id.clone(),
            key: item.key,
            revision: item.revision,
            workspace: None,
            fingerprint: None,
        };
        request.fingerprint = Some(preview(&root, &registry, &request).unwrap().fingerprint);
        let job = super::super::batch::create(&root, &registry, vec![request.clone()]).unwrap();
        let interrupted_key = transfer_key(&request, request.fingerprint.as_deref().unwrap());
        let transfer_dir = bdir.join("transfers");
        storage::private_dir(&transfer_dir).unwrap();
        let invalid_journal = transfer_dir.join(format!("{interrupted_key}.pending.json"));
        fs::write(&invalid_journal, b"incomplete-journal").unwrap();
        let paused = super::super::batch::step(&root, &registry, &b.id, &job.id, false).unwrap();
        assert!(paused.outcomes.is_empty());
        assert!(paused.last_error.is_some());
        fs::remove_file(invalid_journal).unwrap();
        let done = super::super::batch::step(&root, &registry, &b.id, &job.id, false).unwrap();
        assert!(done.last_error.is_none());
        assert!(
            done.outcomes[0].error.is_none(),
            "{:?}",
            done.outcomes[0].error
        );
        let imported = &done.outcomes[0].result.as_ref().unwrap().target_thread_id;
        // Simulate termination after an item committed but before its batch checkpoint.
        storage::write_json(&bdir.join("batches").join(format!("{}.json", job.id)), &job).unwrap();
        let retry = super::super::batch::step(&root, &registry, &b.id, &job.id, false).unwrap();
        assert!(retry.outcomes[0].result.as_ref().unwrap().duplicate);
        assert_eq!(
            &retry.outcomes[0].result.as_ref().unwrap().target_thread_id,
            imported
        );
        let mut client = rpc::Client::start(cli, &bdir).unwrap();
        let read = client
            .call(
                "thread/read",
                json!({"threadId":imported,"includeTurns":true}),
            )
            .unwrap();
        assert!(read.to_string().contains("cyan"));
        assert!(read.to_string().contains("amber"));
        assert!(!read.to_string().contains("DO_NOT_INHERIT_LATER_PARENT"));
        drop(client);
        let target_sessions = bdir.join("codex/sessions");
        storage::private_dir(&target_sessions).unwrap();
        fs::write(
            target_sessions.join(format!("rollout-{}.jsonl", uuid::Uuid::new_v4())),
            b"damaged-unrelated-history\n",
        )
        .unwrap();
        // Simulate a lost fork response. The journal predates the RPC and has no target ID.
        let staged_id = uuid::Uuid::new_v4().to_string();
        let key = "a".repeat(64);
        let transfers = bdir.join("transfers");
        let stage = transfers.join(format!("{key}.jsonl"));
        let staged = fixture(&staged_id, &root, "legacy", "Recoverable value is violet.");
        crate::sync::write_bytes_atomic(&stage, staged.as_bytes()).unwrap();
        let journal = TransferJournal {
            source_id: a.id.clone(),
            source_thread_id: child_id,
            snapshot_hash: key.clone(),
            target_thread_id: None,
            state: "pending".into(),
            stage_id: staged_id.clone(),
            title: "Recovery".into(),
            content_hash: hex::encode(Sha256::digest(staged.as_bytes())),
        };
        storage::write_json(&transfers.join(format!("{key}.pending.json")), &journal).unwrap();
        let mut client = rpc::Client::start(cli, &bdir).unwrap();
        let created = client.call("thread/fork",json!({"threadId":staged_id,"path":stage,"excludeTurns":true,"deferGoalContinuation":true,"sandbox":"read-only","approvalPolicy":"on-request"})).unwrap();
        let created_id = created["thread"]["id"].as_str().unwrap().to_owned();
        drop(client);
        let count_before = paths(&bdir.join("codex")).unwrap().0.len();
        fs::remove_dir_all(&adir).unwrap();
        assert!(
            transfer(&root, &registry, request.clone())
                .unwrap()
                .duplicate
        );
        let recovered = recover(&root, &registry, &b.id, &key, false).unwrap();
        assert_eq!(recovered.target_thread_id, created_id);
        assert_eq!(paths(&bdir.join("codex")).unwrap().0.len(), count_before);
        assert!(pending(&root, &registry, &b.id).unwrap().is_empty());
        assert!(!stage.exists());
        let mut client = rpc::Client::start(cli, &bdir).unwrap();
        let read = client
            .call(
                "thread/read",
                json!({"threadId":recovered.target_thread_id,"includeTurns":true}),
            )
            .unwrap();
        assert!(read.to_string().contains("violet"));
        assert!(!bdir.join("codex/auth.json").exists());
        // Exercise real continuation with a local response stub: verify what the
        // official client sends to its model provider, without any cloud inference.
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::{Duration, Instant};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    Err(_) => return,
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            let mut chunk = [0u8; 8192];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > 4 * 1024 * 1024 {
                    break;
                }
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..split]);
                    let length = head.lines().find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    });
                    if length.is_some_and(|len| bytes.len() >= split + 4 + len) {
                        break;
                    }
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&bytes).to_string());
            let response = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_pathmux_test\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n";
            let _ = write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response);
        });
        client.call("thread/resume",json!({"threadId":imported,"model":"gpt-5.4","modelProvider":"pathmux_test","excludeTurns":true,
            "config":{"model_providers.pathmux_test":{"name":"PathMux local test","base_url":format!("http://{address}/v1"),"wire_api":"responses","requires_openai_auth":false,"supports_websockets":false,"request_max_retries":0}}})).unwrap();
        client.call("turn/start",json!({"threadId":imported,"input":[{"type":"text","text":"Recall the inherited color and child value.","text_elements":[]}]})).unwrap();
        let sent = rx.recv_timeout(Duration::from_secs(20));
        server.join().unwrap();
        let sent = sent.expect("official continuation must reach local test provider");
        assert!(
            sent.contains("cyan")
                && sent.contains("amber")
                && !sent.contains("DO_NOT_INHERIT_LATER_PARENT"),
            "continuation must include imported model context"
        );
        assert!(sent.contains("Recall the inherited color and child value"));
    }

    #[test]
    fn local_images_are_embedded_and_missing_images_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let image = root.join("image.png");
        fs::write(&image, b"\x89PNG\r\n\x1a\nsynthetic").unwrap();
        let mut value = json!({"local_images":[image],"images":[]});
        let mut count = 0;
        embed_images(&mut value, &mut count).unwrap();
        assert_eq!(count, 1);
        assert_eq!(value["local_images"], json!([]));
        assert!(value["images"][0]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        fs::remove_file(&image).unwrap();
        assert!(embed_images(&mut json!({"local_images":[image]}), &mut 0).is_err());
    }
}
