use super::storage::{canonicalize_dir, revision, McpPaths};
use super::validation;
use super::{
    McpLocator, McpScope, McpService, McpState, McpSyncApplyRequest, McpSyncPreview, McpSyncStatus,
    McpSyncTargetInfo, McpTargetDisableRequest, McpTransport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

const REGISTRY_VERSION: u32 = 1;
pub(crate) const TARGET_ID: &str = "codex";
const TARGET_LABEL: &str = "Codex";
const MANAGED_FIELDS: &[&str] = &[
    "command",
    "args",
    "env",
    "cwd",
    "url",
    "http_headers",
    "enabled",
];

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncRegistry {
    version: u32,
    entries: Vec<SyncEntry>,
}

impl Default for SyncRegistry {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncEntry {
    source: McpLocator,
    target_path: String,
    #[serde(default = "default_selected")]
    selected: bool,
    last_source_hash: String,
    last_target_hash: String,
}

fn default_selected() -> bool {
    true
}

struct ConvertedConfig {
    normalized: Map<String, JsonValue>,
    warnings: Vec<String>,
}

fn write_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "OpenAI MCP 同步锁已损坏，请重启管理中心".to_string())
}

fn source_service<'a>(state: &'a McpState, locator: &McpLocator) -> Result<&'a McpService, String> {
    state
        .services
        .iter()
        .find(|service| &service.locator == locator)
        .ok_or_else(|| "来源 MCP 已不存在，请刷新后重试".to_string())
}

fn target_path(paths: &McpPaths, locator: &McpLocator) -> Result<PathBuf, String> {
    match locator.scope {
        McpScope::User => Ok(paths.global_codex_config()),
        McpScope::Local | McpScope::Project => {
            let project = locator
                .project_path
                .as_deref()
                .ok_or_else(|| "项目级同步缺少项目路径".to_string())?;
            Ok(canonicalize_dir(project)?
                .join(".codex")
                .join("config.toml"))
        }
    }
}

fn convert(service: &McpService) -> Result<ConvertedConfig, String> {
    let config = &service.config;
    let mut normalized = Map::new();
    let mut warnings = Vec::new();
    normalized.insert("enabled".into(), JsonValue::Bool(service.enabled));

    match service.transport {
        McpTransport::Stdio => {
            let command = config
                .get("command")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "STDIO MCP 缺少有效 command".to_string())?;
            normalized.insert("command".into(), JsonValue::String(command.to_string()));

            if let Some(args) = config.get("args") {
                let args = args
                    .as_array()
                    .ok_or_else(|| "args 必须是字符串数组".to_string())?;
                if args.iter().any(|value| !value.is_string()) {
                    return Err("args 必须全部为字符串".into());
                }
                normalized.insert("args".into(), JsonValue::Array(args.clone()));
            }

            if let Some(env) = config.get("env") {
                let env = env
                    .as_object()
                    .ok_or_else(|| "env 必须是对象".to_string())?;
                if env.values().any(|value| !value.is_string()) {
                    return Err("env 的值必须全部为字符串".into());
                }
                normalized.insert("env".into(), JsonValue::Object(env.clone()));
            }

            if let Some(cwd) = config.get("cwd") {
                let cwd = cwd.as_str().ok_or_else(|| "cwd 必须是字符串".to_string())?;
                normalized.insert("cwd".into(), JsonValue::String(cwd.to_string()));
            }
        }
        McpTransport::Http => {
            let url = config
                .get("url")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "HTTP MCP 缺少有效 url".to_string())?;
            normalized.insert("url".into(), JsonValue::String(url.to_string()));

            if let Some(headers) = config.get("headers") {
                let headers = headers
                    .as_object()
                    .ok_or_else(|| "headers 必须是对象".to_string())?;
                if headers.values().any(|value| !value.is_string()) {
                    return Err("headers 的值必须全部为字符串".into());
                }
                normalized.insert("http_headers".into(), JsonValue::Object(headers.clone()));
                if headers.keys().any(|name| {
                    matches!(
                        name.to_ascii_lowercase().as_str(),
                        "authorization" | "x-api-key" | "api-key"
                    )
                }) {
                    warnings.push(
                        "HTTP 认证头将写入本机 Codex 配置；预览已脱敏，请确认该电脑可信"
                            .to_string(),
                    );
                }
            }
        }
        McpTransport::Sse => {
            return Err("Codex 当前使用 Streamable HTTP，不支持直接同步 SSE 配置".into())
        }
        McpTransport::Ws => return Err("Codex 当前不支持直接同步 WebSocket MCP".into()),
        McpTransport::Unknown => {
            return Err("无法识别 MCP 传输类型，请先补全 command 或 HTTP type/url".into())
        }
    }

    Ok(ConvertedConfig {
        normalized,
        warnings,
    })
}

fn stable_hash(value: &Map<String, JsonValue>) -> String {
    let ordered: BTreeMap<&String, &JsonValue> = value.iter().collect();
    let bytes = serde_json::to_vec(&ordered).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn read_registry(paths: &McpPaths) -> Result<SyncRegistry, String> {
    let path = paths.openai_sync_registry();
    if !path.exists() {
        return Ok(SyncRegistry::default());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("读取 OpenAI 同步记录失败：{e}"))?;
    let registry: SyncRegistry =
        serde_json::from_str(&text).map_err(|e| format!("OpenAI 同步记录损坏：{e}"))?;
    if registry.version != REGISTRY_VERSION {
        return Err(format!(
            "不支持的 OpenAI 同步记录版本：{}",
            registry.version
        ));
    }
    Ok(registry)
}

fn read_target(path: &Path) -> Result<DocumentMut, String> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let text =
        fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    text.parse::<DocumentMut>()
        .map_err(|e| format!("{} TOML 解析失败：{e}", path.display()))
}

