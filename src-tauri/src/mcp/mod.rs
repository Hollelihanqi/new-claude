// MCP 服务管理：领域类型、Tauri 命令、preview/apply 编排。
// 文件 IO 与作用域发现都在 storage；校验/脱敏/测试在 validation。
// 领域类型在本文件定义，storage/validation 通过 super::* 引用。
// 命令只使用 McpPaths::system()；测试通过 McpPaths::for_test 直接调 storage 内部函数。

mod codex_sync;
mod storage;
mod sync_targets;
mod update;
mod validation;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(crate) use codex_sync::write_guard as target_write_guard;
pub(crate) use storage::{
    environment_cleanup_files, forget_environment_records, repair_backup_permissions, McpPaths,
};

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

/// 一条"死条目"：记录存在、目录已确认不存在（fs::metadata 返回 NotFound）。
/// 只有这类条目允许「一键清理」；权限不足/路径非法/不是目录等一律不进此列表。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDeadEntry {
    /// user:<instance>（.claude.json 的 projects 键）| manager:projects（登记表条目）
    pub source_id: String,
    /// 记录中的原始路径/键（清理时按它定位删除）
    pub raw_path: String,
    /// 所在文件（.claude.json 或 mcp-projects.json），用于展示
    pub file_path: String,
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
    /// 可被「一键清理」安全删除的死条目（目录确认不存在）。
    /// 与 issues 有意冗余：issues 负责"出了什么问题"，deadEntries 负责"哪些能清"。
    pub dead_entries: Vec<McpDeadEntry>,
    pub summary: McpSummary,
    pub operation_warnings: Vec<String>,
    pub sync_targets: Vec<McpSyncTargetInfo>,
    pub sync_target_revisions: BTreeMap<String, String>,
    /// 哪些环境的哪些条目**覆盖了共享配置**（决策 7.2 要求界面必须显示，
    /// 而不是静默以环境为准 —— 静默会让用户以为共享值已经生效）。
    pub shared_overrides: Vec<SharedOverride>,
}

/// 一条"环境覆盖了共享配置"的记录。`shared_value` 与 `env_value` 一起给出来，
/// 界面才能展示"两边差异"。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedOverride {
    pub env: String,
    pub name: String,
    /// 人类可读的原因（见 `shared_config::OverrideReason::label`）
    pub reason: String,
    pub shared_value: Option<Value>,
    pub env_value: Option<Value>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpConnectionState {
    Connected,
    Failed,
    Pending,
    Unknown,
}

/// Claude Code 在一个受管环境中实际完成 MCP 握手后的状态。
/// 这与 `McpService.enabled`（配置开关）是两个完全不同的概念。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionCheck {
    pub environment: String,
    pub name: String,
    pub status: McpConnectionState,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionReport {
    pub checks: Vec<McpConnectionCheck>,
    pub errors: Vec<String>,
}

pub use update::{McpUpdateInfo, McpUpdateReport};

// ---------------- 命令 ----------------

use storage::{
    affected_source_ids, apply_action_files_checked, check_revisions, cleanup_dead_entries,
    collect_state, read_locator_config, register_project, source_file, touches_user,
    touches_user_or_local, unregister_project, validate_action_strict, CleanupReport,
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

/// 一键清理的结果。结构化返回（而非单个字符串）：前端需要区分
/// 全部成功 / 无事可做 / 部分失败来决定通知颜色与是否关闭确认弹窗。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCleanupResult {
    pub removed_count: usize,
    /// 全部未清理条目数（含稳定跳过与可重试的并发冲突）
    pub blocked_count: usize,
    /// `blocked_count` 中「重试可能成功」的那部分（并发冲突），其余是稳定状态
    pub retryable_count: usize,
    pub write_error_count: usize,
    /// true = 没有留下任何**未完成**的工作：既没有写失败，也没有可重试的冲突。
    /// 前端的颜色/是否关闭弹窗一律以它为准，不再自行用 write_error_count 重新推导
    /// （两处推导必然分叉：并发冲突不改 write_errors，只看写失败会把它误判成成功）。
    pub complete: bool,
    /// 人类可读报告（清理分组、跳过原因、失败文件、备份位置）
    pub message: String,
}

