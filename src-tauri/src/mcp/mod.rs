// MCP 服务管理：领域类型、Tauri 命令、preview/apply 编排。
// 文件 IO 与作用域发现都在 storage；校验/脱敏/测试在 validation。
// 领域类型在本文件定义，storage/validation 通过 super::* 引用。
// 命令只使用 McpPaths::system()；测试通过 McpPaths::for_test 直接调 storage 内部函数。

mod codex_sync;
mod storage;
mod sync_targets;
mod validation;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use storage::McpPaths;

// ---------------- 领域类型 ----------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    User,
    Local,
    Project,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
    Ws,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum McpEffectiveState {
    Effective,
    PartiallyShadowed,
    Shadowed,
    Disabled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpLocator {
    pub scope: McpScope,
    pub name: String,
    pub instance_id: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSaveItem {
    pub target: McpLocator,
    pub config: Map<String, Value>,
    pub overwrite: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum McpChangeAction {
    Save {
        #[serde(default)]
        original: Option<McpLocator>,
        target: McpLocator,
        config: Map<String, Value>,
        #[serde(default)]
        overwrite: bool,
    },
    BatchSave {
        items: Vec<McpSaveItem>,
    },
    SetEnabled {
        target: McpLocator,
        enabled: bool,
    },
    Delete {
        target: McpLocator,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpChangeRequest {
    pub action: McpChangeAction,
    pub expected_revisions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInstanceRef {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpProjectRef {
    pub path: String,
    pub label: String,
    pub discovered: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpShadowRef {
    pub scope: McpScope,
    pub name: String,
    pub instance_id: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpService {
    pub locator: McpLocator,
    pub transport: McpTransport,
    pub raw_transport: Option<String>,
    pub config: Map<String, Value>,
    pub enabled: bool,
    pub effective_state: McpEffectiveState,
    pub shadowed_by: Vec<McpShadowRef>,
    pub shadowed_context_count: usize,
    pub source_id: String,
    pub revision: String,
    pub sensitive_paths: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSummary {
    pub total: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub warnings: usize,
    pub shadowed: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSourceIssue {
    pub source_id: String,
    pub path: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum McpSyncStatus {
    NotSynced,
    Synced,
    SourceUpdated,
    TargetModified,
    Conflict,
    Incompatible,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSyncTargetInfo {
    pub locator: McpLocator,
    pub target_id: String,
    pub target_label: String,
    pub status: McpSyncStatus,
    pub connected: bool,
    pub target_path: String,
    pub target_revision: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSyncPreview {
    pub locator: McpLocator,
    pub target_id: String,
    pub target_label: String,
    pub action_label: String,
    pub target_path: String,
    pub redacted_before: Option<Value>,
    pub redacted_after: Value,
    pub warnings: Vec<String>,
    pub expected_source_revision: String,
    pub expected_target_revision: String,
    pub expected_registry_revision: String,
    pub restart_hint: String,
    pub preserved_fields_note: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSyncApplyRequest {
    pub locator: McpLocator,
    pub target_id: String,
    pub expected_source_revision: String,
    pub expected_target_revision: String,
    pub expected_registry_revision: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTargetDisableRequest {
    pub locator: McpLocator,
    pub target_id: String,
    pub expected_target_revision: String,
    pub expected_registry_revision: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpState {
    pub services: Vec<McpService>,
    pub instances: Vec<McpInstanceRef>,
    pub projects: Vec<McpProjectRef>,
    pub revisions: BTreeMap<String, String>,
    pub issues: Vec<McpSourceIssue>,
    pub summary: McpSummary,
    pub operation_warnings: Vec<String>,
    pub sync_targets: Vec<McpSyncTargetInfo>,
    pub sync_target_revisions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAffectedSource {
    pub source_id: String,
    pub path: String,
    pub scope: McpScope,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchItem {
    pub name: String,
    pub scope: McpScope,
    pub source_id: String,
    pub redacted_before: Option<Value>,
    pub redacted_after: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpChangePreview {
    pub action_label: String,
    pub affected_sources: Vec<McpAffectedSource>,
    pub affected_instances: Vec<String>,
    pub redacted_before: Option<Value>,
    pub redacted_after: Option<Value>,
    pub batch_items: Vec<McpBatchItem>,
    pub user_sync_note: Option<String>,
    pub warnings: Vec<String>,
    pub expected_revisions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestRequest {
    #[allow(dead_code)]
    pub locator: Option<McpLocator>,
    pub name: String,
    pub config: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTestStageId {
    Schema,
    Command,
    Url,
    Endpoint,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTestStatus {
    Ok,
    Warn,
    Fail,
    Skipped,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestStage {
    pub id: McpTestStageId,
    pub status: McpTestStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestResult {
    pub ok: bool,
    pub transport: McpTransport,
    pub stages: Vec<McpTestStage>,
    pub sanitized_detail: String,
}

// ---------------- 命令 ----------------

use storage::{
    affected_source_ids, apply_action_files_checked, check_revisions, collect_state,
    read_locator_config, register_project, source_file, touches_user, touches_user_or_local,
    unregister_project, validate_action_strict,
};
use validation::{config_warnings, redact, sensitive_paths, test_basic, validate_name};

fn current_instances() -> Vec<String> {
    crate::configured_profile_names()
}

#[tauri::command]
pub fn list_mcp_services() -> Result<McpState, String> {
    let paths = McpPaths::system();
    let instances = current_instances();
    let state = collect_state(&paths, &instances);
    Ok(sync_targets::attach_sync_state(&paths, state))
}

#[tauri::command]
pub fn register_mcp_project(path: String) -> Result<McpState, String> {
    let paths = McpPaths::system();
    register_project(&paths, &path)?;
    let instances = current_instances();
    let state = collect_state(&paths, &instances);
    Ok(sync_targets::attach_sync_state(&paths, state))
}

#[tauri::command]
pub fn unregister_mcp_project(path: String) -> Result<McpState, String> {
    let paths = McpPaths::system();
    unregister_project(&paths, &path)?;
    let instances = current_instances();
    let state = collect_state(&paths, &instances);
    Ok(sync_targets::attach_sync_state(&paths, state))
}

#[tauri::command]
pub fn preview_mcp_target_sync(
    target_id: String,
    locator: McpLocator,
) -> Result<McpSyncPreview, String> {
    let paths = McpPaths::system();
    let state = collect_state(&paths, &current_instances());
    sync_targets::preview_sync(&target_id, &paths, &state, &locator)
}

#[tauri::command]
pub fn apply_mcp_target_sync(request: McpSyncApplyRequest) -> Result<McpState, String> {
    let paths = McpPaths::system();
    let instances = current_instances();
    let state = collect_state(&paths, &instances);
    sync_targets::apply_sync(&paths, &state, &request)?;
    Ok(sync_targets::attach_sync_state(
        &paths,
        collect_state(&paths, &instances),
    ))
}

#[tauri::command]
pub fn disable_mcp_target(request: McpTargetDisableRequest) -> Result<McpState, String> {
    let paths = McpPaths::system();
    let instances = current_instances();
    let state = collect_state(&paths, &instances);
    sync_targets::disable_target(&paths, &state, &request)?;
    Ok(sync_targets::attach_sync_state(
        &paths,
        collect_state(&paths, &instances),
    ))
}

#[tauri::command]
pub fn preview_mcp_change(request: McpChangeRequest) -> Result<McpChangePreview, String> {
    let paths = McpPaths::system();
    build_preview(&paths, &current_instances(), &request)
}

#[tauri::command]
pub fn apply_mcp_change(request: McpChangeRequest) -> Result<McpState, String> {
    let paths = McpPaths::system();
    let instances = current_instances();
    let action = &request.action;

    // User/Local 写入必须与 CLI --sync 串行化；拿不到锁直接报错，不静默跳过。
    let _guard = if touches_user_or_local(action) {
        match crate::sync::acquire_config_lock() {
            Some(g) => Some(g),
            None => return Err("另一个同步正在进行，请稍后重试".into()),
        }
    } else {
        None
    };

    // 受影响来源会依赖服务当前位于启用源还是停用仓库，必须在配置锁内重新推导；
    // 否则状态可能在“计算 affected”和“获取锁”之间变化，造成 revision 集合漏项。
    let affected = affected_source_ids(&paths, &instances, action)?;
    // 锁内重新读取 revision 校验，拒绝外部并发改写。
    check_revisions(&paths, &instances, &affected, &request.expected_revisions)?;

    // 所有作用域统一走 workbench 事务；Project 启停由 disable_locator/enable_locator
    // 内部分发到 settings.local.json，无需特殊分支（避免空实例数组漏校验次级实例发现的项目）。
    apply_action_files_checked(&paths, &instances, action, &request.expected_revisions)?;

    // User 变更后跨实例同步：主账户写成功即为目标态，部分实例失败保留并明确返回 warning。
    let mut operation_warnings = Vec::new();
    if touches_user(action) {
        match crate::sync::sync_configs_locked(&current_instances()) {
            Ok(outcome) => operation_warnings.extend(outcome.warnings),
            Err(e) => {
                let warning = format!("MCP 变更后的跨实例同步失败：{e}");
                crate::sync::log_line(&warning);
                operation_warnings.push(warning);
            }
        }
    }

    let mut state = collect_state(&paths, &current_instances());
    if let McpChangeAction::SetEnabled { target, .. } = action {
        if let Err(error) = sync_targets::reconcile_source_enabled(&paths, &state, target) {
            let warning = format!("目标端 MCP 状态联动失败，将在后台继续重试：{error}");
            crate::sync::log_line(&warning);
            operation_warnings.push(warning);
        }
    }
    state.summary.warnings += operation_warnings.len();
    state.operation_warnings = operation_warnings;
    Ok(sync_targets::attach_sync_state(&paths, state))
}

#[tauri::command]
pub async fn test_mcp_server(request: McpTestRequest) -> Result<McpTestResult, String> {
    let name = request.name.clone();
    let config = request.config.clone();
    tauri::async_runtime::spawn_blocking(move || test_basic(&name, &config))
        .await
        .map_err(|e| format!("测试任务异常：{e}"))
}

// ---------------- preview 构建 ----------------

pub fn start_mcp_sync_monitors(app: tauri::AppHandle) {
    sync_targets::start_monitors(app);
}

fn build_preview(
    paths: &McpPaths,
    instances: &[String],
    request: &McpChangeRequest,
) -> Result<McpChangePreview, String> {
    let action = &request.action;
    // preview 与 apply 共用同一套硬校验：非法 locator/名称/配置/未登记项目直接拒绝预览。
    validate_action_strict(paths, instances, action)?;
    // 来源读取失败时拒绝预览（不把异常配置当成“不存在”）
    let warnings = validate_action(paths, instances, action)?;

    let affected = affected_source_ids(paths, instances, action)?;
    // 先用页面传入的 revision 检测并发修改：冲突则拒绝预览
    check_revisions(paths, instances, &affected, &request.expected_revisions)?;
    // 通过后才计算 apply 用的新 expectedRevisions
    let mut expected_revisions = BTreeMap::new();
    let mut affected_sources = Vec::new();
    for sid in &affected {
        let path = source_file(paths, instances, sid)?;
        expected_revisions.insert(sid.clone(), storage::revision(&path));
        affected_sources.push(McpAffectedSource {
            source_id: sid.clone(),
            path: path.display().to_string(),
            scope: source_scope_of(sid),
        });
    }

    let (action_label, affected_instances, before, after) =
        describe_action(paths, instances, action)?;

    let redacted_before = before.map(|v| {
        let p = sensitive_paths(&v);
        redact(&v, &p)
    });
    let redacted_after = after.map(|v| {
        let p = sensitive_paths(&v);
        redact(&v, &p)
    });

    // BatchSave：按各 item 展示脱敏前后，便于逐项确认。
    let batch_items: Vec<McpBatchItem> = match action {
        McpChangeAction::BatchSave { items } => items
            .iter()
            .map(|it| -> Result<McpBatchItem, String> {
                let before = read_locator_config(paths, instances, &it.target)?.map(Value::Object);
                let after = Some(Value::Object(it.config.clone()));
                Ok(McpBatchItem {
                    name: it.target.name.clone(),
                    scope: it.target.scope.clone(),
                    source_id: storage::locator_source_ids(&it.target, instances)
                        .into_iter()
                        .next()
                        .unwrap_or_default(),
                    redacted_before: before.map(|v| {
                        let p = sensitive_paths(&v);
                        redact(&v, &p)
                    }),
                    redacted_after: after.map(|v| {
                        let p = sensitive_paths(&v);
                        redact(&v, &p)
                    }),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };

    // User 变更：区分直接写入主账户与随后同步实例（非原子）。
    let user_sync_note = if touches_user(action) {
        Some(
            "直接修改主账户 ~/.claude.json，随后同步到全部实例（非原子：部分实例失败将在下次同步收敛）"
                .to_string(),
        )
    } else {
        None
    };

    // Project 变更：提示 settings.local.json 保持未提交。
    let mut warnings = warnings;
    if action_touches_project(action) {
        warnings.push(
            ".claude/settings.local.json 是本机停用记录，请保持未提交；.mcp.json 如需团队共享可提交"
                .to_string(),
        );
    }

    Ok(McpChangePreview {
        action_label,
        affected_sources,
        affected_instances,
        redacted_before,
        redacted_after,
        batch_items,
        user_sync_note,
        warnings,
        expected_revisions,
    })
}

fn action_touches_project(action: &McpChangeAction) -> bool {
    let p = |l: &McpLocator| l.scope == McpScope::Project;
    match action {
        McpChangeAction::Save {
            original, target, ..
        } => original.as_ref().map(p).unwrap_or(false) || p(target),
        McpChangeAction::BatchSave { items } => items.iter().any(|i| p(&i.target)),
        McpChangeAction::SetEnabled { target, .. } | McpChangeAction::Delete { target } => {
            p(target)
        }
    }
}

fn source_scope_of(sid: &str) -> McpScope {
    if sid.starts_with("user:") {
        McpScope::User
    } else if sid.starts_with("local:") {
        McpScope::Local
    } else if sid.starts_with("project") {
        McpScope::Project
    } else {
        McpScope::User
    }
}

/// 收集动作的结构性警告（保留名、SSE 弃用、配置体积等）。来源读取失败时返回错误阻断预览。
fn validate_action(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let check_cfg = |name: &str, cfg: &Map<String, Value>, out: &mut Vec<String>| {
        if let Err(e) = validate_name(name) {
            out.push(format!("名称：{e}"));
        }
        for w in config_warnings(cfg) {
            out.push(w);
        }
    };
    match action {
        McpChangeAction::Save { target, config, .. } => check_cfg(&target.name, config, &mut out),
        McpChangeAction::BatchSave { items } => {
            for it in items {
                check_cfg(&it.target.name, &it.config, &mut out);
            }
        }
        McpChangeAction::SetEnabled { target, .. } | McpChangeAction::Delete { target } => {
            match read_locator_config(paths, instances, target) {
                Ok(Some(cfg)) => {
                    for w in config_warnings(&cfg) {
                        out.push(w);
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(out)
}

type ActionDescription = (String, Vec<String>, Option<Value>, Option<Value>);

/// 生成动作文案、受影响实例、及脱敏前的前后配置。来源读取失败时返回错误阻断预览。
fn describe_action(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
) -> Result<ActionDescription, String> {
    match action {
        McpChangeAction::Save {
            original,
            target,
            config,
            ..
        } => {
            // target 为空时回退读 original；original 来源损坏/不可读/结构非法必须直接报错，
            // 禁止用 .ok()/.flatten() 擦除成“配置不存在”，导致 preview 成功却缺真实 before。
            let before = match read_locator_config(paths, instances, target)? {
                Some(config) => Some(config),
                None => match original {
                    Some(original) => read_locator_config(paths, instances, original)?,
                    None => None,
                },
            }
            .map(Value::Object);
            let after = Some(Value::Object(config.clone()));
            let label = if original.as_ref() == Some(target) {
                format!("更新服务「{}」", target.name)
            } else if original.is_some() {
                format!("移动/重命名服务为「{}」", target.name)
            } else {
                format!("新建服务「{}」", target.name)
            };
            Ok((label, instances_for(target, instances), before, after))
        }
        McpChangeAction::BatchSave { items } => {
            let mut all = Vec::new();
            for it in items {
                for i in instances_for(&it.target, instances) {
                    if !all.contains(&i) {
                        all.push(i);
                    }
                }
            }
            Ok((format!("批量保存 {} 个服务", items.len()), all, None, None))
        }
        McpChangeAction::SetEnabled { target, enabled } => {
            let cfg = read_locator_config(paths, instances, target)?.map(Value::Object);
            let label = if *enabled {
                format!("启用服务「{}」", target.name)
            } else {
                format!("停用服务「{}」", target.name)
            };
            Ok((label, instances_for(target, instances), cfg.clone(), cfg))
        }
        McpChangeAction::Delete { target } => {
            let cfg = read_locator_config(paths, instances, target)?.map(Value::Object);
            Ok((
                format!("删除服务「{}」", target.name),
                instances_for(target, instances),
                cfg,
                None,
            ))
        }
    }
}

fn instances_for(loc: &McpLocator, instances: &[String]) -> Vec<String> {
    match loc.scope {
        McpScope::User => {
            let mut v = vec!["__main__".to_string()];
            for n in instances {
                if !v.contains(n) {
                    v.push(n.clone());
                }
            }
            v
        }
        McpScope::Local => loc.instance_id.clone().map(|i| vec![i]).unwrap_or_default(),
        McpScope::Project => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    fn setup() -> (McpPaths, Temp) {
        let dir = std::env::temp_dir().join(format!(
            "ccm-mcp-mod-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let paths = McpPaths::for_test(dir.clone());
        (paths, Temp(dir))
    }

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_main(paths: &McpPaths, v: Value) {
        storage::write_json_transactional(paths, &paths.main_claude_json(), &v).unwrap();
    }
    fn map_of(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }
    fn user_locator(name: &str) -> McpLocator {
        McpLocator {
            scope: McpScope::User,
            name: name.into(),
            instance_id: None,
            project_path: None,
        }
    }
    fn save_action(target: McpLocator, config: Map<String, Value>) -> McpChangeAction {
        McpChangeAction::Save {
            original: None,
            target,
            config,
            overwrite: false,
        }
    }
    /// 取某 sourceId 当前文件 revision（用于构造 expected_revisions）。
    fn rev_of(paths: &McpPaths, sid: &str) -> String {
        let p = storage::source_file(paths, &[], sid).unwrap();
        storage::revision(&p)
    }

    #[test]
    fn preview_rejects_corrupt_original_source_bytes_unchanged() {
        // original 来源 JSON 损坏，target 位于其他合法来源：preview 必须返回 Err，
        // 错误发生后所有来源文件原始字节不变。
        let (paths, _t) = setup();
        let proj = paths.home.join("p");
        fs::create_dir_all(&proj).unwrap();
        storage::register_project(&paths, &proj.display().to_string()).unwrap();
        // original = user old，其来源 main .claude.json 损坏
        fs::write(paths.main_claude_json(), "{ broken json").unwrap();
        let main_bytes = fs::read(paths.main_claude_json()).unwrap();
        // 预填 expected_revisions，确保失败来自 original 来源读取而非 revision 缺失
        let project_sid = format!("project:{}", proj.display());
        let mut expected = BTreeMap::new();
        expected.insert("user:__main__".to_string(), rev_of(&paths, "user:__main__"));
        expected.insert(
            project_sid,
            rev_of(&paths, &format!("project:{}", proj.display())),
        );
        let original = user_locator("old");
        let target = McpLocator {
            scope: McpScope::Project,
            name: "new".into(),
            instance_id: None,
            project_path: Some(proj.display().to_string()),
        };
        let request = McpChangeRequest {
            action: McpChangeAction::Save {
                original: Some(original),
                target,
                config: map_of(json!({"command":"node"})),
                overwrite: false,
            },
            expected_revisions: expected,
        };
        let res = build_preview(&paths, &[], &request);
        assert!(res.is_err(), "original 来源损坏时 preview 必须拒绝");
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            main_bytes,
            "原始 bytes 不变"
        );
        assert!(!proj.join(".mcp.json").exists(), "target 来源不应被创建");
    }

    #[test]
    fn preview_rejects_revision_conflict_after_external_change() {
        // list 后外部修改来源：preview 因 revision 冲突拒绝
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"a":{"command":"node"}}}));
        let mut expected = BTreeMap::new();
        expected.insert("user:__main__".to_string(), rev_of(&paths, "user:__main__"));
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_main(&paths, json!({"mcpServers":{"a":{"command":"python"}}}));
        let request = McpChangeRequest {
            action: save_action(user_locator("b"), map_of(json!({"command":"go"}))),
            expected_revisions: expected,
        };
        assert!(
            build_preview(&paths, &[], &request).is_err(),
            "外部修改后 revision 冲突应拒绝 preview"
        );
    }

    #[test]
    fn preview_rejects_missing_expected_revision() {
        // affected source 缺少 expected revision：preview 拒绝
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let request = McpChangeRequest {
            action: save_action(user_locator("b"), map_of(json!({"command":"go"}))),
            expected_revisions: BTreeMap::new(),
        };
        assert!(
            build_preview(&paths, &[], &request).is_err(),
            "缺少 affected source 的 expected revision 应拒绝 preview"
        );
    }

    #[test]
    fn preview_allows_create_when_source_missing() {
        // 明确 "missing" revision 且文件确实不存在时允许创建
        let (paths, _t) = setup();
        assert!(!paths.main_claude_json().exists());
        let mut expected = BTreeMap::new();
        expected.insert("user:__main__".to_string(), "missing".to_string());
        let request = McpChangeRequest {
            action: save_action(user_locator("b"), map_of(json!({"command":"go"}))),
            expected_revisions: expected,
        };
        let res = build_preview(&paths, &[], &request);
        assert!(
            res.is_ok(),
            "文件不存在且 revision=missing 应允许创建预览，got: {:?}",
            res.err()
        );
    }

    #[test]
    fn preview_rejects_corrupt_enabled_source() {
        // 损坏 enabled 来源：describe_action 读取 target 来源失败 → preview 拒绝
        let (paths, _t) = setup();
        fs::write(paths.main_claude_json(), "{ not json").unwrap();
        let bytes = fs::read(paths.main_claude_json()).unwrap();
        let mut expected = BTreeMap::new();
        expected.insert("user:__main__".to_string(), rev_of(&paths, "user:__main__"));
        let request = McpChangeRequest {
            action: save_action(user_locator("x"), map_of(json!({"command":"node"}))),
            expected_revisions: expected,
        };
        assert!(
            build_preview(&paths, &[], &request).is_err(),
            "损坏 enabled 来源应拒绝 preview"
        );
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            bytes,
            "原始 bytes 不变"
        );
    }

    #[test]
    fn preview_rejects_corrupt_disabled_store() {
        // 损坏 disabled store：affected_source_ids 判定 target 是否停用时失败 → preview 拒绝
        let (paths, _t) = setup();
        fs::write(paths.disabled_store(), json!({"version":"bad"}).to_string()).unwrap();
        write_main(&paths, json!({}));
        let request = McpChangeRequest {
            action: save_action(user_locator("x"), map_of(json!({"command":"node"}))),
            expected_revisions: BTreeMap::new(),
        };
        assert!(
            build_preview(&paths, &[], &request).is_err(),
            "损坏 disabled store 应拒绝 preview"
        );
    }

    #[test]
    fn project_preview_ignores_unrelated_corrupt_disabled_store() {
        let (paths, _t) = setup();
        let project = paths.home.join("project-preview");
        fs::create_dir_all(&project).unwrap();
        storage::register_project(&paths, &project.display().to_string()).unwrap();
        fs::write(
            paths.disabled_store(),
            json!({"version": "broken", "entries": []}).to_string(),
        )
        .unwrap();
        let disabled_bytes = fs::read(paths.disabled_store()).unwrap();
        let target = McpLocator {
            scope: McpScope::Project,
            name: "project-service".into(),
            instance_id: None,
            project_path: Some(project.display().to_string()),
        };
        let project_sid = storage::source_id::project(&project.display().to_string());
        let mut expected = BTreeMap::new();
        expected.insert(project_sid.clone(), rev_of(&paths, &project_sid));
        let request = McpChangeRequest {
            action: save_action(target, map_of(json!({"command": "node"}))),
            expected_revisions: expected,
        };

        let preview = build_preview(&paths, &[], &request).unwrap();
        assert_eq!(preview.affected_sources.len(), 1);
        assert_eq!(preview.affected_sources[0].source_id, project_sid);
        assert_eq!(
            fs::read(paths.disabled_store()).unwrap(),
            disabled_bytes,
            "preview 不得修改 disabled store"
        );
    }
}