fn server_table<'a>(doc: &'a DocumentMut, name: &str) -> Option<&'a Table> {
    doc.get("mcp_servers")
        .and_then(Item::as_table)
        .and_then(|servers| servers.get(name))
        .and_then(Item::as_table)
}

fn strings_from_item(item: &Item) -> Option<Vec<JsonValue>> {
    let array = item.as_array()?;
    let mut values = Vec::with_capacity(array.len());
    for value in array.iter() {
        values.push(JsonValue::String(value.as_str()?.to_string()));
    }
    Some(values)
}

fn string_map_from_item(item: &Item) -> Option<Map<String, JsonValue>> {
    if let Some(table) = item.as_table() {
        let mut result = Map::new();
        for (key, value) in table.iter() {
            result.insert(
                key.to_string(),
                JsonValue::String(value.as_str()?.to_string()),
            );
        }
        return Some(result);
    }
    let inline = item.as_inline_table()?;
    let mut result = Map::new();
    for (key, value) in inline.iter() {
        result.insert(
            key.to_string(),
            JsonValue::String(value.as_str()?.to_string()),
        );
    }
    Some(result)
}

fn normalized_target(
    doc: &DocumentMut,
    name: &str,
) -> Result<Option<Map<String, JsonValue>>, String> {
    let Some(table) = server_table(doc, name) else {
        return Ok(None);
    };
    let mut normalized = Map::new();
    if let Some(value) = table.get("enabled") {
        normalized.insert(
            "enabled".into(),
            JsonValue::Bool(
                value
                    .as_bool()
                    .ok_or_else(|| format!("mcp_servers.{name}.enabled 必须是布尔值"))?,
            ),
        );
    } else {
        normalized.insert("enabled".into(), JsonValue::Bool(true));
    }
    for key in ["command", "cwd", "url"] {
        if let Some(value) = table.get(key) {
            normalized.insert(
                key.into(),
                JsonValue::String(
                    value
                        .as_str()
                        .ok_or_else(|| format!("mcp_servers.{name}.{key} 必须是字符串"))?
                        .to_string(),
                ),
            );
        }
    }
    if let Some(args) = table.get("args") {
        normalized.insert(
            "args".into(),
            JsonValue::Array(
                strings_from_item(args)
                    .ok_or_else(|| format!("mcp_servers.{name}.args 必须是字符串数组"))?,
            ),
        );
    }
    for key in ["env", "http_headers"] {
        if let Some(value) = table.get(key) {
            normalized.insert(
                key.into(),
                JsonValue::Object(
                    string_map_from_item(value)
                        .ok_or_else(|| format!("mcp_servers.{name}.{key} 必须是字符串映射"))?,
                ),
            );
        }
    }
    Ok(Some(normalized))
}

fn ensure_server_table<'a>(doc: &'a mut DocumentMut, name: &str) -> Result<&'a mut Table, String> {
    if !doc.contains_key("mcp_servers") {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = doc["mcp_servers"]
        .as_table_mut()
        .ok_or_else(|| "mcp_servers 必须是 TOML 表".to_string())?;
    if !servers.contains_key(name) {
        servers[name] = Item::Table(Table::new());
    }
    servers[name]
        .as_table_mut()
        .ok_or_else(|| format!("mcp_servers.{name} 必须是 TOML 表"))
}

fn set_existing_server_enabled(
    doc: &mut DocumentMut,
    name: &str,
    enabled: bool,
) -> Result<(), String> {
    let table = doc
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .and_then(|servers| servers.get_mut(name))
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("目标端不存在 MCP「{name}」，请刷新后重试"))?;
    table["enabled"] = toml_edit::value(enabled);
    Ok(())
}

fn string_array(values: &[JsonValue]) -> Result<TomlValue, String> {
    let mut array = Array::new();
    for value in values {
        array.push(
            value
                .as_str()
                .ok_or_else(|| "数组成员必须是字符串".to_string())?,
        );
    }
    Ok(TomlValue::Array(array))
}

fn inline_string_map(values: &Map<String, JsonValue>) -> Result<TomlValue, String> {
    let mut table = InlineTable::new();
    for (key, value) in values {
        table.insert(
            key,
            TomlValue::from(
                value
                    .as_str()
                    .ok_or_else(|| "映射值必须是字符串".to_string())?,
            ),
        );
    }
    Ok(TomlValue::InlineTable(table))
}

fn apply_normalized(
    doc: &mut DocumentMut,
    name: &str,
    normalized: &Map<String, JsonValue>,
) -> Result<(), String> {
    let table = ensure_server_table(doc, name)?;
    for key in MANAGED_FIELDS {
        table.remove(key);
    }

    for key in ["command", "cwd", "url"] {
        if let Some(value) = normalized.get(key) {
            table[key] = toml_edit::value(
                value
                    .as_str()
                    .ok_or_else(|| format!("{key} 必须是字符串"))?,
            );
        }
    }
    if let Some(value) = normalized.get("args") {
        table["args"] = Item::Value(string_array(
            value
                .as_array()
                .ok_or_else(|| "args 必须是数组".to_string())?,
        )?);
    }
    if let Some(value) = normalized.get("env") {
        let values = value
            .as_object()
            .ok_or_else(|| "env 必须是对象".to_string())?;
        let mut env = Table::new();
        for (key, value) in values {
            env[key] = toml_edit::value(
                value
                    .as_str()
                    .ok_or_else(|| "env 值必须是字符串".to_string())?,
            );
        }
        table["env"] = Item::Table(env);
    }
    if let Some(value) = normalized.get("http_headers") {
        table["http_headers"] = Item::Value(inline_string_map(
            value
                .as_object()
                .ok_or_else(|| "http_headers 必须是对象".to_string())?,
        )?);
    }
    table["enabled"] = toml_edit::value(
        normalized
            .get("enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true),
    );
    Ok(())
}

fn registry_entry<'a>(registry: &'a SyncRegistry, locator: &McpLocator) -> Option<&'a SyncEntry> {
    registry
        .entries
        .iter()
        .find(|entry| &entry.source == locator)
}