/// 一键清理两类"死条目"（目录确认不存在，fs::metadata == NotFound）：
/// 1) 各环境用户级源文件 projects 下的死键；2) 登记表 mcp-projects.json 的死条目。
/// 其他失败原因（权限不足/路径非法/不是目录/位置不确定/文件被并发修改）一律跳过并在消息里说明。
#[tauri::command]
pub fn cleanup_dead_project_entries() -> Result<McpCleanupResult, String> {
    let paths = McpPaths::system();
    let instances = current_instances();
    // 写用户级源文件必须与 CLI --sync 串行化（先例 apply_mcp_change）；
    // 拿不到锁直接报错，不静默跳过。
    let _guard = match crate::sync::acquire_config_lock() {
        Some(g) => g,
        None => return Err("另一个同步正在进行，请稍后重试".into()),
    };
    let report = cleanup_dead_entries(&paths, &instances)?;
    Ok(cleanup_result(&report, &paths))
}

/// [CleanupReport] → 前端消费的结构化结果。
///
/// 抽成函数是为了让测试直接跑**生产映射**：此前测试内联抄了一份同样的映射，
/// 改 `complete` 语义时两边会静默分叉。
fn cleanup_result(report: &CleanupReport, paths: &McpPaths) -> McpCleanupResult {
    McpCleanupResult {
        removed_count: report.removed.len(),
        blocked_count: report.blocked.len(),
        retryable_count: report.retryable_count(),
        write_error_count: report.write_errors.len(),
        // 「未完成」= 有写失败，或留下重试可能成功的冲突。稳定跳过（目录已重建、
        // 权限不足等）不算未完成，重试也改变不了结果。
        complete: report.write_errors.is_empty() && report.retryable_count() == 0,
        message: format_cleanup_report(report, paths),
    }
}

/// 把清理报告格式化为一条人类可读消息（前端 notifications.show 直接展示）。
fn format_cleanup_report(report: &CleanupReport, paths: &McpPaths) -> String {
    use std::fmt::Write;
    if report.removed.is_empty() && report.blocked.is_empty() && report.write_errors.is_empty() {
        return "没有可清理的死条目".into();
    }
    let mut msg = String::new();
    if !report.removed.is_empty() {
        let mut keys_by_env: BTreeMap<String, usize> = BTreeMap::new();
        let mut registry_count = 0usize;
        for e in &report.removed {
            match e {
                storage::DeadEntry::ProjectKey { instance, .. } => {
                    *keys_by_env.entry(instance.clone()).or_default() += 1;
                }
                storage::DeadEntry::RegistryProject { .. } => registry_count += 1,
            }
        }
        let mut parts = Vec::new();
        if !keys_by_env.is_empty() {
            let total: usize = keys_by_env.values().sum();
            let detail = keys_by_env
                .iter()
                .map(|(env, n)| format!("{env}×{n}"))
                .collect::<Vec<_>>()
                .join("、");
            parts.push(format!("项目键 {total} 条（{detail}）"));
        }
        if registry_count > 0 {
            parts.push(format!("项目登记表 {registry_count} 条"));
        }
        let _ = write!(
            msg,
            "已清理 {}：{}；原文件已自动备份到 {}",
            report.removed.len(),
            parts.join("、"),
            paths.backup_dir().display()
        );
    } else {
        msg.push_str("没有清理任何条目");
    }
    if !report.blocked.is_empty() {
        // 有重试可能的先点名，否则用户只看到"跳过"会以为无事可做。
        let retryable = report.retryable_count();
        if retryable > 0 {
            let _ = write!(msg, "；其中 {retryable} 条可在稍后重试");
        }
        let _ = write!(msg, "；跳过 {} 条：", report.blocked.len());
        let details = report
            .blocked
            .iter()
            .map(|b| format!("{}（{}）", b.raw, b.reason))
            .collect::<Vec<_>>()
            .join("、");
        msg.push_str(&details);
    }
    if !report.write_errors.is_empty() {
        let _ = write!(msg, "；注意：以下文件写入失败：");
        let details = report
            .write_errors
            .iter()
            .map(|(file, err)| format!("{file}：{err}"))
            .collect::<Vec<_>>()
            .join("、");
        msg.push_str(&details);
    }
    msg
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
    // 内部分发到 settings.local.json，无需特殊分支（避免空环境数组漏校验次级环境发现的项目）。
    apply_action_files_checked(&paths, &instances, action, &request.expected_revisions)?;

    // User 变更后跨环境同步：默认 Claude写成功即为目标态，部分环境失败保留并明确返回 warning。
    let mut operation_warnings = Vec::new();
    if touches_user(action) {
        match crate::sync::sync_configs_locked(&current_instances()) {
            Ok(outcome) => operation_warnings.extend(outcome.warnings),
            Err(e) => {
                let warning = format!("MCP 变更后的跨环境同步失败：{e}");
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

/// 获取所有受管环境的真实 MCP 连接状态。
///
/// 当前列表中的「所有环境」服务会被分发到每个环境，因此必须逐环境检查；只看配置
/// 是否启用会把“进程启动失败 / MCP 握手失败”错误地展示为正常。
#[tauri::command]
pub async fn probe_mcp_connections() -> Result<McpConnectionReport, String> {
    let paths = McpPaths::system();
    let instances = current_instances();
    let state = collect_state(&paths, &instances);
    let names = state
        .services
        .iter()
        .filter(|service| service.locator.scope == McpScope::User && service.enabled)
        .map(|service| service.locator.name.clone())
        .collect::<Vec<_>>();
    let overridden_entries = state
        .shared_overrides
        .iter()
        .map(|item| (item.env.clone(), item.name.clone()))
        .collect::<std::collections::HashSet<_>>();
    let stdio_commands = state
        .services
        .iter()
        .filter(|service| {
            service.locator.scope == McpScope::User
                && service.enabled
                && service.transport == McpTransport::Stdio
        })
        .filter_map(|service| {
            let command = service.config.get("command")?.as_str()?;
            (!command.contains("${")).then(|| (service.locator.name.clone(), command.to_string()))
        })
        .collect::<Vec<_>>();

    tauri::async_runtime::spawn_blocking(move || {
        let runtime_path = crate::claude_cli::refreshed_runtime_path();
        let unresolved = stdio_commands
            .iter()
            .filter(|(_, command)| {
                crate::claude_cli::resolve_runtime_command(command, runtime_path.as_deref())
                    .is_none()
            })
            .map(|(name, command)| (name.clone(), command.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut checks = Vec::new();
        let mut errors = Vec::new();
        for environment in instances {
            let config_dir = crate::sync::instance_dir(&environment);
            match crate::claude_cli::run_mcp_health_list(&config_dir, runtime_path.as_deref()) {
                Ok(output) => {
                    let mut environment_checks =
                        parse_mcp_health_output(&environment, &names, &output);
                    for check in &mut environment_checks {
                        if !overridden_entries
                            .contains(&(check.environment.clone(), check.name.clone()))
                        {
                            if let Some(command) = unresolved.get(&check.name) {
                                check.status = McpConnectionState::Failed;
                                check.detail = format!(
                                    "找不到启动程序「{command}」；已重新读取系统 PATH，仍未找到"
                                );
                            }
                        }
                    }
                    checks.extend(environment_checks);
                }
                Err(error) => errors.push(format!("{environment}：{error}")),
            }
        }
        McpConnectionReport { checks, errors }
    })
    .await
    .map_err(|e| format!("MCP 连接检查任务异常：{e}"))
}

/// 只分析本地配置，不访问网络。用于列表首次展示版本来源和更新开关能力。
#[tauri::command]
pub fn list_mcp_update_info() -> Result<McpUpdateReport, String> {
    update::list_update_info(&McpPaths::system(), &current_instances())
}

/// 检测可更新 MCP 的最新版本。`target=None` 时只检测已开启更新检测的全部条目；
/// 指定 target 时也仍尊重该条目的开关，不会绕过用户选择。
#[tauri::command]
pub async fn check_mcp_updates(target: Option<McpLocator>) -> Result<McpUpdateReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        update::check_updates(&McpPaths::system(), &current_instances(), target.as_ref())
    })
    .await
    .map_err(|error| format!("MCP 版本检测任务异常：{error}"))?
}

/// 更新检测偏好属于 PathMux 自身，不写入或污染 Claude MCP 配置。
#[tauri::command]
pub fn set_mcp_update_check(target: McpLocator, enabled: bool) -> Result<McpUpdateInfo, String> {
    update::set_update_check(&McpPaths::system(), &current_instances(), &target, enabled)
}

fn safe_mcp_connection_detail(status: &McpConnectionState, result: &str) -> String {
    let normalized = result.to_ascii_lowercase();
    match status {
        McpConnectionState::Connected => "Claude Code：已连接".into(),
        McpConnectionState::Pending => "Claude Code：等待授权".into(),
        McpConnectionState::Unknown => "Claude Code：状态无法识别".into(),
        McpConnectionState::Failed => {
            if normalized.contains("enoent")
                || normalized.contains("not found")
                || normalized.contains("not recognized")
                || normalized.contains("failed to spawn")
            {
                "Claude Code：找不到启动程序".into()
            } else if normalized.contains("timed out") || normalized.contains("timeout") {
                "Claude Code：连接超时".into()
            } else if normalized.contains("exited")
                || normalized.contains("exit code")
                || normalized.contains("terminated")
            {
                "Claude Code：服务进程启动后退出".into()
            } else {
                "Claude Code：MCP 握手失败".into()
            }
        }
    }
}

fn parse_mcp_health_output(
    environment: &str,
    names: &[String],
    output: &str,
) -> Vec<McpConnectionCheck> {
    let clean = strip_terminal_escapes(output);
    names
        .iter()
        .map(|name| {
            let line = clean.lines().find(|line| {
                let trimmed = line.trim_start();
                trimmed == name
                    || trimmed
                        .strip_prefix(name.as_str())
                        .is_some_and(|rest| rest.starts_with(':') || rest.starts_with(' '))
            });
            let (status, detail) = match line {
                Some(line) => {
                    let normalized = line.to_ascii_lowercase();
                    let status = if normalized.contains("failed")
                        || normalized.contains("error")
                        || normalized.contains("disconnected")
                        || line.contains('✗')
                        || line.contains('×')
                    {
                        McpConnectionState::Failed
                    } else if normalized.contains("pending")
                        || normalized.contains("approval")
                        || normalized.contains("approve")
                    {
                        McpConnectionState::Pending
                    } else if normalized.contains("connected")
                        || line.contains('✓')
                        || line.contains('✔')
                    {
                        McpConnectionState::Connected
                    } else {
                        McpConnectionState::Unknown
                    };
                    // `claude mcp list` 在状态前还会回显完整 command / URL；其中可能
                    // 带令牌或查询参数。界面只需要状态结论，绝不能把整条命令送到前端。
                    let result = line
                        .rsplit_once(" - ")
                        .map(|(_, result)| result.trim())
                        .unwrap_or_default();
                    let detail = safe_mcp_connection_detail(&status, result);
                    (status, detail)
                }
                None => (
                    McpConnectionState::Unknown,
                    "Claude Code 未返回该服务的状态".into(),
                ),
            };
            McpConnectionCheck {
                environment: environment.to_string(),
                name: name.clone(),
                status,
                detail,
            }
        })
        .collect()
}

/// 去掉 CLI 彩色输出中的 ANSI/OSC 控制序列，避免状态关键字被转义码截断。
fn strip_terminal_escapes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) | None => {}
        }
    }
    out
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

    // 「所有环境」的变更：写的是**应用共享库**，随后**单向分发**到各环境。
    // 旧文案说的是"直接修改默认 Claude ~/.claude.json，随后同步到全部环境"——
    // 决策 7.2 之后默认 Claude 已退出这条链，那句话不再成立。
    let user_sync_note = if touches_user(action) {
        Some(
            "写入应用共享库 ~/.cc-manager/shared/mcp.json，随后单向分发到全部环境\
             （非原子：个别环境失败会在下次分发时收敛；环境里同名的自有配置优先，不会被覆盖）"
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

/// 生成动作文案、受影响环境、及脱敏前的前后配置。来源读取失败时返回错误阻断预览。
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
                format!("允许 Claude 加载「{}」", target.name)
            } else {
                format!("停止让 Claude 加载「{}」", target.name)
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
        // User 在领域模型里代表 PathMux 共享源，不是默认 Claude。
        // `__main__` 只是后端定位共享文件的内部标识，绝不能出现在用户可见的
        // “受影响环境”里，否则会让人误以为默认 Claude 也被改写。
        McpScope::User => instances.to_vec(),
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
        storage::write_json_transactional(paths, &paths.shared_mcp_json(), &v).unwrap();
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
        fs::write(paths.shared_mcp_json(), "{ broken json").unwrap();
        let main_bytes = fs::read(paths.shared_mcp_json()).unwrap();
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
            fs::read(paths.shared_mcp_json()).unwrap(),
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
        assert!(!paths.shared_mcp_json().exists());
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
        fs::write(paths.shared_mcp_json(), "{ not json").unwrap();
        let bytes = fs::read(paths.shared_mcp_json()).unwrap();
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
            fs::read(paths.shared_mcp_json()).unwrap(),
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

    #[test]
    fn cleanup_report_message_lists_counts_and_caveats() {
        let (paths, _t) = setup();
        let report = CleanupReport {
            removed: vec![
                storage::DeadEntry::ProjectKey {
                    instance: "hq".into(),
                    raw_key: "E:/dead-hq".into(),
                    file: PathBuf::from("f1"),
                },
                storage::DeadEntry::ProjectKey {
                    instance: "ds".into(),
                    raw_key: "E:/dead-ds".into(),
                    file: PathBuf::from("f2"),
                },
                storage::DeadEntry::RegistryProject {
                    raw_path: "E:/dead-reg".into(),
                },
            ],
            blocked: vec![storage::BlockedEntry::stable(
                "E:/a-file",
                "不是目录：E:/a-file",
            )],
            write_errors: vec![("f3.json".to_string(), "boom".to_string())],
        };
        let msg = format_cleanup_report(&report, &paths);
        assert!(msg.contains("已清理 3"), "总数：{msg}");
        assert!(msg.contains("项目键 2 条"), "项目键分组：{msg}");
        assert!(
            msg.contains("ds×1、hq×1"),
            "按环境计数（BTreeMap 排序）：{msg}"
        );
        assert!(msg.contains("项目登记表 1 条"), "登记表分组：{msg}");
        assert!(msg.contains("备份"), "备份提示：{msg}");
        assert!(msg.contains("跳过 1 条"), "跳过计数：{msg}");
        assert!(
            msg.contains("E:/a-file（不是目录：E:/a-file）"),
            "跳过明细：{msg}"
        );
        assert!(msg.contains("写入失败"), "写失败标题：{msg}");
        assert!(msg.contains("f3.json：boom"), "写失败明细：{msg}");

        let empty = CleanupReport {
            removed: vec![],
            blocked: vec![],
            write_errors: vec![],
        };
        assert_eq!(format_cleanup_report(&empty, &paths), "没有可清理的死条目");
    }

    /// P3 回归：命令必须返回结构化结果，前端据此区分成功/部分失败，
    /// 不得把「写失败但未中断」伪装成绿色成功。用生产映射 [cleanup_result]，
    /// 不内联抄一份，避免改语义时两边分叉。
    #[test]
    fn cleanup_result_exposes_counts_and_completeness() {
        let (paths, _t) = setup();
        let report = CleanupReport {
            removed: vec![storage::DeadEntry::RegistryProject {
                raw_path: "E:/gone".into(),
            }],
            blocked: vec![],
            write_errors: vec![("f.json".to_string(), "boom".to_string())],
        };
        let result = cleanup_result(&report, &paths);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.write_error_count, 1);
        assert!(!result.complete, "有写失败时 complete 必须为 false");
        assert!(result.message.contains("写入失败"));

        let clean = CleanupReport {
            removed: vec![],
            blocked: vec![],
            write_errors: vec![],
        };
        assert!(cleanup_result(&clean, &paths).complete);
    }

    /// 并发冲突（scan 与写盘之间文件被外部改写）只进 blocked、不改 write_errors：
    /// 若 complete 仍按「无写失败」判定，用户会看到灰色"没有清理任何条目"，
    /// 掩盖了"重试即可清掉"这一事实。回归锁定它必须为未完成且可重试。
    #[test]
    fn cleanup_result_treats_concurrent_conflict_as_incomplete() {
        let (paths, _t) = setup();
        let report = CleanupReport {
            removed: vec![],
            blocked: vec![storage::BlockedEntry::retryable(
                "/gone",
                "文件在清理期间被外部修改，本轮未清理，请重试",
            )],
            write_errors: vec![],
        };
        let result = cleanup_result(&report, &paths);
        assert_eq!(result.retryable_count, 1);
        assert_eq!(result.blocked_count, 1);
        assert_eq!(result.write_error_count, 0);
        assert!(
            !result.complete,
            "留下可重试的冲突时 complete 必须为 false，否则前端会误报成功"
        );
        assert!(
            result.message.contains("1 条可在稍后重试"),
            "消息应点名还有可重试的条目：{}",
            result.message
        );
    }

    /// 稳定跳过（目录已重建 / 权限不足）重试无意义，不得被当成"未完成"反复催用户重试。
    #[test]
    fn cleanup_result_keeps_stable_skips_complete() {
        let (paths, _t) = setup();
        let report = CleanupReport {
            removed: vec![],
            blocked: vec![storage::BlockedEntry::stable(
                "/back",
                "目录现已存在，自动跳过",
            )],
            write_errors: vec![],
        };
        let result = cleanup_result(&report, &paths);
        assert_eq!(result.retryable_count, 0);
        assert_eq!(result.blocked_count, 1);
        assert!(result.complete, "稳定跳过不算未完成");
    }

    #[test]
    fn parses_real_mcp_health_states_and_strips_terminal_colors() {
        let output = concat!(
            "Checking MCP server health...\n",
            "\u{1b}[32mcodegraph: node server.js - ✓ Connected\u{1b}[0m\n",
            "realagent-gx-test: python main.py - ✗ Failed to connect\n",
            "needs-approval: https://example.test - Pending approval\n"
        );
        let names = vec![
            "codegraph".into(),
            "realagent-gx-test".into(),
            "needs-approval".into(),
            "missing".into(),
        ];
        let checks = parse_mcp_health_output("ds", &names, output);
        assert_eq!(checks[0].status, McpConnectionState::Connected);
        assert_eq!(checks[1].status, McpConnectionState::Failed);
        assert_eq!(checks[2].status, McpConnectionState::Pending);
        assert_eq!(checks[3].status, McpConnectionState::Unknown);
        assert_eq!(checks[0].environment, "ds");
        assert!(!checks[0].detail.contains('\u{1b}'));
        assert!(
            !checks[0].detail.contains("server.js"),
            "不得把完整命令发给前端"
        );
    }

    #[test]
    fn health_parser_matches_complete_server_name_only() {
        let names = vec!["agent".into(), "agent-pro".into()];
        let checks = parse_mcp_health_output(
            "hq",
            &names,
            "agent-pro: command - ✓ Connected\nagent: command - ✗ Failed",
        );
        assert_eq!(checks[0].status, McpConnectionState::Failed);
        assert_eq!(checks[1].status, McpConnectionState::Connected);
    }

    #[test]
    fn health_parser_explains_distinct_failure_stages_without_exposing_commands() {
        let cases = [
            ("spawn ENOENT", "找不到启动程序"),
            ("connection timed out", "连接超时"),
            ("process exited with exit code 1", "服务进程启动后退出"),
            ("Failed to connect", "MCP 握手失败"),
        ];
        for (raw, expected) in cases {
            let checks = parse_mcp_health_output(
                "hq",
                &["codegraph".into()],
                &format!("codegraph: secret-command --token abc - ✗ {raw}"),
            );
            assert!(checks[0].detail.contains(expected), "{}", checks[0].detail);
            assert!(!checks[0].detail.contains("secret-command"));
            assert!(!checks[0].detail.contains("abc"));
        }
    }

    #[test]
    fn shared_scope_preview_never_exposes_main_as_an_environment() {
        let environments = vec!["hq".into(), "ds".into()];
        assert_eq!(
            instances_for(&user_locator("demo"), &environments),
            environments
        );
    }
}