fn registry_entry_mut<'a>(
    registry: &'a mut SyncRegistry,
    locator: &McpLocator,
) -> Option<&'a mut SyncEntry> {
    registry
        .entries
        .iter_mut()
        .find(|entry| &entry.source == locator)
}

fn redacted(value: &Map<String, JsonValue>) -> JsonValue {
    let value = JsonValue::Object(value.clone());
    let paths = validation::sensitive_paths(&value);
    validation::redact(&value, &paths)
}

fn sync_status(
    service: &McpService,
    target: Option<&Map<String, JsonValue>>,
    entry: Option<&SyncEntry>,
) -> McpSyncStatus {
    let source = match convert(service) {
        Ok(value) => value.normalized,
        Err(_) => return McpSyncStatus::Incompatible,
    };
    let Some(entry) = entry else {
        return McpSyncStatus::NotSynced;
    };
    let source_changed = stable_hash(&source) != entry.last_source_hash;
    let target_changed = target
        .map(stable_hash)
        .unwrap_or_else(|| "missing".to_string())
        != entry.last_target_hash;
    match (source_changed, target_changed) {
        (false, false) => McpSyncStatus::Synced,
        (true, false) => McpSyncStatus::SourceUpdated,
        (false, true) => McpSyncStatus::TargetModified,
        (true, true) => McpSyncStatus::Conflict,
    }
}

fn status_detail(status: &McpSyncStatus, target_exists: bool) -> String {
    match status {
        McpSyncStatus::NotSynced if target_exists => {
            "ChatGPT/Codex 已有同名配置，首次同步前需要确认差异".into()
        }
        McpSyncStatus::NotSynced => "尚未同步到 ChatGPT/Codex".into(),
        McpSyncStatus::Synced => "配置一致；来源更新后将自动同步".into(),
        McpSyncStatus::SourceUpdated => "检测到 Claude Code 配置更新，等待确认同步".into(),
        McpSyncStatus::TargetModified => "ChatGPT/Codex 配置已被单独修改，自动同步已暂停".into(),
        McpSyncStatus::Conflict => "两端都已修改，需要人工选择后再同步".into(),
        McpSyncStatus::Incompatible => "当前配置不能转换为 Codex MCP 配置".into(),
    }
}

pub(crate) fn attach_sync_state(paths: &McpPaths, mut state: McpState) -> McpState {
    let registry = read_registry(paths);
    let registry_error = registry.as_ref().err().cloned();
    let registry = registry.unwrap_or_default();
    let mut infos = Vec::with_capacity(state.services.len());

    for service in &state.services {
        let path = match target_path(paths, &service.locator) {
            Ok(path) => path,
            Err(error) => {
                infos.push(McpSyncTargetInfo {
                    locator: service.locator.clone(),
                    target_id: TARGET_ID.into(),
                    target_label: TARGET_LABEL.into(),
                    status: McpSyncStatus::Incompatible,
                    connected: false,
                    target_path: String::new(),
                    target_revision: "missing".into(),
                    detail: error,
                });
                continue;
            }
        };
        let entry = registry_entry(&registry, &service.locator);
        let target =
            read_target(&path).and_then(|doc| normalized_target(&doc, &service.locator.name));
        let connected = entry.map(|entry| entry.selected).unwrap_or(false);
        if let Err(error) = convert(service) {
            infos.push(McpSyncTargetInfo {
                locator: service.locator.clone(),
                target_id: TARGET_ID.into(),
                target_label: TARGET_LABEL.into(),
                status: McpSyncStatus::Incompatible,
                connected,
                target_path: path.display().to_string(),
                target_revision: revision(&path),
                detail: error,
            });
            continue;
        }
        let (status, mut detail) = match target {
            Ok(target) => {
                let status = sync_status(service, target.as_ref(), entry);
                let detail = status_detail(&status, target.is_some());
                (status, detail)
            }
            Err(error) => (McpSyncStatus::Incompatible, error),
        };
        if let Some(error) = &registry_error {
            detail = error.clone();
        }
        let status = if registry_error.is_some() {
            McpSyncStatus::Incompatible
        } else {
            status
        };
        infos.push(McpSyncTargetInfo {
            locator: service.locator.clone(),
            target_id: TARGET_ID.into(),
            target_label: TARGET_LABEL.into(),
            status,
            connected,
            target_path: path.display().to_string(),
            target_revision: revision(&path),
            detail,
        });
    }
    state.sync_targets.extend(infos);
    state
        .sync_target_revisions
        .insert(TARGET_ID.into(), revision(&paths.openai_sync_registry()));
    state
}

pub(crate) fn preview_sync(
    paths: &McpPaths,
    state: &McpState,
    locator: &McpLocator,
) -> Result<McpSyncPreview, String> {
    let service = source_service(state, locator)?;
    if !service.enabled {
        return Err("请先开启 MCP 状态，再开启目标端使用开关".into());
    }
    let converted = convert(service)?;
    let path = target_path(paths, locator)?;
    let doc = read_target(&path)?;
    let before = normalized_target(&doc, &locator.name)?;
    let registry = read_registry(paths)?;
    let entry = registry_entry(&registry, locator);
    let status = sync_status(service, before.as_ref(), entry);
    let mut warnings = converted.warnings;
    match status {
        McpSyncStatus::TargetModified => warnings.push(
            "ChatGPT/Codex 中的受管配置已修改；继续会用 Claude Code 配置覆盖这些字段"
                .into(),
        ),
        McpSyncStatus::Conflict => warnings.push(
            "Claude Code 与 ChatGPT/Codex 都已修改；继续会以 Claude Code 为准".into(),
        ),
        McpSyncStatus::NotSynced if before.is_some() => warnings.push(
            "目标存在同名 MCP；继续会覆盖 command/url/args/env/headers/enabled，保留目标端工具审批等专属设置"
                .into(),
        ),
        _ => {}
    }
    if locator.scope == McpScope::Local {
        warnings.push(
            "Claude Local 作用域没有对应的实例概念，将同步到该项目的 .codex/config.toml".into(),
        );
    }

    Ok(McpSyncPreview {
        locator: locator.clone(),
        target_id: TARGET_ID.into(),
        target_label: TARGET_LABEL.into(),
        action_label: if entry.is_some() {
            format!("更新同步「{}」到 ChatGPT/Codex", locator.name)
        } else {
            format!("同步「{}」到 ChatGPT/Codex", locator.name)
        },
        target_path: path.display().to_string(),
        redacted_before: before.as_ref().map(redacted),
        redacted_after: redacted(&converted.normalized),
        warnings,
        expected_source_revision: service.revision.clone(),
        expected_target_revision: revision(&path),
        expected_registry_revision: revision(&paths.openai_sync_registry()),
        restart_hint: "同步完成后需要重启 ChatGPT/Codex。".into(),
        preserved_fields_note:
            "只更新目标 MCP 的连接配置和启用状态；目标端已有的工具白名单与审批设置会保留。".into(),
    })
}

fn unique_suffix() -> String {
    let n = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    format!("{}-{millis}-{n}", std::process::id())
}

fn write_text_atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("目标没有父目录：{}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    let path_hash = target_hash(path);
    let tmp_prefix = format!(".ccm-openai-tmp-{path_hash}-");
    let tmp = parent.join(format!("{tmp_prefix}{}", unique_suffix()));
    let rollback = parent.join(format!(".ccm-openai-rollback-{path_hash}"));
    if !path.exists() && rollback.exists() {
        fs::rename(&rollback, path).map_err(|e| format!("恢复上次中断留下的配置失败：{e}"))?;
        return Err("检测到上次写入中断，已恢复原配置，请刷新后重试".into());
    }
    if path.exists() && rollback.exists() {
        fs::remove_file(&rollback).map_err(|e| format!("清理旧回滚文件失败：{e}"))?;
    }
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&tmp_prefix) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    {
        let mut file = fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败：{e}"))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
        file.sync_all()
            .map_err(|e| format!("同步临时文件失败：{e}"))?;
    }
    if path.exists() {
        fs::rename(path, &rollback).map_err(|e| format!("暂存原配置失败：{e}"))?;
    }
    match fs::rename(&tmp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&rollback);
            Ok(())
        }
        Err(error) => {
            if rollback.exists() {
                let _ = fs::rename(&rollback, path);
            }
            let _ = fs::remove_file(&tmp);
            Err(format!("替换配置失败：{error}"))
        }
    }
}

fn backup_target(paths: &McpPaths, target: &Path) -> Result<(), String> {
    if !target.exists() {
        return Ok(());
    }
    let dir = paths.backup_dir().join("openai");
    fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败：{e}"))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let hash = target_hash(target);
    let backup = dir.join(format!("{}-{hash}-{file_name}", unique_suffix()));
    fs::copy(target, backup).map_err(|e| format!("备份 OpenAI MCP 配置失败：{e}"))?;
    let mut backups: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| format!("读取备份目录失败：{e}"))?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(&hash))
        .collect();
    backups.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    for old in backups.into_iter().skip(5) {
        let _ = fs::remove_file(old.path());
    }
    Ok(())
}

fn target_hash(path: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.display().to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn registry_text(registry: &SyncRegistry) -> Result<String, String> {
    serde_json::to_string_pretty(registry)
        .map(|text| format!("{text}\n"))
        .map_err(|e| format!("序列化同步记录失败：{e}"))
}

fn upsert_entry(
    registry: &mut SyncRegistry,
    locator: &McpLocator,
    path: &Path,
    source_hash: String,
    target_hash: String,
) {
    let target_path = path.display().to_string();
    registry.entries.retain(|entry| {
        entry.source == *locator
            || entry.target_path != target_path
            || entry.source.name != locator.name
    });
    if let Some(entry) = registry_entry_mut(registry, locator) {
        entry.target_path = target_path;
        entry.selected = true;
        entry.last_source_hash = source_hash;
        entry.last_target_hash = target_hash;
        return;
    }
    registry.entries.push(SyncEntry {
        source: locator.clone(),
        target_path,
        selected: true,
        last_source_hash: source_hash,
        last_target_hash: target_hash,
    });
}

fn write_target_and_registry(
    paths: &McpPaths,
    target_path: &Path,
    target_doc: &DocumentMut,
    registry: &SyncRegistry,
) -> Result<(), String> {
    let registry_path = paths.openai_sync_registry();
    let old_target = fs::read(target_path).ok();
    let old_registry = fs::read(&registry_path).ok();
    backup_target(paths, target_path)?;
    write_text_atomic(target_path, &target_doc.to_string())?;
    if let Err(error) = write_text_atomic(&registry_path, &registry_text(registry)?) {
        match old_target {
            Some(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let _ = write_text_atomic(target_path, &text);
            }
            None => {
                let _ = fs::remove_file(target_path);
            }
        }
        match old_registry {
            Some(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let _ = write_text_atomic(&registry_path, &text);
            }
            None => {
                let _ = fs::remove_file(&registry_path);
            }
        }
        return Err(error);
    }
    Ok(())
}

pub(crate) fn apply_manual_sync(
    paths: &McpPaths,
    state: &McpState,
    request: &McpSyncApplyRequest,
) -> Result<(), String> {
    let _guard = write_guard()?;
    let service = source_service(state, &request.locator)?;
    if !service.enabled {
        return Err("请先开启 MCP 状态，再开启目标端使用开关".into());
    }
    if service.revision != request.expected_source_revision {
        return Err("Claude Code MCP 已被修改，请刷新预览后重试".into());
    }
    let path = target_path(paths, &request.locator)?;
    if revision(&path) != request.expected_target_revision {
        return Err("ChatGPT/Codex MCP 已被修改，请刷新预览后重试".into());
    }
    if revision(&paths.openai_sync_registry()) != request.expected_registry_revision {
        return Err("同步设置已被其他操作修改，请刷新后重试".into());
    }

    let converted = convert(service)?;
    let source_hash = stable_hash(&converted.normalized);
    let mut doc = read_target(&path)?;
    apply_normalized(&mut doc, &request.locator.name, &converted.normalized)?;
    let target = normalized_target(&doc, &request.locator.name)?
        .ok_or_else(|| "生成目标配置失败".to_string())?;
    let target_hash = stable_hash(&target);
    let mut registry = read_registry(paths)?;
    upsert_entry(
        &mut registry,
        &request.locator,
        &path,
        source_hash,
        target_hash,
    );
    write_target_and_registry(paths, &path, &doc, &registry)
}

pub(crate) fn disable_target(
    paths: &McpPaths,
    state: &McpState,
    request: &McpTargetDisableRequest,
) -> Result<(), String> {
    let _guard = write_guard()?;
    let service = source_service(state, &request.locator)?;
    if !service.enabled {
        return Err("MCP 总开关已关闭，无需重复关闭目标端".into());
    }
    let path = target_path(paths, &request.locator)?;
    if revision(&path) != request.expected_target_revision {
        return Err("目标端 MCP 配置已发生变化，请刷新后重试".into());
    }
    if revision(&paths.openai_sync_registry()) != request.expected_registry_revision {
        return Err("同步设置已被其他操作修改，请刷新后重试".into());
    }

    let mut doc = read_target(&path)?;
    let before = normalized_target(&doc, &request.locator.name)?
        .ok_or_else(|| "目标端 MCP 配置不存在，请刷新后重试".to_string())?;
    let mut registry = read_registry(paths)?;
    let entry = registry_entry_mut(&mut registry, &request.locator)
        .ok_or_else(|| "该 MCP 尚未接入目标端".to_string())?;
    let target_was_unchanged = stable_hash(&before) == entry.last_target_hash;
    set_existing_server_enabled(&mut doc, &request.locator.name, false)?;
    let after = normalized_target(&doc, &request.locator.name)?
        .ok_or_else(|| "更新目标端 MCP 状态失败".to_string())?;
    entry.selected = false;
    if target_was_unchanged {
        entry.last_target_hash = stable_hash(&after);
    }
    write_target_and_registry(paths, &path, &doc, &registry)
}

pub(crate) fn reconcile_source_enabled(
    paths: &McpPaths,
    state: &McpState,
    locator: &McpLocator,
) -> Result<(), String> {
    let _guard = write_guard()?;
    let service = source_service(state, locator)?;
    let mut registry = read_registry(paths)?;
    let Some(entry) = registry_entry_mut(&mut registry, locator) else {
        return Ok(());
    };
    if !entry.selected {
        return Ok(());
    }

    let path = target_path(paths, locator)?;
    let mut doc = read_target(&path)?;
    let before = normalized_target(&doc, &locator.name)?
        .ok_or_else(|| "目标端 MCP 配置不存在，请重新开启目标端使用开关".to_string())?;
    let current_enabled = before
        .get("enabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    if current_enabled == service.enabled {
        return Ok(());
    }

    let target_was_unchanged = stable_hash(&before) == entry.last_target_hash;
    set_existing_server_enabled(&mut doc, &locator.name, service.enabled)?;
    let after = normalized_target(&doc, &locator.name)?
        .ok_or_else(|| "更新目标端 MCP 状态失败".to_string())?;
    entry.last_source_hash = stable_hash(&convert(service)?.normalized);
    if target_was_unchanged {
        entry.last_target_hash = stable_hash(&after);
    }
    write_target_and_registry(paths, &path, &doc, &registry)
}

fn auto_sync_all_locked(paths: &McpPaths, state: &McpState) -> Result<bool, String> {
    let mut registry = read_registry(paths)?;
    let mut changed = false;

    for index in 0..registry.entries.len() {
        if !registry.entries[index].selected {
            continue;
        }
        let locator = registry.entries[index].source.clone();
        let Some(service) = state
            .services
            .iter()
            .find(|service| service.locator == locator)
        else {
            continue;
        };
        let converted = match convert(service) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let source_hash = stable_hash(&converted.normalized);
        let path = target_path(paths, &locator)?;
        let target_revision = revision(&path);
        let mut doc = read_target(&path)?;
        let Some(current_target) = normalized_target(&doc, &locator.name)? else {
            continue;
        };
        let current_target_hash = stable_hash(&current_target);
        let current_enabled = current_target
            .get("enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true);
        if current_enabled != service.enabled {
            let target_was_unchanged =
                current_target_hash == registry.entries[index].last_target_hash;
            set_existing_server_enabled(&mut doc, &locator.name, service.enabled)?;
            let next_target = normalized_target(&doc, &locator.name)?
                .ok_or_else(|| "自动联动目标端 MCP 状态失败".to_string())?;
            if revision(&path) != target_revision {
                continue;
            }
            registry.entries[index].last_source_hash = source_hash;
            if target_was_unchanged {
                registry.entries[index].last_target_hash = stable_hash(&next_target);
            }
            registry.entries[index].target_path = path.display().to_string();
            write_target_and_registry(paths, &path, &doc, &registry)?;
            changed = true;
            continue;
        }
        if source_hash == registry.entries[index].last_source_hash {
            continue;
        }
        if current_target_hash != registry.entries[index].last_target_hash {
            continue;
        }

        apply_normalized(&mut doc, &locator.name, &converted.normalized)?;
        let next_target = normalized_target(&doc, &locator.name)?
            .ok_or_else(|| "自动同步生成目标配置失败".to_string())?;
        if revision(&path) != target_revision {
            continue;
        }
        registry.entries[index].last_source_hash = source_hash;
        registry.entries[index].last_target_hash = stable_hash(&next_target);
        registry.entries[index].target_path = path.display().to_string();
        write_target_and_registry(paths, &path, &doc, &registry)?;
        changed = true;
    }
    Ok(changed)
}

pub(crate) fn auto_sync_all(paths: &McpPaths, state: &McpState) -> Result<bool, String> {
    let _guard = write_guard()?;
    auto_sync_all_locked(paths, state)
}

pub(crate) fn start_monitor(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        let paths = McpPaths::system();
        let instances = super::current_instances();
        let state = super::storage::collect_state(&paths, &instances);
        match auto_sync_all(&paths, &state) {
            Ok(true) => {
                let _ = app.emit("mcp-sync-target-updated", TARGET_ID);
            }
            Ok(false) => {}
            Err(error) => crate::sync::log_line(&format!("OpenAI MCP 自动同步失败：{error}")),
        }
        std::thread::sleep(Duration::from_secs(2));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpEffectiveState;
    use serde_json::json;

    fn service(root: &Path, config: JsonValue) -> McpService {
        McpService {
            locator: McpLocator {
                scope: McpScope::User,
                name: "demo".into(),
                instance_id: None,
                project_path: None,
            },
            transport: validation::infer_transport(config.as_object().unwrap()),
            raw_transport: None,
            config: config.as_object().unwrap().clone(),
            enabled: true,
            effective_state: McpEffectiveState::Effective,
            shadowed_by: vec![],
            shadowed_context_count: 0,
            source_id: "user:__main__".into(),
            revision: revision(&root.join(".claude.json")),
            sensitive_paths: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn stdio_conversion_preserves_command_args_env_and_enabled() {
        let root = std::env::temp_dir().join(format!("ccm-openai-convert-{}", unique_suffix()));
        let item = service(
            &root,
            json!({
                "command": "npx",
                "args": ["-y", "server"],
                "env": {"TOKEN": "secret"}
            }),
        );
        let converted = convert(&item).unwrap();
        assert_eq!(converted.normalized["command"], "npx");
        assert_eq!(converted.normalized["args"], json!(["-y", "server"]));
        assert_eq!(converted.normalized["env"]["TOKEN"], "secret");
        assert_eq!(converted.normalized["enabled"], true);
    }

    #[test]
    fn applying_config_preserves_codex_only_tool_policy() {
        let mut doc = r#"
[mcp_servers.demo]
command = "old"
enabled_tools = ["read"]
default_tools_approval_mode = "prompt"
"#
        .parse::<DocumentMut>()
        .unwrap();
        let normalized = json!({
            "command": "new",
            "args": ["server"],
            "enabled": true
        })
        .as_object()
        .unwrap()
        .clone();
        apply_normalized(&mut doc, "demo", &normalized).unwrap();
        let text = doc.to_string();
        assert!(text.contains("command = \"new\""));
        assert!(text.contains("enabled_tools = [\"read\"]"));
        assert!(text.contains("default_tools_approval_mode = \"prompt\""));
    }

    #[test]
    fn status_distinguishes_source_target_and_conflict_changes() {
        let root = std::env::temp_dir().join(format!("ccm-openai-status-{}", unique_suffix()));
        let original = service(&root, json!({"command": "node", "args": ["a"]}));
        let source = convert(&original).unwrap().normalized;
        let entry = SyncEntry {
            source: original.locator.clone(),
            target_path: "x".into(),
            selected: true,
            last_source_hash: stable_hash(&source),
            last_target_hash: stable_hash(&source),
        };
        assert_eq!(
            sync_status(&original, Some(&source), Some(&entry)),
            McpSyncStatus::Synced
        );

        let changed_source = service(&root, json!({"command": "node", "args": ["b"]}));
        assert_eq!(
            sync_status(&changed_source, Some(&source), Some(&entry)),
            McpSyncStatus::SourceUpdated
        );

        let target = json!({"command": "other", "enabled": true})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            sync_status(&original, Some(&target), Some(&entry)),
            McpSyncStatus::TargetModified
        );
        assert_eq!(
            sync_status(&changed_source, Some(&target), Some(&entry)),
            McpSyncStatus::Conflict
        );
    }

    #[test]
    fn manual_sync_writes_codex_config_and_registry() {
        let root = std::env::temp_dir().join(format!("ccm-openai-apply-{}", unique_suffix()));
        fs::create_dir_all(&root).unwrap();
        let paths = McpPaths::for_test(root.clone());
        fs::write(
            root.join(".claude.json"),
            json!({"mcpServers":{"demo":{"command":"node","args":["server.js"]}}}).to_string(),
        )
        .unwrap();
        let item = service(&root, json!({"command": "node", "args": ["server.js"]}));
        let state = McpState {
            services: vec![item.clone()],
            instances: vec![],
            projects: vec![],
            revisions: BTreeMap::new(),
            issues: vec![],
            summary: super::super::McpSummary {
                total: 1,
                enabled: 1,
                disabled: 0,
                warnings: 0,
                shadowed: 0,
            },
            operation_warnings: vec![],
            sync_targets: vec![],
            sync_target_revisions: BTreeMap::new(),
        };
        let preview = preview_sync(&paths, &state, &item.locator).unwrap();
        apply_manual_sync(
            &paths,
            &state,
            &McpSyncApplyRequest {
                locator: item.locator.clone(),
                target_id: TARGET_ID.into(),
                expected_source_revision: preview.expected_source_revision,
                expected_target_revision: preview.expected_target_revision,
                expected_registry_revision: preview.expected_registry_revision,
            },
        )
        .unwrap();
        let text = fs::read_to_string(paths.global_codex_config()).unwrap();
        assert!(text.contains("[mcp_servers.demo]"));
        assert!(text.contains("command = \"node\""));
        let registry = read_registry(&paths).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(registry.entries[0].selected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn disabling_target_keeps_target_source_and_unrelated_config() {
        let root = std::env::temp_dir().join(format!("ccm-openai-disable-{}", unique_suffix()));
        fs::create_dir_all(&root).unwrap();
        let paths = McpPaths::for_test(root.clone());
        fs::write(
            root.join(".claude.json"),
            json!({"mcpServers":{"demo":{"command":"node","args":["server.js"]}}}).to_string(),
        )
        .unwrap();
        fs::create_dir_all(paths.global_codex_config().parent().unwrap()).unwrap();
        fs::write(
            paths.global_codex_config(),
            "[mcp_servers.unrelated]\ncommand = \"keep\"\n",
        )
        .unwrap();

        let item = service(&root, json!({"command": "node", "args": ["server.js"]}));
        let state = McpState {
            services: vec![item.clone()],
            instances: vec![],
            projects: vec![],
            revisions: BTreeMap::new(),
            issues: vec![],
            summary: super::super::McpSummary {
                total: 1,
                enabled: 1,
                disabled: 0,
                warnings: 0,
                shadowed: 0,
            },
            operation_warnings: vec![],
            sync_targets: vec![],
            sync_target_revisions: BTreeMap::new(),
        };
        let preview = preview_sync(&paths, &state, &item.locator).unwrap();
        apply_manual_sync(
            &paths,
            &state,
            &McpSyncApplyRequest {
                locator: item.locator.clone(),
                target_id: TARGET_ID.into(),
                expected_source_revision: preview.expected_source_revision,
                expected_target_revision: preview.expected_target_revision,
                expected_registry_revision: preview.expected_registry_revision,
            },
        )
        .unwrap();
        let connected_state = attach_sync_state(&paths, state.clone());
        assert!(connected_state.sync_targets[0].connected);

        disable_target(
            &paths,
            &state,
            &McpTargetDisableRequest {
                locator: item.locator.clone(),
                target_id: TARGET_ID.into(),
                expected_target_revision: revision(&paths.global_codex_config()),
                expected_registry_revision: revision(&paths.openai_sync_registry()),
            },
        )
        .unwrap();

        let target = fs::read_to_string(paths.global_codex_config()).unwrap();
        assert!(target.contains("[mcp_servers.demo]"));
        assert!(target.contains("enabled = false"));
        assert!(target.contains("[mcp_servers.unrelated]"));
        assert!(root.join(".claude.json").exists());
        assert!(fs::read_to_string(root.join(".claude.json"))
            .unwrap()
            .contains("\"demo\""));
        let registry = read_registry(&paths).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(!registry.entries[0].selected);
        let mut changed_state = state.clone();
        changed_state.services[0] =
            service(&root, json!({"command": "node", "args": ["changed.js"]}));
        assert!(!auto_sync_all(&paths, &changed_state).unwrap());
        assert!(!fs::read_to_string(paths.global_codex_config())
            .unwrap()
            .contains("changed.js"));
        let disabled_state = attach_sync_state(&paths, state);
        assert!(!disabled_state.sync_targets[0].connected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn master_status_disables_and_restores_selected_target_without_removing_it() {
        let root = std::env::temp_dir().join(format!("ccm-openai-master-{}", unique_suffix()));
        fs::create_dir_all(&root).unwrap();
        let paths = McpPaths::for_test(root.clone());
        fs::write(
            root.join(".claude.json"),
            json!({"mcpServers":{"demo":{"command":"node","args":["server.js"]}}}).to_string(),
        )
        .unwrap();
        let item = service(&root, json!({"command": "node", "args": ["server.js"]}));
        let mut state = McpState {
            services: vec![item.clone()],
            instances: vec![],
            projects: vec![],
            revisions: BTreeMap::new(),
            issues: vec![],
            summary: super::super::McpSummary {
                total: 1,
                enabled: 1,
                disabled: 0,
                warnings: 0,
                shadowed: 0,
            },
            operation_warnings: vec![],
            sync_targets: vec![],
            sync_target_revisions: BTreeMap::new(),
        };
        let preview = preview_sync(&paths, &state, &item.locator).unwrap();
        apply_manual_sync(
            &paths,
            &state,
            &McpSyncApplyRequest {
                locator: item.locator.clone(),
                target_id: TARGET_ID.into(),
                expected_source_revision: preview.expected_source_revision,
                expected_target_revision: preview.expected_target_revision,
                expected_registry_revision: preview.expected_registry_revision,
            },
        )
        .unwrap();

        state.services[0].enabled = false;
        state.services[0].effective_state = McpEffectiveState::Disabled;
        reconcile_source_enabled(&paths, &state, &item.locator).unwrap();
        let disabled =
            normalized_target(&read_target(&paths.global_codex_config()).unwrap(), "demo")
                .unwrap()
                .unwrap();
        assert_eq!(disabled["enabled"], false);
        assert!(attach_sync_state(&paths, state.clone()).sync_targets[0].connected);

        let mut externally_enabled = read_target(&paths.global_codex_config()).unwrap();
        set_existing_server_enabled(&mut externally_enabled, "demo", true).unwrap();
        write_text_atomic(
            &paths.global_codex_config(),
            &externally_enabled.to_string(),
        )
        .unwrap();
        assert!(auto_sync_all(&paths, &state).unwrap());
        let corrected =
            normalized_target(&read_target(&paths.global_codex_config()).unwrap(), "demo")
                .unwrap()
                .unwrap();
        assert_eq!(corrected["enabled"], false);

        state.services[0].enabled = true;
        state.services[0].effective_state = McpEffectiveState::Effective;
        reconcile_source_enabled(&paths, &state, &item.locator).unwrap();
        let enabled =
            normalized_target(&read_target(&paths.global_codex_config()).unwrap(), "demo")
                .unwrap()
                .unwrap();
        assert_eq!(enabled["enabled"], true);
        assert!(attach_sync_state(&paths, state).sync_targets[0].connected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_registry_entries_default_to_selected() {
        let locator = McpLocator {
            scope: McpScope::User,
            name: "demo".into(),
            instance_id: None,
            project_path: None,
        };
        let value = json!({
            "version": 1,
            "entries": [{
                "source": locator,
                "targetPath": "config.toml",
                "autoSync": false,
                "lastSourceHash": "source",
                "lastTargetHash": "target"
            }]
        });
        let registry: SyncRegistry = serde_json::from_value(value).unwrap();
        assert!(registry.entries[0].selected);
    }

    #[test]
    fn auto_sync_updates_only_when_target_still_matches_last_sync() {
        let root = std::env::temp_dir().join(format!("ccm-openai-auto-{}", unique_suffix()));
        fs::create_dir_all(&root).unwrap();
        let paths = McpPaths::for_test(root.clone());
        fs::write(
            root.join(".claude.json"),
            json!({"mcpServers":{"demo":{"command":"node","args":["a.js"]}}}).to_string(),
        )
        .unwrap();
        let original = service(&root, json!({"command": "node", "args": ["a.js"]}));
        let mut state = McpState {
            services: vec![original.clone()],
            instances: vec![],
            projects: vec![],
            revisions: BTreeMap::new(),
            issues: vec![],
            summary: super::super::McpSummary {
                total: 1,
                enabled: 1,
                disabled: 0,
                warnings: 0,
                shadowed: 0,
            },
            operation_warnings: vec![],
            sync_targets: vec![],
            sync_target_revisions: BTreeMap::new(),
        };
        let preview = preview_sync(&paths, &state, &original.locator).unwrap();
        apply_manual_sync(
            &paths,
            &state,
            &McpSyncApplyRequest {
                locator: original.locator.clone(),
                target_id: TARGET_ID.into(),
                expected_source_revision: preview.expected_source_revision,
                expected_target_revision: preview.expected_target_revision,
                expected_registry_revision: preview.expected_registry_revision,
            },
        )
        .unwrap();
        let changed = service(&root, json!({"command": "node", "args": ["b.js"]}));
        state.services = vec![changed.clone()];
        assert!(auto_sync_all(&paths, &state).unwrap());
        let text = fs::read_to_string(paths.global_codex_config()).unwrap();
        assert!(text.contains("\"b.js\""));

        let mut target_doc = read_target(&paths.global_codex_config()).unwrap();
        let external = json!({"command": "external", "enabled": true})
            .as_object()
            .unwrap()
            .clone();
        apply_normalized(&mut target_doc, "demo", &external).unwrap();
        write_text_atomic(&paths.global_codex_config(), &target_doc.to_string()).unwrap();

        state.services = vec![service(&root, json!({"command": "node", "args": ["c.js"]}))];
        assert!(!auto_sync_all(&paths, &state).unwrap());
        let text = fs::read_to_string(paths.global_codex_config()).unwrap();
        assert!(text.contains("command = \"external\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn interrupted_atomic_write_is_restored_before_next_write() {
        let root = std::env::temp_dir().join(format!("ccm-openai-recover-{}", unique_suffix()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("config.toml");
        fs::write(&target, "original = true\n").unwrap();
        let rollback = root.join(format!(".ccm-openai-rollback-{}", target_hash(&target)));
        fs::rename(&target, &rollback).unwrap();

        let error = write_text_atomic(&target, "replacement = true\n").unwrap_err();
        assert!(error.contains("已恢复原配置"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "original = true\n");
        assert!(!rollback.exists());
        let _ = fs::remove_dir_all(root);
    }
}
