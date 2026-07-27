// 作用域发现、路径解析、revision、备份、原子写入、停用仓库、list/apply 核心逻辑。
// 所有文件 IO 集中在此；mod.rs 只做命令封装、锁与跨实例同步编排。
// 测试通过 McpPaths::for_test(root) 注入独立 home，绝不读写真实 home。
//
// 关键不变量：
// - apply 阶段每个物理文件只读一次、在同一内存文档上依次应用全部 mutation、最后每文件一个 PlannedWrite；
// - 文件不存在才允许创建空对象；存在但读取/JSON 错误/顶层非对象时拒绝覆盖；
// - preview 与 apply 共用同一套硬校验，apply 不信任 preview 已调用。

use super::*;
use crate::home;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::validation;

pub(crate) const MAIN_INSTANCE: &str = "__main__";

// 全局单调计数，保证同进程同纳秒也能生成唯一临时/备份名。
static UNIQ_COUNTER: AtomicU64 = AtomicU64::new(0);

fn uniq_token() -> String {
    let n = UNIQ_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{nanos}-{n}")
}

// ---------------- 路径模型 ----------------

pub(crate) struct McpPaths {
    pub home: PathBuf,
    pub manager_dir: PathBuf,
}

impl McpPaths {
    pub fn system() -> Self {
        Self {
            home: home(),
            manager_dir: crate::cfg_dir(),
        }
    }

    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self {
        let manager = root.join(".cc-manager");
        let _ = fs::create_dir_all(&manager);
        Self {
            home: root,
            manager_dir: manager,
        }
    }

    pub fn main_claude_json(&self) -> PathBuf {
        self.home.join(".claude.json")
    }

    pub fn instance_claude_json(&self, instance_id: &str) -> Result<PathBuf, String> {
        if !safe_instance_id(instance_id) {
            return Err(format!("非法实例标识「{instance_id}」"));
        }
        if instance_id == MAIN_INSTANCE {
            Ok(self.main_claude_json())
        } else {
            Ok(self
                .home
                .join(".claude-split")
                .join(instance_id)
                .join(".claude")
                .join(".claude.json"))
        }
    }

    pub fn project_mcp_json(&self, project_path: &str) -> Result<PathBuf, String> {
        let canon = canonicalize_dir(project_path)?;
        Ok(canon.join(".mcp.json"))
    }

    pub fn project_local_settings(&self, project_path: &str) -> Result<PathBuf, String> {
        let canon = canonicalize_dir(project_path)?;
        Ok(canon.join(".claude").join("settings.local.json"))
    }

    pub fn disabled_store(&self) -> PathBuf {
        self.manager_dir.join("mcp-disabled.json")
    }

    pub fn project_registry(&self) -> PathBuf {
        self.manager_dir.join("mcp-projects.json")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.manager_dir.join("mcp-backups")
    }

    pub fn openai_sync_registry(&self) -> PathBuf {
        self.manager_dir.join("mcp-openai-sync.json")
    }

    pub fn global_codex_config(&self) -> PathBuf {
        self.home.join(".codex").join("config.toml")
    }
}

fn safe_instance_id(id: &str) -> bool {
    if id.is_empty() || id == "." || id == ".." {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// canonicalize 目录：必须存在且是目录；去掉 Windows \\?\ 前缀以稳定 key。
pub(crate) fn canonicalize_dir(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains("..") {
        return Err("项目路径非法".into());
    }
    let p = PathBuf::from(path);
    let canon = fs::canonicalize(&p).map_err(|_| format!("目录不存在或不可访问：{path}"))?;
    if !canon.is_dir() {
        return Err(format!("不是目录：{path}"));
    }
    Ok(strip_verbatim(&canon))
}

fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
}

/// 路径归一化用于等价比较：Windows 大小写不敏感、统一分隔符、去前缀。
fn norm_path(s: &str) -> String {
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    let s = s.replace('/', std::path::MAIN_SEPARATOR_STR);
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

// ---------------- 文档读取 ----------------

enum DocRead {
    Value(Value),
    Missing,
    Failed(String),
}

fn read_doc(path: &Path) -> DocRead {
    if !path.exists() {
        return DocRead::Missing;
    }
    match fs::read_to_string(path) {
        Err(e) => DocRead::Failed(format!("读取失败：{e}")),
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => DocRead::Value(v),
            Err(e) => DocRead::Failed(format!("JSON 解析失败：{e}")),
        },
    }
}

// ---------------- revision ----------------

pub(crate) fn revision(path: &Path) -> String {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return "missing".to_string(),
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let len = meta.len();
    let bytes = fs::read(path).unwrap_or_default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{}:{}:{}", mtime, len, h.finish())
}

// ---------------- sourceId 与文件映射 ----------------

pub(crate) mod source_id {
    pub fn user(instance_id: &str) -> String {
        format!("user:{instance_id}")
    }
    pub fn local(instance_id: &str, project: &str) -> String {
        format!("local:{instance_id}:{project}")
    }
    pub fn project(project: &str) -> String {
        format!("project:{project}")
    }
    pub fn project_settings(project: &str) -> String {
        format!("project-settings:{project}")
    }
    pub const DISABLED: &str = "manager:disabled";
    pub const PROJECTS: &str = "manager:projects";
}

pub(crate) fn source_file(
    paths: &McpPaths,
    instances: &[String],
    sid: &str,
) -> Result<PathBuf, String> {
    if sid == source_id::DISABLED {
        return Ok(paths.disabled_store());
    }
    if sid == source_id::PROJECTS {
        return Ok(paths.project_registry());
    }
    if let Some(inst) = sid.strip_prefix("user:") {
        if inst != MAIN_INSTANCE && !instances.iter().any(|x| x == inst) {
            return Err(format!("未知实例：{inst}"));
        }
        return paths.instance_claude_json(inst);
    }
    if let Some(rest) = sid.strip_prefix("local:") {
        let (inst, _project) = split_two(rest)?;
        if inst != MAIN_INSTANCE && !instances.iter().any(|x| x == &inst) {
            return Err(format!("未知实例：{inst}"));
        }
        paths.instance_claude_json(&inst)?;
        return paths.instance_claude_json(&inst);
    }
    if let Some(rest) = sid.strip_prefix("project:") {
        return paths.project_mcp_json(rest);
    }
    if let Some(rest) = sid.strip_prefix("project-settings:") {
        return paths.project_local_settings(rest);
    }
    Err(format!("无法识别的 sourceId：{sid}"))
}

fn split_two(s: &str) -> Result<(String, String), String> {
    let mut it = s.splitn(2, ':');
    let a = it.next().ok_or("格式错误")?;
    let b = it.next().ok_or("格式错误")?;
    Ok((a.to_string(), b.to_string()))
}

// ---------------- 项目注册表 ----------------

#[derive(Serialize, Deserialize, Default, Clone)]
struct ProjectRegistry {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    projects: Vec<String>,
}

/// 返回 (注册表, 解析问题)。文件不存在 → 默认空；存在但解析失败 → 默认空 + issue。
fn read_registry(paths: &McpPaths) -> (ProjectRegistry, Option<McpSourceIssue>) {
    match read_doc(&paths.project_registry()) {
        DocRead::Value(v) => {
            // version 必须为 1；缺失或非 1 均拒绝
            match v.get("version").and_then(|x| x.as_u64()) {
                Some(1) => {}
                _ => {
                    return (
                        ProjectRegistry::default(),
                        Some(McpSourceIssue {
                            source_id: source_id::PROJECTS.to_string(),
                            path: paths.project_registry().display().to_string(),
                            detail: "mcp-projects.json version 必须为 1".into(),
                        }),
                    );
                }
            }
            match serde_json::from_value::<ProjectRegistry>(v) {
                Ok(r) => (r, None),
                Err(e) => (
                    ProjectRegistry::default(),
                    Some(McpSourceIssue {
                        source_id: source_id::PROJECTS.to_string(),
                        path: paths.project_registry().display().to_string(),
                        detail: format!("mcp-projects.json 解析失败：{e}"),
                    }),
                ),
            }
        }
        DocRead::Missing => (ProjectRegistry::default(), None),
        DocRead::Failed(e) => (
            ProjectRegistry::default(),
            Some(McpSourceIssue {
                source_id: source_id::PROJECTS.to_string(),
                path: paths.project_registry().display().to_string(),
                detail: e,
            }),
        ),
    }
}

fn write_registry(paths: &McpPaths, mut reg: ProjectRegistry) -> Result<(), String> {
    reg.version = 1;
    reg.projects.sort_by_key(|a| norm_path(a));
    reg.projects.dedup_by(|a, b| norm_path(a) == norm_path(b));
    write_json_transactional(
        paths,
        &paths.project_registry(),
        &serde_json::to_value(&reg).map_err(|e| e.to_string())?,
    )
}

pub(crate) fn register_project(paths: &McpPaths, path: &str) -> Result<(), String> {
    let canon = canonicalize_dir(path)?;
    let (reg, issue) = read_registry(paths);
    if let Some(i) = issue {
        return Err(i.detail);
    }
    let mut reg = reg;
    let canon_s = canon.display().to_string();
    if !reg
        .projects
        .iter()
        .any(|p| norm_path(p) == norm_path(&canon_s))
    {
        reg.projects.push(canon_s);
    }
    write_registry(paths, reg)
}

pub(crate) fn unregister_project(paths: &McpPaths, path: &str) -> Result<(), String> {
    let canon = canonicalize_dir(path)?;
    let (reg, issue) = read_registry(paths);
    if let Some(i) = issue {
        return Err(i.detail);
    }
    let mut reg = reg;
    let canon_s = canon.display().to_string();
    reg.projects.retain(|p| norm_path(p) != norm_path(&canon_s));
    write_registry(paths, reg)
}

// ---------------- 停用仓库 ----------------

#[derive(Serialize, Deserialize, Default, Clone)]
struct DisabledStore {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: Vec<DisabledEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DisabledEntry {
    scope: McpScope,
    name: String,
    instance_id: Option<String>,
    project_path: Option<String>,
    config: Map<String, Value>,
    disabled_at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn validate_disabled_entries(entries: &[DisabledEntry]) -> Result<(), String> {
    for entry in entries {
        match entry.scope {
            McpScope::User => {
                if entry.instance_id.is_some() || entry.project_path.is_some() {
                    return Err("mcp-disabled.json 的 User 条目不能包含实例或项目".into());
                }
            }
            McpScope::Local => {
                if entry.instance_id.as_deref().is_none_or(str::is_empty)
                    || entry.project_path.as_deref().is_none_or(str::is_empty)
                {
                    return Err("mcp-disabled.json 的 Local 条目必须包含实例和项目".into());
                }
            }
            McpScope::Project => {
                return Err(
                    "mcp-disabled.json 只允许 User/Local；Project 必须使用 disabledMcpjsonServers"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

fn read_disabled(paths: &McpPaths) -> (DisabledStore, Option<McpSourceIssue>) {
    match read_doc(&paths.disabled_store()) {
        DocRead::Value(v) => {
            // version 必须为 1；缺失或非 1 均拒绝，不存在需要兼容的无版本历史格式
            match v.get("version").and_then(|x| x.as_u64()) {
                Some(1) => {}
                _ => {
                    return (
                        DisabledStore::default(),
                        Some(McpSourceIssue {
                            source_id: source_id::DISABLED.to_string(),
                            path: paths.disabled_store().display().to_string(),
                            detail: "mcp-disabled.json version 必须为 1".into(),
                        }),
                    );
                }
            }
            match serde_json::from_value::<DisabledStore>(v) {
                Ok(s) => match validate_disabled_entries(&s.entries) {
                    Ok(()) => (s, None),
                    Err(detail) => (
                        DisabledStore::default(),
                        Some(McpSourceIssue {
                            source_id: source_id::DISABLED.to_string(),
                            path: paths.disabled_store().display().to_string(),
                            detail,
                        }),
                    ),
                },
                Err(e) => (
                    DisabledStore::default(),
                    Some(McpSourceIssue {
                        source_id: source_id::DISABLED.to_string(),
                        path: paths.disabled_store().display().to_string(),
                        detail: format!("mcp-disabled.json 解析失败：{e}"),
                    }),
                ),
            }
        }
        DocRead::Missing => (DisabledStore::default(), None),
        DocRead::Failed(e) => (
            DisabledStore::default(),
            Some(McpSourceIssue {
                source_id: source_id::DISABLED.to_string(),
                path: paths.disabled_store().display().to_string(),
                detail: e,
            }),
        ),
    }
}

fn disabled_matches(e: &DisabledEntry, locator: &McpLocator) -> bool {
    e.scope == locator.scope
        && e.name == locator.name
        && e.instance_id.as_deref() == locator.instance_id.as_deref()
        && norm_opt(e.project_path.as_deref()) == norm_opt(locator.project_path.as_deref())
}

fn norm_opt(s: Option<&str>) -> String {
    s.map(norm_path).unwrap_or_default()
}

// ---------------- 备份与原子写入 ----------------

fn source_hash(target: &Path) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    target.display().to_string().hash(&mut h);
    format!("{:x}", h.finish())
}

fn backup_path(paths: &McpPaths, target: &Path) -> PathBuf {
    let dir = paths.backup_dir().join(source_hash(target));
    let _ = fs::create_dir_all(&dir);
    dir.join(format!("{}.json", uniq_token()))
}

/// 轮换：每个 source-hash 只保留最近 5 份备份（按文件名内嵌时间戳排序）。
fn rotate_backups(dir: &Path) {
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(_) => return,
    };
    if entries.len() <= 5 {
        return;
    }
    entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let to_remove = entries.len().saturating_sub(5);
    for p in entries.into_iter().take(to_remove) {
        let _ = fs::remove_file(p);
    }
}

/// 清理同 source 遗留临时文件；rollback 仅在 target 存在时清理（target 缺失时的 rollback
/// 由 write_json_transactional 先恢复再终止，不能在此删）。
fn clean_leftover(_paths: &McpPaths, target: &Path) {
    let dir = match target.parent() {
        Some(d) => d,
        None => return,
    };
    let hash = source_hash(target);
    let tmp_prefix = format!(".ccm-tmp-{hash}-");
    let rollback_name = format!(".ccm-rollback-{hash}");
    let target_exists = target.exists();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&tmp_prefix) || (target_exists && name == rollback_name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// 原子写入：唯一临时文件 → 备份 → rollback rename → target rename → 删 rollback。
/// 备份统一落盘到 ~/.cc-manager（不污染项目目录 / Git）。关键步骤失败必须返回错误。
/// 若 target 不存在但 rollback 存在（上次 target→rollback 后中断），先恢复 rollback 并终止。
pub(crate) fn write_json_transactional(
    paths: &McpPaths,
    target: &Path,
    value: &Value,
) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let rollback = target.with_file_name(format!(".ccm-rollback-{}", source_hash(target)));
    if !target.exists() && rollback.exists() {
        // 上次在 target→rollback 后、tmp→target 前中断：恢复 rollback 为 target，终止本次操作。
        // 恢复 rename 必须检查结果，失败时报告恢复失败而非假装"已恢复"。
        return match fs::rename(&rollback, target) {
            Ok(()) => Err(format!(
                "{}：检测到上次中断留下的 rollback，已恢复原配置，请刷新后重试",
                target.display()
            )),
            Err(e) => Err(format!(
                "{}：检测到上次中断留下的 rollback 且恢复失败（{e}），请手动检查",
                target.display()
            )),
        };
    }
    clean_leftover(paths, target);

    let text = serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败：{e}"))?;
    let tmp = target.with_file_name(format!(".ccm-tmp-{}-{}", source_hash(target), uniq_token()));
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败：{e}"))?;
        f.write_all(text.as_bytes())
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
        f.sync_all().map_err(|e| format!("同步临时文件失败：{e}"))?;
    }

    if target.exists() {
        let bak = backup_path(paths, target);
        fs::copy(target, &bak).map_err(|e| format!("备份失败：{e}"))?;
        if let Some(dir) = bak.parent() {
            rotate_backups(dir);
        }
    }

    if target.exists() {
        let _ = fs::remove_file(&rollback);
        fs::rename(target, &rollback).map_err(|e| format!("备份原文件失败：{e}"))?;
    }
    match fs::rename(&tmp, target) {
        Ok(()) => {
            let _ = fs::remove_file(&rollback);
            Ok(())
        }
        Err(e) => {
            if rollback.exists() {
                if let Err(re) = fs::rename(&rollback, target) {
                    return Err(format!("提升临时文件失败：{e}；且 rollback 恢复失败：{re}"));
                }
            }
            Err(format!("提升临时文件失败：{e}"))
        }
    }
}

pub(crate) struct PlannedWrite {
    pub path: PathBuf,
    pub value: Value,
}
struct Snap {
    path: PathBuf,
    existed: bool,
    bytes: Option<Vec<u8>>,
    revision: String,
}

/// 受影响文件的事务：按给定顺序写入，任一失败按相反顺序恢复（含当前文件）。
/// writes 已由上层保证每个物理文件只出现一次。恢复失败不忽略，并入错误信息。
/// snapshot 创建时文件存在但读取失败（权限/I/O 错误）→ 在任何写入发生前立即终止；仅有 NotFound 表 existed=false。
#[cfg(test)]
pub(crate) fn apply_storage_transaction(
    paths: &McpPaths,
    writes: Vec<PlannedWrite>,
) -> Result<(), String> {
    apply_storage_transaction_inner(paths, writes, None)
}

fn apply_storage_transaction_inner(
    paths: &McpPaths,
    writes: Vec<PlannedWrite>,
    expected_by_path: Option<&BTreeMap<PathBuf, Vec<(String, String)>>>,
) -> Result<(), String> {
    // 先复核全部 affected source（包含本次只读但决定状态迁移的来源），避免停用仓库
    // 与直接配置在 preview 后发生竞态而生成重复定义。
    if let Some(expected_by_path) = expected_by_path {
        for (path, expected) in expected_by_path {
            let current_revision = revision(path);
            for (source_id, value) in expected {
                if value != &current_revision {
                    return Err(format!("配置已被外部修改（{source_id}），请刷新后重试"));
                }
            }
        }
    }
    let mut snaps = Vec::with_capacity(writes.len());
    for w in &writes {
        let (existed, bytes) = match fs::read(&w.path) {
            Ok(bytes) => (true, Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, None),
            Err(e) => {
                return Err(format!("读取事务快照失败：{e}"));
            }
        };
        let current_revision = revision(&w.path);
        if let Some(expected_by_path) = expected_by_path {
            let expected = expected_by_path
                .get(&w.path)
                .ok_or_else(|| format!("写入来源未出现在 preview 中：{}", w.path.display()))?;
            for (source_id, value) in expected {
                if value != &current_revision {
                    return Err(format!("配置已被外部修改（{source_id}），请刷新后重试"));
                }
            }
        }
        snaps.push(Snap {
            path: w.path.clone(),
            existed,
            bytes,
            revision: current_revision,
        });
    }

    for (idx, w) in writes.iter().enumerate() {
        // Workbench 构建和快照完成后仍可能有外部编辑；每个文件替换前再检查一次。
        if revision(&w.path) != snaps[idx].revision {
            let restore_errs = rollback(snaps.iter().take(idx));
            let mut msg = format!(
                "配置在事务提交前被外部修改（{}），请刷新后重试",
                w.path.display()
            );
            if !restore_errs.is_empty() {
                msg.push_str(&format!("；回滚亦失败：{}", restore_errs.join("；")));
            }
            return Err(msg);
        }
        if let Some(parent) = w.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = write_json_transactional(paths, &w.path, &w.value) {
            // 只回滚"已经尝试过"的 snapshot 前缀（含当前文件），不动尚未处理的文件。
            // write_json_transactional 可能已部分恢复当前文件，这里再用原 bytes 覆盖确保一致。
            let restore_errs = rollback(snaps.iter().take(idx + 1));
            let mut msg = format!("写入 {} 失败：{e}", w.path.display());
            if !restore_errs.is_empty() {
                msg.push_str(&format!("；回滚亦失败：{}", restore_errs.join("；")));
            }
            return Err(msg);
        }
    }
    Ok(())
}

fn apply_storage_transaction_ordered(
    paths: &McpPaths,
    writes: Vec<PlannedWrite>,
    source_ids_by_path: &BTreeMap<PathBuf, Vec<String>>,
    expected: Option<&BTreeMap<String, String>>,
) -> Result<(), String> {
    let writes = sort_writes_by_source_id(writes, source_ids_by_path);
    let expected_by_path = expected
        .map(|expected| {
            let mut out: BTreeMap<PathBuf, Vec<(String, String)>> = BTreeMap::new();
            for (path, source_ids) in source_ids_by_path {
                let mut values = Vec::new();
                for source_id in source_ids {
                    let value = expected.get(source_id).ok_or_else(|| {
                        format!("revision 集合缺少 {source_id}（请刷新页面后重试）")
                    })?;
                    values.push((source_id.clone(), value.clone()));
                }
                out.insert(path.clone(), values);
            }
            Ok::<_, String>(out)
        })
        .transpose()?;
    apply_storage_transaction_inner(paths, writes, expected_by_path.as_ref())
}

fn sort_writes_by_source_id(
    mut writes: Vec<PlannedWrite>,
    source_ids_by_path: &BTreeMap<PathBuf, Vec<String>>,
) -> Vec<PlannedWrite> {
    writes.sort_by(|a, b| {
        let a_id = source_ids_by_path
            .get(&a.path)
            .and_then(|ids| ids.first())
            .map(String::as_str)
            .unwrap_or("");
        let b_id = source_ids_by_path
            .get(&b.path)
            .and_then(|ids| ids.first())
            .map(String::as_str)
            .unwrap_or("");
        a_id.cmp(b_id).then_with(|| a.path.cmp(&b.path))
    });
    writes
}

/// 按相反顺序恢复；返回恢复失败的明细（不静默吞掉）。
fn rollback<'a, I>(snaps: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a Snap>,
{
    let mut vec: Vec<&Snap> = snaps.into_iter().collect();
    vec.reverse();
    let mut errs = Vec::new();
    for s in vec {
        if s.existed {
            if let Some(bytes) = &s.bytes {
                if let Err(e) = fs::write(&s.path, bytes) {
                    errs.push(format!("恢复 {} 失败：{e}", s.path.display()));
                }
            }
        } else if let Err(e) = fs::remove_file(&s.path) {
            // 原本不存在且未被写入的文件，回滚删除时 NotFound 属正常，忽略；其他错误上报。
            if e.kind() != std::io::ErrorKind::NotFound {
                errs.push(format!("删除 {} 失败：{e}", s.path.display()));
            }
        }
    }
    errs
}

// ---------------- 文档内 mutation 工具 ----------------

fn read_user_map(doc: &Value) -> Result<Map<String, Value>, String> {
    match doc.get("mcpServers") {
        None => Ok(Map::new()),
        Some(v) if v.is_object() => Ok(v.as_object().cloned().unwrap_or_default()),
        Some(_) => Err("mcpServers 不是对象".into()),
    }
}

fn find_local_project_key(doc: &Value, canonical: &str) -> Result<Option<String>, String> {
    let projs = match doc.get("projects") {
        None => return Ok(None),
        Some(v) if v.is_object() => v.as_object().unwrap(),
        Some(_) => return Err("projects 不是对象".into()),
    };
    let ncanon = norm_path(canonical);
    for k in projs.keys() {
        if norm_path(k) == ncanon {
            return Ok(Some(k.clone()));
        }
    }
    for k in projs.keys() {
        if let Ok(existing_canon) = canonicalize_dir(k) {
            if norm_path(&existing_canon.display().to_string()) == ncanon {
                return Ok(Some(k.clone()));
            }
        }
    }
    Ok(None)
}

fn read_local_map(doc: &Value, canonical: &str) -> Result<Map<String, Value>, String> {
    let projs_val = match doc.get("projects") {
        None => return Ok(Map::new()),
        Some(v) if v.is_object() => v.as_object().unwrap(),
        Some(_) => return Err("projects 不是对象".into()),
    };
    let key = match find_local_project_key(doc, canonical)? {
        Some(k) => k,
        None => return Ok(Map::new()),
    };
    let node = projs_val.get(&key).unwrap();
    if !node.is_object() {
        return Err("项目节点不是对象".into());
    }
    match node.get("mcpServers") {
        None => Ok(Map::new()),
        Some(v) if v.is_object() => Ok(v.as_object().cloned().unwrap_or_default()),
        Some(_) => Err("mcpServers 不是对象".into()),
    }
}

fn as_object_mut(v: &mut Value) -> Option<&mut Map<String, Value>> {
    v.as_object_mut()
}

fn set_user_map(doc: &mut Value, new_map: Map<String, Value>) {
    if let Some(obj) = as_object_mut(doc) {
        obj.insert("mcpServers".into(), Value::Object(new_map));
    }
}

/// 写入/删除 local：沿用已有等价 key，没有则用 canonical 作新 key。
/// 字段缺失可创建默认；存在但类型错误返回 Err，不静默跳过。
fn mutate_local(
    doc: &mut Value,
    canonical: &str,
    mutate: &dyn Fn(&mut Map<String, Value>),
) -> Result<(), String> {
    let key = find_local_project_key(doc, canonical)?.unwrap_or_else(|| canonical.to_string());
    let obj = as_object_mut(doc).ok_or("文档顶层不是对象")?;
    let projects = obj
        .entry("projects".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !projects.is_object() {
        return Err("projects 不是对象".into());
    }
    let projects_obj = projects.as_object_mut().unwrap();
    let node = projects_obj
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    if !node.is_object() {
        return Err("项目节点不是对象".into());
    }
    let node_obj = node.as_object_mut().unwrap();
    let servers = node_obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        return Err("mcpServers 不是对象".into());
    }
    let servers_obj = servers.as_object_mut().unwrap();
    mutate(servers_obj);
    Ok(())
}

fn read_project_mcp_map(doc: &Value) -> Result<Map<String, Value>, String> {
    match doc.get("mcpServers") {
        None => Ok(Map::new()),
        Some(v) if v.is_object() => Ok(v.as_object().cloned().unwrap_or_default()),
        Some(_) => Err("mcpServers 不是对象".into()),
    }
}

fn set_project_mcp_map(doc: &mut Value, new_map: Map<String, Value>) {
    if let Some(obj) = as_object_mut(doc) {
        obj.insert("mcpServers".into(), Value::Object(new_map));
    }
}

// ---------------- 实例集合 ----------------

pub(crate) fn all_instances(profile_names: &[String]) -> Vec<String> {
    let mut v = vec![MAIN_INSTANCE.to_string()];
    for n in profile_names {
        if !n.is_empty() && !v.contains(n) {
            v.push(n.clone());
        }
    }
    v
}

/// 已登记 + 已发现的项目 canonical 集合（用于 locator 校验）。
fn known_projects(paths: &McpPaths, profile_names: &[String]) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    let (reg, _) = read_registry(paths);
    for p in &reg.projects {
        if let Ok(c) = canonicalize_dir(p) {
            set.insert(c.display().to_string());
        }
    }
    for inst in &all_instances(profile_names) {
        if let Ok(p) = paths.instance_claude_json(inst) {
            if let DocRead::Value(doc) = read_doc(&p) {
                if let Some(projs) = doc.get("projects").and_then(|v| v.as_object()) {
                    for k in projs.keys() {
                        if let Ok(c) = canonicalize_dir(k) {
                            set.insert(c.display().to_string());
                        }
                    }
                }
            }
        }
    }
    set
}

// ---------------- 硬校验（preview/apply 共用） ----------------

/// 对动作做后端硬校验；返回 Err 则 preview 与 apply 都拒绝。apply 不信任 preview 已调用。
pub(crate) fn validate_action_strict(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
) -> Result<(), String> {
    let known = known_projects(paths, instances);
    let inst_set: BTreeSet<String> = all_instances(instances).iter().cloned().collect();
    let check_locator = |loc: &McpLocator| -> Result<(), String> {
        match loc.scope {
            McpScope::User => {
                if loc.instance_id.is_some() {
                    return Err("用户级不能指定实例".into());
                }
                if loc.project_path.is_some() {
                    return Err("用户级不能指定项目".into());
                }
            }
            McpScope::Local => {
                let inst = loc.instance_id.as_deref().ok_or("项目本地必须选择实例")?;
                if !inst_set.contains(inst) {
                    return Err(format!("未知实例：{inst}"));
                }
                let proj = loc.project_path.as_deref().ok_or("项目本地必须选择项目")?;
                let canon = canonicalize_dir(proj)?;
                if !known.contains(&canon.display().to_string()) {
                    return Err("项目未登记或未发现，请先用「添加项目」登记".into());
                }
            }
            McpScope::Project => {
                if loc.instance_id.is_some() {
                    return Err("项目共享不能指定实例".into());
                }
                let proj = loc.project_path.as_deref().ok_or("项目共享必须选择项目")?;
                let canon = canonicalize_dir(proj)?;
                if !known.contains(&canon.display().to_string()) {
                    return Err("项目未登记或未发现，请先用「添加项目」登记".into());
                }
            }
        }
        Ok(())
    };
    let check_config = |name: &str, cfg: &Map<String, Value>| -> Result<(), String> {
        validation::validate_config_strict(cfg).map_err(|e| format!("服务「{name}」：{e}"))
    };
    match action {
        McpChangeAction::Save {
            original,
            target,
            config,
            ..
        } => {
            check_locator(target)?;
            // 有 original 时必须先校验 original locator 本身合法，并主动读取 original 来源：
            // 来源损坏/不可读/结构非法时直接拒绝，保证不经过 preview 的直接 apply 也会被拦。
            if let Some(orig) = original.as_ref() {
                check_locator(orig)?;
                read_locator_config(paths, instances, orig)?;
            }
            // 旧非法名 grandfather：仅当 original 确实存在（启用来源或停用仓库）且名称未改变时才允许；
            // 伪造的不存在 original 必须按新名校验并拒绝非法名。
            let name_unchanged = original
                .as_ref()
                .map(|o| o.name == target.name)
                .unwrap_or(false);
            if name_unchanged {
                let orig = original.as_ref().unwrap();
                let exists = read_service_config(paths, instances, orig)?.is_some()
                    || is_in_disabled_store(paths, orig)?;
                if !exists {
                    validation::validate_name(&target.name).map_err(|e| e.to_string())?;
                }
            } else {
                validation::validate_name(&target.name).map_err(|e| e.to_string())?;
            }
            check_config(&target.name, config)?;
        }
        McpChangeAction::BatchSave { items } => {
            if items.is_empty() || items.len() > 100 {
                return Err("批量保存需 1-100 项".into());
            }
            let mut seen = BTreeSet::new();
            for it in items {
                if !seen.insert(locator_key_str(&it.target)) {
                    return Err("批量保存存在重复目标".into());
                }
                check_locator(&it.target)?;
                validation::validate_name(&it.target.name).map_err(|e| e.to_string())?;
                check_config(&it.target.name, &it.config)?;
            }
        }
        McpChangeAction::SetEnabled { target, .. } | McpChangeAction::Delete { target } => {
            check_locator(target)?;
        }
    }
    Ok(())
}

fn locator_key_str(l: &McpLocator) -> String {
    format!(
        "{:?}|{}|{}|{}",
        l.scope,
        l.name,
        l.instance_id.clone().unwrap_or_default(),
        l.project_path.clone().unwrap_or_default()
    )
}

// ---------------- 工作台：单文件单文档，依次 mutation ----------------

struct WorkDoc {
    value: Value,
    dirty: bool,
    existed: bool, // 文件在磁盘上是否存在（刚创建的空文档 = false）
}

struct Workbench<'a> {
    paths: &'a McpPaths,
    order: Vec<PathBuf>,
    docs: BTreeMap<PathBuf, WorkDoc>,
}

impl<'a> Workbench<'a> {
    fn new(paths: &'a McpPaths) -> Self {
        Self {
            paths,
            order: Vec::new(),
            docs: BTreeMap::new(),
        }
    }

    /// 载入文件到工作台。不存在 → 空对象；存在但解析失败/非对象 → Err（拒绝覆盖）。
    fn ensure(&mut self, path: &Path) -> Result<(), String> {
        if self.docs.contains_key(path) {
            return Ok(());
        }
        let wd = match read_doc(path) {
            DocRead::Missing => WorkDoc {
                value: Value::Object(Map::new()),
                dirty: false,
                existed: false,
            },
            DocRead::Value(v) if v.is_object() => WorkDoc {
                value: v,
                dirty: false,
                existed: true,
            },
            DocRead::Value(_) => return Err(format!("{} 顶层不是对象，拒绝覆盖", path.display())),
            DocRead::Failed(e) => {
                return Err(format!("{}：{e}", path.display()));
            }
        };
        self.order.push(path.to_path_buf());
        self.docs.insert(path.to_path_buf(), wd);
        Ok(())
    }

    fn user_set(
        &mut self,
        path: &Path,
        name: &str,
        config: Map<String, Value>,
    ) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let mut map = read_user_map(&doc.value)?;
        map.insert(name.into(), Value::Object(config));
        set_user_map(&mut doc.value, map);
        doc.dirty = true;
        Ok(())
    }

    fn user_del(&mut self, path: &Path, name: &str) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let mut map = read_user_map(&doc.value)?;
        map.remove(name);
        set_user_map(&mut doc.value, map);
        doc.dirty = true;
        Ok(())
    }

    fn local_set(
        &mut self,
        path: &Path,
        canon: &str,
        name: &str,
        config: Map<String, Value>,
    ) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let n = name.to_string();
        mutate_local(&mut doc.value, canon, &|servers| {
            servers.insert(n.clone(), Value::Object(config.clone()));
        })?;
        doc.dirty = true;
        Ok(())
    }

    fn local_del(&mut self, path: &Path, canon: &str, name: &str) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let n = name.to_string();
        mutate_local(&mut doc.value, canon, &|servers| {
            servers.remove(&n);
        })?;
        doc.dirty = true;
        Ok(())
    }

    fn project_set(
        &mut self,
        path: &Path,
        name: &str,
        config: Map<String, Value>,
    ) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let mut map = read_project_mcp_map(&doc.value)?;
        map.insert(name.into(), Value::Object(config));
        set_project_mcp_map(&mut doc.value, map);
        doc.dirty = true;
        Ok(())
    }

    fn project_del(&mut self, path: &Path, name: &str) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let mut map = read_project_mcp_map(&doc.value)?;
        map.remove(name);
        set_project_mcp_map(&mut doc.value, map);
        doc.dirty = true;
        Ok(())
    }

    fn settings_disable(&mut self, path: &Path, name: &str) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let obj = match as_object_mut(&mut doc.value) {
            Some(o) => o,
            None => return Err("settings.local.json 顶层非对象".into()),
        };
        let arr_val = obj
            .entry("disabledMcpjsonServers".to_string())
            .or_insert_with(|| Value::Array(vec![]));
        if !arr_val.is_array() {
            return Err("disabledMcpjsonServers 不是数组".into());
        }
        let a = arr_val.as_array_mut().unwrap();
        if a.iter().any(|v| !v.is_string()) {
            return Err("disabledMcpjsonServers 含非字符串项".into());
        }
        if !a.iter().any(|v| v.as_str() == Some(name)) {
            a.push(Value::String(name.into()));
        }
        a.sort_by(|x, y| x.as_str().unwrap_or("").cmp(y.as_str().unwrap_or("")));
        a.dedup();
        doc.dirty = true;
        Ok(())
    }

    fn settings_enable(&mut self, path: &Path, name: &str) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        let mut changed = false;
        if let Some(arr_val) = doc.value.get_mut("disabledMcpjsonServers") {
            if !arr_val.is_array() {
                return Err("disabledMcpjsonServers 不是数组".into());
            }
            let a = arr_val.as_array_mut().unwrap();
            if a.iter().any(|v| !v.is_string()) {
                return Err("disabledMcpjsonServers 含非字符串项".into());
            }
            let before = a.len();
            a.retain(|v| v.as_str() != Some(name));
            changed = a.len() != before;
            if a.is_empty() {
                if let Some(obj) = as_object_mut(&mut doc.value) {
                    obj.remove("disabledMcpjsonServers");
                }
            }
        }
        doc.dirty |= changed;
        Ok(())
    }

    fn disabled_entries(&mut self, path: &Path) -> Result<Vec<DisabledEntry>, String> {
        self.ensure(path)?;
        let doc = self.docs.get(path).unwrap();
        // 严格反序列化：合法 JSON 但字段结构错误也必须报错，避免把原内容当空仓库覆盖。
        let store: DisabledStore = serde_json::from_value(doc.value.clone())
            .map_err(|e| format!("mcp-disabled.json 结构错误：{e}"))?;
        // 文件在磁盘上存在时才校验 version；刚创建的空文档由 disabled_replace 写入 version=1
        if doc.existed {
            match doc.value.get("version").and_then(|x| x.as_u64()) {
                Some(1) => {}
                _ => return Err("mcp-disabled.json version 必须为 1".into()),
            }
        }
        validate_disabled_entries(&store.entries)?;
        Ok(store.entries)
    }

    fn disabled_replace(&mut self, path: &Path, entries: Vec<DisabledEntry>) -> Result<(), String> {
        self.ensure(path)?;
        let doc = self.docs.get_mut(path).unwrap();
        doc.value = serde_json::to_value(&DisabledStore {
            version: 1,
            entries,
        })
        .map_err(|e| e.to_string())?;
        doc.dirty = true;
        Ok(())
    }

    /// 读取 locator 当前配置（从启用源）。源文件不可解析或字段类型错误 → Err；
    /// 命中已存在的非对象成员也返回 Err，禁止把非法成员当成“不存在”。
    fn current_config(
        &mut self,
        locator: &McpLocator,
    ) -> Result<Option<Map<String, Value>>, String> {
        match locator.scope {
            McpScope::User => {
                let p = self.paths.instance_claude_json(MAIN_INSTANCE)?;
                self.ensure(&p)?;
                let doc = self.docs.get(&p).unwrap();
                let map = read_user_map(&doc.value)?;
                member_config(&map, &locator.name)
            }
            McpScope::Local => {
                let (inst, proj) = local_parts(locator)?;
                let canon = canonicalize_dir(&proj)?;
                let p = self.paths.instance_claude_json(&inst)?;
                self.ensure(&p)?;
                let doc = self.docs.get(&p).unwrap();
                let map = read_local_map(&doc.value, &canon.display().to_string())?;
                member_config(&map, &locator.name)
            }
            McpScope::Project => {
                let proj = locator.project_path.as_deref().ok_or("缺少项目路径")?;
                let p = self.paths.project_mcp_json(proj)?;
                self.ensure(&p)?;
                let doc = self.docs.get(&p).unwrap();
                let map = read_project_mcp_map(&doc.value)?;
                member_config(&map, &locator.name)
            }
        }
    }

    /// 输出每文件一个 PlannedWrite（按载入顺序，保留 disable 先仓库后来源等语义）。
    fn emit(self) -> Vec<PlannedWrite> {
        let docs = self.docs;
        self.order
            .into_iter()
            .filter_map(|p| {
                let wd = docs.get(&p)?;
                if wd.dirty {
                    Some(PlannedWrite {
                        path: p,
                        value: wd.value.clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

/// 从 mcpServers map 取出 name 对应配置：缺失 → None；命中非对象成员 → Err，
/// 禁止把已存在的非法成员当成“不存在”。
fn member_config(
    map: &Map<String, Value>,
    name: &str,
) -> Result<Option<Map<String, Value>>, String> {
    match map.get(name) {
        None => Ok(None),
        Some(v) if v.is_object() => Ok(Some(v.as_object().cloned().unwrap_or_default())),
        Some(_) => Err(format!("mcpServers[{name}] 不是对象")),
    }
}

fn local_parts(locator: &McpLocator) -> Result<(String, String), String> {
    match (&locator.instance_id, &locator.project_path) {
        (Some(i), Some(p)) => Ok((i.clone(), p.clone())),
        _ => Err("Local locator 缺少实例或项目".into()),
    }
}

/// 读取 locator 当前配置：优先启用源，其次停用仓库（User/Local）。来源错误（解析失败等）传播上游。
pub(crate) fn read_locator_config(
    paths: &McpPaths,
    instances: &[String],
    locator: &McpLocator,
) -> Result<Option<Map<String, Value>>, String> {
    match read_service_config(paths, instances, locator) {
        Ok(Some(c)) => return Ok(Some(c)),
        Err(e) => return Err(e),
        Ok(None) => {}
    }
    if locator.scope == McpScope::Project {
        return Ok(None);
    }
    let (store, issue) = read_disabled(paths);
    if let Some(i) = issue {
        return Err(i.detail);
    }
    Ok(store
        .entries
        .iter()
        .find(|e| disabled_matches(e, locator))
        .map(|e| e.config.clone()))
}

pub(crate) fn read_service_config(
    paths: &McpPaths,
    _instances: &[String],
    locator: &McpLocator,
) -> Result<Option<Map<String, Value>>, String> {
    match locator.scope {
        McpScope::User => {
            let path = paths.instance_claude_json(MAIN_INSTANCE)?;
            let doc = match read_doc(&path) {
                DocRead::Missing => return Ok(None),
                DocRead::Failed(e) => return Err(e),
                DocRead::Value(d) if d.is_object() => d,
                DocRead::Value(_) => return Err(format!("{} 顶层不是对象", path.display())),
            };
            match read_user_map(&doc)?.get(&locator.name) {
                None => Ok(None),
                Some(v) if v.is_object() => Ok(Some(v.as_object().cloned().unwrap_or_default())),
                Some(_) => Err(format!("mcpServers[{}] 不是对象", locator.name)),
            }
        }
        McpScope::Local => {
            let (inst, proj) = local_parts(locator)?;
            let canon = canonicalize_dir(&proj)?;
            let path = paths.instance_claude_json(&inst)?;
            let doc = match read_doc(&path) {
                DocRead::Missing => return Ok(None),
                DocRead::Failed(e) => return Err(e),
                DocRead::Value(d) if d.is_object() => d,
                DocRead::Value(_) => return Err(format!("{} 顶层不是对象", path.display())),
            };
            match read_local_map(&doc, &canon.display().to_string())?.get(&locator.name) {
                None => Ok(None),
                Some(v) if v.is_object() => Ok(Some(v.as_object().cloned().unwrap_or_default())),
                Some(_) => Err(format!("mcpServers[{}] 不是对象", locator.name)),
            }
        }
        McpScope::Project => {
            let proj = locator.project_path.as_deref().ok_or("缺少项目路径")?;
            let path = paths.project_mcp_json(proj)?;
            let doc = match read_doc(&path) {
                DocRead::Missing => return Ok(None),
                DocRead::Failed(e) => return Err(e),
                DocRead::Value(d) if d.is_object() => d,
                DocRead::Value(_) => return Err(format!("{} 顶层不是对象", path.display())),
            };
            match read_project_mcp_map(&doc)?.get(&locator.name) {
                None => Ok(None),
                Some(v) if v.is_object() => Ok(Some(v.as_object().cloned().unwrap_or_default())),
                Some(_) => Err(format!("mcpServers[{}] 不是对象", locator.name)),
            }
        }
    }
}

/// locator 当前是否位于停用仓库（启用来源是否缺该服务）。
/// disabled store 损坏（读取/解析失败）时返回 Err，禁止当成空仓库继续推导受影响来源或写入启用配置。
fn is_in_disabled_store(paths: &McpPaths, locator: &McpLocator) -> Result<bool, String> {
    if locator.scope == McpScope::Project {
        return Ok(false);
    }
    let (store, issue) = read_disabled(paths);
    if let Some(i) = issue {
        return Err(i.detail);
    }
    Ok(store.entries.iter().any(|e| disabled_matches(e, locator)))
}

#[derive(Clone, Copy)]
struct LocatorPresence {
    enabled_source: bool,
    manager_disabled: bool,
    project_disabled: bool,
}

impl LocatorPresence {
    fn exists(self) -> bool {
        self.enabled_source || self.manager_disabled
    }

    fn disabled(self) -> bool {
        self.manager_disabled || self.project_disabled
    }
}

fn locator_presence(
    paths: &McpPaths,
    instances: &[String],
    locator: &McpLocator,
) -> Result<LocatorPresence, String> {
    let enabled_source = read_service_config(paths, instances, locator)?.is_some();
    let manager_disabled = is_in_disabled_store(paths, locator)?;
    if enabled_source && manager_disabled {
        return Err(format!(
            "服务「{}」同时存在于启用来源和停用仓库，请先修复重复定义",
            locator.name
        ));
    }
    let project_disabled = if locator.scope == McpScope::Project {
        let project = locator.project_path.as_deref().ok_or("缺少项目路径")?;
        let settings_path = paths.project_local_settings(project)?;
        let (names, issue) = read_disabled_project_names(&settings_path, project);
        if let Some(issue) = issue {
            return Err(issue.detail);
        }
        names.iter().any(|name| name == &locator.name)
    } else {
        false
    };
    Ok(LocatorPresence {
        enabled_source,
        manager_disabled,
        project_disabled,
    })
}

fn save_locator_disabled(
    wb: &mut Workbench<'_>,
    paths: &McpPaths,
    target: &McpLocator,
    config: Map<String, Value>,
) -> Result<(), String> {
    if target.scope == McpScope::Project {
        return Err("Project 服务不能写入 mcp-disabled.json".into());
    }
    let ds = paths.disabled_store();
    let mut entries = wb.disabled_entries(&ds)?;
    entries.retain(|entry| !disabled_matches(entry, target));
    entries.push(DisabledEntry {
        scope: target.scope.clone(),
        name: target.name.clone(),
        instance_id: target.instance_id.clone(),
        project_path: target.project_path.clone(),
        config,
        disabled_at: now_secs(),
    });
    wb.disabled_replace(&ds, entries)
}

fn delete_locator_disabled(
    wb: &mut Workbench<'_>,
    paths: &McpPaths,
    target: &McpLocator,
) -> Result<(), String> {
    if target.scope == McpScope::Project {
        return Ok(());
    }
    let ds = paths.disabled_store();
    let entries = wb.disabled_entries(&ds)?;
    if !entries.iter().any(|entry| disabled_matches(entry, target)) {
        return Ok(());
    }
    let mut remaining = entries;
    remaining.retain(|entry| !disabled_matches(entry, target));
    wb.disabled_replace(&ds, remaining)
}

fn save_locator_with_state(
    wb: &mut Workbench<'_>,
    paths: &McpPaths,
    target: &McpLocator,
    config: Map<String, Value>,
    disabled: bool,
    clear_project_disabled: bool,
) -> Result<(), String> {
    if target.scope == McpScope::Project {
        save_locator_enabled(wb, paths, target, config)?;
        let project = target.project_path.as_deref().ok_or("缺少项目路径")?;
        let settings = paths.project_local_settings(project)?;
        if disabled {
            wb.settings_disable(&settings, &target.name)
        } else if clear_project_disabled {
            wb.settings_enable(&settings, &target.name)
        } else {
            Ok(())
        }
    } else if disabled {
        save_locator_disabled(wb, paths, target, config)
    } else {
        save_locator_enabled(wb, paths, target, config)
    }
}

// ---------------- apply：直接文件写入 ----------------

#[cfg(test)]
pub(crate) fn apply_action_files(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
) -> Result<Vec<String>, String> {
    apply_action_files_with_revisions(paths, instances, action, None)
}

pub(crate) fn apply_action_files_checked(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
    expected: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    apply_action_files_with_revisions(paths, instances, action, Some(expected))
}

fn apply_action_files_with_revisions(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
    expected: Option<&BTreeMap<String, String>>,
) -> Result<Vec<String>, String> {
    validate_action_strict(paths, instances, action)?;
    let affected = affected_source_ids(paths, instances, action)?;
    let mut wb = Workbench::new(paths);
    let mut affected_instances: Vec<String> = Vec::new();
    match action {
        McpChangeAction::Save {
            original,
            target,
            config,
            overwrite,
        } => {
            let target_presence = locator_presence(paths, instances, target)?;
            let original_presence = original
                .as_ref()
                .map(|locator| locator_presence(paths, instances, locator))
                .transpose()?;

            if let (Some(original), Some(presence)) = (original.as_ref(), original_presence) {
                if !presence.exists() {
                    return Err(format!("找不到原服务「{}」，请刷新后重试", original.name));
                }
            }

            let same_locator = original.as_ref() == Some(target);
            if !same_locator && target_presence.exists() && !*overwrite {
                return Err(format!("目标位置已存在服务「{}」，未选择覆盖", target.name));
            }

            // 有 original 时最终状态继承 original；新建/导入覆盖时保持 target 现有状态。
            let final_disabled = original_presence
                .map(LocatorPresence::disabled)
                .unwrap_or_else(|| target_presence.disabled());

            if !same_locator {
                if let (Some(original), Some(presence)) = (original.as_ref(), original_presence) {
                    if presence.enabled_source {
                        delete_locator_enabled(
                            &mut wb,
                            paths,
                            original,
                            presence.project_disabled,
                        )?;
                    } else if presence.manager_disabled {
                        delete_locator_disabled(&mut wb, paths, original)?;
                    }
                }
                if target_presence.enabled_source {
                    delete_locator_enabled(
                        &mut wb,
                        paths,
                        target,
                        target_presence.project_disabled,
                    )?;
                } else if target_presence.manager_disabled {
                    delete_locator_disabled(&mut wb, paths, target)?;
                }
            }

            save_locator_with_state(
                &mut wb,
                paths,
                target,
                config.clone(),
                final_disabled,
                target_presence.project_disabled && !final_disabled,
            )?;
            collect_instances(target, &mut affected_instances);
            if let Some(original) = original {
                collect_instances(original, &mut affected_instances);
            }
        }
        McpChangeAction::BatchSave { items } => {
            // items 数量/重复已在 validate_action_strict 校验
            for it in items {
                let presence = locator_presence(paths, instances, &it.target)?;
                if presence.exists() && !it.overwrite {
                    return Err(format!(
                        "目标位置已存在服务「{}」，未选择覆盖",
                        it.target.name
                    ));
                }
                save_locator_with_state(
                    &mut wb,
                    paths,
                    &it.target,
                    it.config.clone(),
                    presence.disabled(),
                    presence.project_disabled && !presence.disabled(),
                )?;
                collect_instances(&it.target, &mut affected_instances);
            }
        }
        McpChangeAction::SetEnabled { target, enabled } => {
            if *enabled {
                enable_locator(&mut wb, paths, target)?;
            } else {
                disable_locator(&mut wb, paths, target)?;
            }
            collect_instances(target, &mut affected_instances);
        }
        McpChangeAction::Delete { target } => {
            delete_locator_anywhere(&mut wb, paths, target)?;
            collect_instances(target, &mut affected_instances);
        }
    }
    let writes = wb.emit();
    let mut source_ids_by_path: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for source_id in affected {
        let path = source_file(paths, instances, &source_id)?;
        source_ids_by_path.entry(path).or_default().push(source_id);
    }
    for source_ids in source_ids_by_path.values_mut() {
        source_ids.sort();
        source_ids.dedup();
    }
    for write in &writes {
        if !source_ids_by_path.contains_key(&write.path) {
            return Err(format!(
                "内部错误：实际写入来源未出现在 preview 中：{}",
                write.path.display()
            ));
        }
    }
    apply_storage_transaction_ordered(paths, writes, &source_ids_by_path, expected)?;
    affected_instances.sort();
    affected_instances.dedup();
    Ok(affected_instances)
}

fn collect_instances(loc: &McpLocator, out: &mut Vec<String>) {
    if loc.scope == McpScope::User {
        out.push(MAIN_INSTANCE.to_string());
    } else if let Some(i) = &loc.instance_id {
        out.push(i.clone());
    }
}

fn save_locator_enabled(
    wb: &mut Workbench,
    paths: &McpPaths,
    target: &McpLocator,
    config: Map<String, Value>,
) -> Result<(), String> {
    match target.scope {
        McpScope::User => {
            let p = paths.instance_claude_json(MAIN_INSTANCE)?;
            wb.user_set(&p, &target.name, config)
        }
        McpScope::Local => {
            let (inst, proj) = local_parts(target)?;
            let canon = canonicalize_dir(&proj)?;
            let p = paths.instance_claude_json(&inst)?;
            wb.local_set(&p, &canon.display().to_string(), &target.name, config)
        }
        McpScope::Project => {
            let proj = target.project_path.as_deref().ok_or("缺少项目路径")?;
            let p = paths.project_mcp_json(proj)?;
            wb.project_set(&p, &target.name, config)
        }
    }
}

fn delete_locator_enabled(
    wb: &mut Workbench,
    paths: &McpPaths,
    target: &McpLocator,
    clear_project_disabled: bool,
) -> Result<(), String> {
    match target.scope {
        McpScope::User => {
            let p = paths.instance_claude_json(MAIN_INSTANCE)?;
            wb.user_del(&p, &target.name)
        }
        McpScope::Local => {
            let (inst, proj) = local_parts(target)?;
            let canon = canonicalize_dir(&proj)?;
            let p = paths.instance_claude_json(&inst)?;
            wb.local_del(&p, &canon.display().to_string(), &target.name)
        }
        McpScope::Project => {
            let proj = target.project_path.as_deref().ok_or("缺少项目路径")?;
            let mcp = paths.project_mcp_json(proj)?;
            wb.project_del(&mcp, &target.name)?;
            if clear_project_disabled {
                let settings = paths.project_local_settings(proj)?;
                wb.settings_enable(&settings, &target.name)?;
            }
            Ok(())
        }
    }
}

fn disable_locator(
    wb: &mut Workbench,
    paths: &McpPaths,
    target: &McpLocator,
) -> Result<(), String> {
    if target.scope == McpScope::Project {
        let proj = target.project_path.as_deref().ok_or("缺少项目路径")?;
        // 停用前确认 .mcp.json 中确实存在该服务
        if wb.current_config(target)?.is_none() {
            return Err(format!(
                "项目 .mcp.json 中不存在服务「{}」，无法停用",
                target.name
            ));
        }
        let settings = paths.project_local_settings(proj)?;
        return wb.settings_disable(&settings, &target.name);
    }
    // User/Local：找不到启用配置必须报错，禁止用空配置创建幽灵停用项
    let config = wb
        .current_config(target)?
        .ok_or_else(|| format!("找不到启用配置，无法停用「{}」", target.name))?;
    // 先写入停用仓库（载入顺序：仓库在前 → 落盘顺序仓库在前）
    let ds = paths.disabled_store();
    let mut entries = wb.disabled_entries(&ds)?;
    entries.retain(|e| !disabled_matches(e, target));
    entries.push(DisabledEntry {
        scope: target.scope.clone(),
        name: target.name.clone(),
        instance_id: target.instance_id.clone(),
        project_path: target.project_path.clone(),
        config,
        disabled_at: now_secs(),
    });
    wb.disabled_replace(&ds, entries)?;
    // 再从启用来源移除
    delete_locator_enabled(wb, paths, target, false)?;
    Ok(())
}

fn enable_locator(wb: &mut Workbench, paths: &McpPaths, target: &McpLocator) -> Result<(), String> {
    if target.scope == McpScope::Project {
        let proj = target.project_path.as_deref().ok_or("缺少项目路径")?;
        if wb.current_config(target)?.is_none() {
            return Err(format!(
                "项目 .mcp.json 中不存在服务「{}」，无法启用",
                target.name
            ));
        }
        let settings = paths.project_local_settings(proj)?;
        return wb.settings_enable(&settings, &target.name);
    }
    let ds = paths.disabled_store();
    let entries = wb.disabled_entries(&ds)?;
    let entry = entries
        .iter()
        .find(|e| disabled_matches(e, target))
        .cloned()
        .ok_or_else(|| format!("停用仓库中找不到「{}」", target.name))?;
    // 先写回启用来源（载入顺序：来源在前 → 落盘顺序来源在前）
    save_locator_enabled(wb, paths, target, entry.config.clone())?;
    let mut new_entries = entries;
    new_entries.retain(|e| !disabled_matches(e, target));
    wb.disabled_replace(&ds, new_entries)?;
    Ok(())
}

fn delete_locator_anywhere(
    wb: &mut Workbench,
    paths: &McpPaths,
    target: &McpLocator,
) -> Result<(), String> {
    match target.scope {
        McpScope::User | McpScope::Local => {
            if wb.current_config(target)?.is_some() {
                delete_locator_enabled(wb, paths, target, false)?;
            }
            let ds = paths.disabled_store();
            let entries = wb.disabled_entries(&ds)?;
            if entries.iter().any(|e| disabled_matches(e, target)) {
                let mut new_entries = entries;
                new_entries.retain(|e| !disabled_matches(e, target));
                wb.disabled_replace(&ds, new_entries)?;
            }
        }
        McpScope::Project => {
            delete_locator_enabled(wb, paths, target, true)?;
        }
    }
    Ok(())
}

// ---------------- 受影响 source / revision ----------------

pub(crate) fn locator_source_ids(loc: &McpLocator, instances: &[String]) -> Vec<String> {
    match loc.scope {
        McpScope::User => {
            let mut v = vec![source_id::user(MAIN_INSTANCE)];
            for n in instances {
                if n != MAIN_INSTANCE {
                    v.push(source_id::user(n));
                }
            }
            v
        }
        McpScope::Local => match (&loc.instance_id, &loc.project_path) {
            (Some(inst), Some(proj)) => vec![source_id::local(inst, proj)],
            _ => vec![],
        },
        McpScope::Project => match &loc.project_path {
            Some(proj) => vec![source_id::project(proj)],
            None => vec![],
        },
    }
}

pub(crate) fn affected_source_ids(
    paths: &McpPaths,
    instances: &[String],
    action: &McpChangeAction,
) -> Result<Vec<String>, String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    let mut needs_disabled = false;
    let mut needs_project_settings: BTreeSet<String> = BTreeSet::new();
    match action {
        McpChangeAction::Save {
            original, target, ..
        } => {
            // 方案要求移动/重命名始终校验 original 与 target 两侧 revision；
            // 即使某一侧当前停用、实际定义位于 manager store，也保留其直接 source。
            for locator in original.iter().chain(std::iter::once(target)) {
                for sid in locator_source_ids(locator, instances) {
                    set.insert(sid);
                }
            }
            let target_presence = locator_presence(paths, instances, target)?;
            let original_presence = original
                .as_ref()
                .map(|locator| locator_presence(paths, instances, locator))
                .transpose()?;
            if let (Some(original), Some(presence)) = (original.as_ref(), original_presence) {
                if !presence.exists() {
                    return Err(format!("找不到原服务「{}」，请刷新后重试", original.name));
                }
                if presence.enabled_source {
                    for sid in locator_source_ids(original, instances) {
                        set.insert(sid);
                    }
                }
                if presence.manager_disabled {
                    set.insert(source_id::DISABLED.to_string());
                }
                if presence.project_disabled {
                    if let Some(project) = &original.project_path {
                        needs_project_settings.insert(source_id::project_settings(project));
                    }
                }
            }
            if target_presence.enabled_source {
                for sid in locator_source_ids(target, instances) {
                    set.insert(sid);
                }
            }
            if target_presence.manager_disabled {
                set.insert(source_id::DISABLED.to_string());
            }
            if target_presence.project_disabled {
                if let Some(project) = &target.project_path {
                    needs_project_settings.insert(source_id::project_settings(project));
                }
            }

            let final_disabled = original_presence
                .map(LocatorPresence::disabled)
                .unwrap_or_else(|| target_presence.disabled());
            if target.scope == McpScope::Project {
                for sid in locator_source_ids(target, instances) {
                    set.insert(sid);
                }
                if final_disabled {
                    if let Some(project) = &target.project_path {
                        needs_project_settings.insert(source_id::project_settings(project));
                    }
                }
            } else if final_disabled {
                set.insert(source_id::DISABLED.to_string());
            } else {
                for sid in locator_source_ids(target, instances) {
                    set.insert(sid);
                }
            }
        }
        McpChangeAction::BatchSave { items } => {
            for it in items {
                for sid in locator_source_ids(&it.target, instances) {
                    set.insert(sid);
                }
                let presence = locator_presence(paths, instances, &it.target)?;
                if it.target.scope == McpScope::Project {
                    for sid in locator_source_ids(&it.target, instances) {
                        set.insert(sid);
                    }
                    if presence.project_disabled {
                        if let Some(project) = &it.target.project_path {
                            needs_project_settings.insert(source_id::project_settings(project));
                        }
                    }
                } else if presence.manager_disabled {
                    set.insert(source_id::DISABLED.to_string());
                } else {
                    for sid in locator_source_ids(&it.target, instances) {
                        set.insert(sid);
                    }
                }
            }
        }
        McpChangeAction::SetEnabled { target, .. } => {
            for sid in locator_source_ids(target, instances) {
                set.insert(sid);
            }
            match target.scope {
                McpScope::User | McpScope::Local => needs_disabled = true,
                McpScope::Project => {
                    if let Some(p) = &target.project_path {
                        needs_project_settings.insert(source_id::project_settings(p));
                    }
                }
            }
        }
        McpChangeAction::Delete { target } => {
            for sid in locator_source_ids(target, instances) {
                set.insert(sid);
            }
            match target.scope {
                McpScope::User | McpScope::Local => needs_disabled = true,
                McpScope::Project => {
                    if let Some(p) = &target.project_path {
                        needs_project_settings.insert(source_id::project_settings(p));
                    }
                }
            }
        }
    }
    if needs_disabled {
        set.insert(source_id::DISABLED.to_string());
    }
    for ps in needs_project_settings {
        set.insert(ps);
    }
    Ok(set.into_iter().collect())
}

pub(crate) fn check_revisions(
    paths: &McpPaths,
    instances: &[String],
    affected: &[String],
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    for sid in affected {
        let path = source_file(paths, instances, sid)?;
        let current = revision(&path);
        let exp = expected
            .get(sid)
            .ok_or_else(|| format!("revision 集合缺少 {sid}（请刷新页面后重试）"))?;
        if current.as_str() != exp.as_str() {
            return Err(format!("配置已被外部修改（{sid}），请刷新后重试"));
        }
    }
    Ok(())
}

pub(crate) fn touches_user_or_local(action: &McpChangeAction) -> bool {
    let scope_of = |l: &McpLocator| matches!(l.scope, McpScope::User | McpScope::Local);
    match action {
        McpChangeAction::Save {
            original, target, ..
        } => original.iter().any(scope_of) || scope_of(target),
        McpChangeAction::BatchSave { items } => items.iter().any(|i| scope_of(&i.target)),
        McpChangeAction::SetEnabled { target, .. } | McpChangeAction::Delete { target } => {
            scope_of(target)
        }
    }
}

pub(crate) fn touches_user(action: &McpChangeAction) -> bool {
    let is_user = |l: &McpLocator| l.scope == McpScope::User;
    match action {
        McpChangeAction::Save {
            original, target, ..
        } => original.iter().any(is_user) || is_user(target),
        McpChangeAction::BatchSave { items } => items.iter().any(|i| is_user(&i.target)),
        McpChangeAction::SetEnabled { target, .. } | McpChangeAction::Delete { target } => {
            is_user(target)
        }
    }
}

// ---------------- list / collect_state ----------------

pub(crate) fn collect_state(paths: &McpPaths, profile_names: &[String]) -> McpState {
    let instances = all_instances(profile_names);
    let mut issues: Vec<McpSourceIssue> = Vec::new();
    let mut revisions: BTreeMap<String, String> = BTreeMap::new();
    let mut project_set: BTreeSet<String> = BTreeSet::new();

    let (reg, reg_issue) = read_registry(paths);
    revisions.insert(
        source_id::PROJECTS.to_string(),
        revision(&paths.project_registry()),
    );
    if let Some(i) = reg_issue {
        issues.push(i);
    }
    for p in &reg.projects {
        match canonicalize_dir(p) {
            Ok(c) => {
                project_set.insert(c.display().to_string());
            }
            Err(e) => {
                // 注册项目目录不可访问/不存在：保留登记路径生成来源问题，禁止静默跳过。
                issues.push(McpSourceIssue {
                    source_id: source_id::PROJECTS.to_string(),
                    path: p.clone(),
                    detail: e,
                });
            }
        }
    }
    let registered: BTreeSet<String> = project_set.clone();

    enum Row {
        User {
            name: String,
            config: Map<String, Value>,
        },
        Local {
            instance: String,
            project: String,
            name: String,
            config: Map<String, Value>,
        },
    }
    let mut enabled_rows: Vec<Row> = Vec::new();
    let mut main_user_map: Option<Map<String, Value>> = None;
    let mut contexts: BTreeSet<(String, String)> = BTreeSet::new();

    for inst in &instances {
        let path = match paths.instance_claude_json(inst) {
            Ok(p) => p,
            Err(e) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::user(inst),
                    path: String::new(),
                    detail: e,
                });
                continue;
            }
        };
        revisions.insert(source_id::user(inst), revision(&path));
        match read_doc(&path) {
            DocRead::Value(doc) if doc.is_object() => {
                let user_map = match read_user_map(&doc) {
                    Ok(m) => m,
                    Err(e) => {
                        issues.push(McpSourceIssue {
                            source_id: source_id::user(inst),
                            path: path.display().to_string(),
                            detail: e,
                        });
                        Map::new()
                    }
                };
                if inst == MAIN_INSTANCE {
                    main_user_map = Some(user_map.clone());
                    for (name, cfg) in &user_map {
                        if let Some(cfg_obj) = cfg.as_object() {
                            enabled_rows.push(Row::User {
                                name: name.clone(),
                                config: cfg_obj.clone(),
                            });
                        } else {
                            issues.push(McpSourceIssue {
                                source_id: source_id::user(inst),
                                path: path.display().to_string(),
                                detail: format!("mcpServers[{name}] 不是对象"),
                            });
                        }
                    }
                } else if let Some(main) = &main_user_map {
                    if user_map.len() != main.len()
                        || user_map.iter().any(|(k, v)| main.get(k) != Some(v))
                    {
                        issues.push(McpSourceIssue {
                            source_id: source_id::user(inst),
                            path: path.display().to_string(),
                            detail: "该实例的顶层 mcpServers 与主账户不一致，将在下次同步收敛"
                                .into(),
                        });
                    }
                }
                match doc.get("projects") {
                    None => {}
                    Some(projs_val) if projs_val.is_object() => {
                        let projs = projs_val.as_object().unwrap();
                        for origkey in projs.keys() {
                            let canon = match canonicalize_dir(origkey) {
                                Ok(c) => c,
                                Err(e) => {
                                    // 项目键不可访问/不存在：生成来源问题，禁止静默跳过。
                                    issues.push(McpSourceIssue {
                                        source_id: source_id::user(inst),
                                        path: path.display().to_string(),
                                        detail: format!("项目键「{origkey}」无法规范化：{e}"),
                                    });
                                    continue;
                                }
                            };
                            let cs = canon.display().to_string();
                            contexts.insert((inst.clone(), cs.clone()));
                            project_set.insert(cs.clone());
                            revisions.insert(source_id::local(inst, &cs), revision(&path));
                            let local_map = match read_local_map(&doc, &cs) {
                                Ok(m) => m,
                                Err(e) => {
                                    issues.push(McpSourceIssue {
                                        source_id: source_id::local(inst, &cs),
                                        path: path.display().to_string(),
                                        detail: e,
                                    });
                                    Map::new()
                                }
                            };
                            for (name, cfg) in &local_map {
                                if let Some(cfg_obj) = cfg.as_object() {
                                    enabled_rows.push(Row::Local {
                                        instance: inst.clone(),
                                        project: cs.clone(),
                                        name: name.clone(),
                                        config: cfg_obj.clone(),
                                    });
                                } else {
                                    issues.push(McpSourceIssue {
                                        source_id: source_id::local(inst, &cs),
                                        path: path.display().to_string(),
                                        detail: format!("mcpServers[{name}] 不是对象"),
                                    });
                                }
                            }
                        }
                    }
                    Some(_) => {
                        // .claude.json 存在 projects 但不是对象：生成来源问题。
                        issues.push(McpSourceIssue {
                            source_id: source_id::user(inst),
                            path: path.display().to_string(),
                            detail: "projects 不是对象".into(),
                        });
                    }
                }
            }
            DocRead::Value(_) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::user(inst),
                    path: path.display().to_string(),
                    detail: ".claude.json 顶层不是对象".into(),
                });
            }
            DocRead::Missing => {
                if inst == MAIN_INSTANCE {
                    main_user_map = Some(Map::new());
                }
            }
            DocRead::Failed(e) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::user(inst),
                    path: path.display().to_string(),
                    detail: e,
                });
            }
        }
    }

    // 覆盖关系以“全部实例 × 全部已知项目”为上下文全集。手工登记但尚未出现在
    // .claude.json.projects 的项目同样会在该项目运行时覆盖同名 User 定义。
    contexts.clear();
    for inst in &instances {
        for project in &project_set {
            contexts.insert((inst.clone(), project.clone()));
        }
    }

    struct ProjectRow {
        project: String,
        name: String,
        config: Map<String, Value>,
    }
    let mut project_rows: Vec<ProjectRow> = Vec::new();
    let mut project_disabled: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for proj in &project_set {
        let mcp_json = match paths.project_mcp_json(proj) {
            Ok(p) => p,
            Err(e) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::project(proj),
                    path: String::new(),
                    detail: e,
                });
                continue;
            }
        };
        revisions.insert(source_id::project(proj), revision(&mcp_json));
        let settings_path = paths
            .project_local_settings(proj)
            .unwrap_or_else(|_| PathBuf::from(proj));
        revisions.insert(source_id::project_settings(proj), revision(&settings_path));
        let (disabled_names, settings_issue) = read_disabled_project_names(&settings_path, proj);
        if let Some(i) = settings_issue {
            issues.push(i);
        }
        project_disabled.insert(proj.clone(), disabled_names);
        match read_doc(&mcp_json) {
            DocRead::Value(doc) if doc.is_object() => {
                let map = match read_project_mcp_map(&doc) {
                    Ok(m) => m,
                    Err(e) => {
                        issues.push(McpSourceIssue {
                            source_id: source_id::project(proj),
                            path: mcp_json.display().to_string(),
                            detail: e,
                        });
                        Map::new()
                    }
                };
                for (name, cfg) in &map {
                    if let Some(cfg_obj) = cfg.as_object() {
                        project_rows.push(ProjectRow {
                            project: proj.clone(),
                            name: name.clone(),
                            config: cfg_obj.clone(),
                        });
                    } else {
                        issues.push(McpSourceIssue {
                            source_id: source_id::project(proj),
                            path: mcp_json.display().to_string(),
                            detail: format!("mcpServers[{name}] 不是对象"),
                        });
                    }
                }
            }
            DocRead::Value(_) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::project(proj),
                    path: mcp_json.display().to_string(),
                    detail: ".mcp.json 顶层不是对象".into(),
                });
            }
            DocRead::Missing => {}
            DocRead::Failed(e) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::project(proj),
                    path: mcp_json.display().to_string(),
                    detail: e,
                });
            }
        }
    }

    // 所有实例×已知项目组合（含尚未创建的 Local 上下文），保证新建 Local 有 revision 可用。
    // instance_claude_json 失败（非法实例标识）必须生成来源问题，禁止用 if let Ok 静默丢弃。
    for inst in &instances {
        let p = match paths.instance_claude_json(inst) {
            Ok(p) => p,
            Err(e) => {
                issues.push(McpSourceIssue {
                    source_id: source_id::user(inst),
                    path: String::new(),
                    detail: e,
                });
                continue;
            }
        };
        let rev = storage::revision(&p);
        for proj in &project_set {
            revisions
                .entry(source_id::local(inst, proj))
                .or_insert_with(|| rev.clone());
        }
    }

    let (disabled, disabled_issue) = read_disabled(paths);
    revisions.insert(
        source_id::DISABLED.to_string(),
        revision(&paths.disabled_store()),
    );
    if let Some(i) = disabled_issue {
        issues.push(i);
    }

    // ---- 覆盖关系索引：Project 仅计入未停用的（停用后 User 恢复生效）----
    use std::collections::HashMap;
    let mut local_by_name: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in &enabled_rows {
        if let Row::Local {
            instance,
            project,
            name,
            ..
        } = r
        {
            local_by_name
                .entry(name.clone())
                .or_default()
                .push((instance.clone(), project.clone()));
        }
    }
    let mut project_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for r in &project_rows {
        let disabled_names = project_disabled
            .get(&r.project)
            .cloned()
            .unwrap_or_default();
        if !disabled_names.contains(&r.name) {
            project_by_name
                .entry(r.name.clone())
                .or_default()
                .push(r.project.clone());
        }
    }

    let mut services: Vec<McpService> = Vec::new();
    let mut enabled_keys: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for r in &enabled_rows {
        match r {
            Row::User { name, .. } => {
                enabled_keys.insert(("user".into(), name.clone(), String::new(), String::new()));
            }
            Row::Local {
                instance,
                project,
                name,
                ..
            } => {
                enabled_keys.insert((
                    "local".into(),
                    name.clone(),
                    instance.clone(),
                    project.clone(),
                ));
            }
        }
    }

    // User 行
    for r in &enabled_rows {
        if let Row::User { name, config } = r {
            let (transport, raw_t) = transport_pair(config);
            let mut warns = validation::config_warnings(config);
            if !validation::name_is_valid(name) {
                warns.push(format!("服务名「{name}」不符合命名规则，建议重命名"));
            }
            let mut overridden = 0usize;
            for (inst, proj) in &contexts {
                let has_local = local_by_name
                    .get(name)
                    .map(|v| v.contains(&(inst.clone(), proj.clone())))
                    .unwrap_or(false);
                let has_project = project_by_name
                    .get(name)
                    .map(|v| v.contains(proj))
                    .unwrap_or(false);
                if has_local || has_project {
                    overridden += 1;
                }
            }
            let total = contexts.len();
            let (state, shadowed_by) = if overridden == 0 {
                (McpEffectiveState::Effective, vec![])
            } else if overridden == total {
                (
                    McpEffectiveState::Shadowed,
                    shadow_refs_for_user(name, &local_by_name, &project_by_name),
                )
            } else {
                (
                    McpEffectiveState::PartiallyShadowed,
                    shadow_refs_for_user(name, &local_by_name, &project_by_name),
                )
            };
            services.push(McpService {
                locator: McpLocator {
                    scope: McpScope::User,
                    name: name.clone(),
                    instance_id: None,
                    project_path: None,
                },
                transport,
                raw_transport: raw_t,
                config: config.clone(),
                enabled: true,
                effective_state: state,
                shadowed_by,
                shadowed_context_count: overridden,
                source_id: source_id::user(MAIN_INSTANCE),
                revision: revisions
                    .get(&source_id::user(MAIN_INSTANCE))
                    .cloned()
                    .unwrap_or_else(|| "missing".into()),
                sensitive_paths: validation::sensitive_paths(&Value::Object(config.clone())),
                warnings: warns,
            });
        }
    }

    // Local 行
    for r in &enabled_rows {
        if let Row::Local {
            instance,
            project,
            name,
            config,
        } = r
        {
            let (transport, raw_t) = transport_pair(config);
            let mut warns = validation::config_warnings(config);
            if !validation::name_is_valid(name) {
                warns.push(format!("服务名「{name}」不符合命名规则，建议重命名"));
            }
            services.push(McpService {
                locator: McpLocator {
                    scope: McpScope::Local,
                    name: name.clone(),
                    instance_id: Some(instance.clone()),
                    project_path: Some(project.clone()),
                },
                transport,
                raw_transport: raw_t,
                config: config.clone(),
                enabled: true,
                effective_state: McpEffectiveState::Effective,
                shadowed_by: vec![],
                shadowed_context_count: 0,
                source_id: source_id::local(instance, project),
                revision: revisions
                    .get(&source_id::local(instance, project))
                    .cloned()
                    .unwrap_or_else(|| "missing".into()),
                sensitive_paths: validation::sensitive_paths(&Value::Object(config.clone())),
                warnings: warns,
            });
        }
    }

    // Project 行
    for r in &project_rows {
        let disabled_names = project_disabled
            .get(&r.project)
            .cloned()
            .unwrap_or_default();
        let enabled = !disabled_names.contains(&r.name);
        let (transport, raw_t) = transport_pair(&r.config);
        let mut warns = validation::config_warnings(&r.config);
        if !validation::name_is_valid(&r.name) {
            warns.push(format!("服务名「{}」不符合命名规则，建议重命名", r.name));
        }
        let local_shadow: Vec<(String, String)> = local_by_name
            .get(&r.name)
            .map(|v| v.iter().filter(|(_, p)| p == &r.project).cloned().collect())
            .unwrap_or_default();
        let (state, shadowed_by) = if !enabled {
            (McpEffectiveState::Disabled, vec![])
        } else if !local_shadow.is_empty() {
            (
                McpEffectiveState::Shadowed,
                local_shadow
                    .iter()
                    .map(|(inst, p)| McpShadowRef {
                        scope: McpScope::Local,
                        name: r.name.clone(),
                        instance_id: Some(inst.clone()),
                        project_path: Some(p.clone()),
                    })
                    .collect(),
            )
        } else {
            (McpEffectiveState::Effective, vec![])
        };
        services.push(McpService {
            locator: McpLocator {
                scope: McpScope::Project,
                name: r.name.clone(),
                instance_id: None,
                project_path: Some(r.project.clone()),
            },
            transport,
            raw_transport: raw_t,
            config: r.config.clone(),
            enabled,
            effective_state: state,
            shadowed_by,
            shadowed_context_count: local_shadow.len(),
            source_id: source_id::project(&r.project),
            revision: revisions
                .get(&source_id::project(&r.project))
                .cloned()
                .unwrap_or_else(|| "missing".into()),
            sensitive_paths: validation::sensitive_paths(&Value::Object(r.config.clone())),
            warnings: warns,
        });
    }

    // 停用仓库行（User/Local）
    for e in &disabled.entries {
        let in_enabled = match e.scope {
            McpScope::User => enabled_keys.contains(&(
                "user".into(),
                e.name.clone(),
                String::new(),
                String::new(),
            )),
            McpScope::Local => enabled_keys.iter().any(|k| {
                k.0 == "local"
                    && k.1 == e.name
                    && k.2 == e.instance_id.clone().unwrap_or_default()
                    && norm_path(&k.3) == norm_opt(e.project_path.as_deref())
            }),
            McpScope::Project => continue,
        };
        if in_enabled {
            continue;
        }
        let (transport, raw_t) = transport_pair(&e.config);
        let mut warns = validation::config_warnings(&e.config);
        if !validation::name_is_valid(&e.name) {
            warns.push(format!("服务名「{}」不符合命名规则", e.name));
        }
        services.push(McpService {
            locator: McpLocator {
                scope: e.scope.clone(),
                name: e.name.clone(),
                instance_id: e.instance_id.clone(),
                project_path: e.project_path.clone(),
            },
            transport,
            raw_transport: raw_t,
            config: e.config.clone(),
            enabled: false,
            effective_state: McpEffectiveState::Disabled,
            shadowed_by: vec![],
            shadowed_context_count: 0,
            source_id: source_id::DISABLED.to_string(),
            revision: revisions
                .get(source_id::DISABLED)
                .cloned()
                .unwrap_or_else(|| "missing".into()),
            sensitive_paths: validation::sensitive_paths(&Value::Object(e.config.clone())),
            warnings: warns,
        });
    }

    services.sort_by(|a, b| {
        let rank = |s: &McpService| match s.effective_state {
            McpEffectiveState::Disabled => 0,
            _ if !s.warnings.is_empty() => 1,
            McpEffectiveState::Shadowed | McpEffectiveState::PartiallyShadowed => 2,
            _ => 3,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.locator.name.cmp(&b.locator.name))
    });

    let instance_refs: Vec<McpInstanceRef> = instances
        .iter()
        .map(|i| McpInstanceRef {
            id: i.clone(),
            label: if i == MAIN_INSTANCE {
                "主账户".into()
            } else {
                i.clone()
            },
        })
        .collect();
    let project_refs: Vec<McpProjectRef> = project_set
        .iter()
        .map(|p| McpProjectRef {
            path: p.clone(),
            label: label_of(p),
            discovered: !registered.contains(p),
        })
        .collect();

    let summary = build_summary(&services, &issues);

    McpState {
        services,
        instances: instance_refs,
        projects: project_refs,
        revisions,
        issues,
        summary,
        operation_warnings: Vec::new(),
        sync_targets: Vec::new(),
        sync_target_revisions: BTreeMap::new(),
    }
}

fn transport_pair(config: &Map<String, Value>) -> (McpTransport, Option<String>) {
    (
        validation::infer_transport(config),
        validation::raw_transport(config),
    )
}

fn shadow_refs_for_user(
    name: &str,
    local_by_name: &std::collections::HashMap<String, Vec<(String, String)>>,
    project_by_name: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<McpShadowRef> {
    let mut refs = vec![];
    if let Some(locals) = local_by_name.get(name) {
        for (inst, proj) in locals {
            refs.push(McpShadowRef {
                scope: McpScope::Local,
                name: name.to_string(),
                instance_id: Some(inst.clone()),
                project_path: Some(proj.clone()),
            });
        }
    }
    if let Some(projs) = project_by_name.get(name) {
        for proj in projs {
            refs.push(McpShadowRef {
                scope: McpScope::Project,
                name: name.to_string(),
                instance_id: None,
                project_path: Some(proj.clone()),
            });
        }
    }
    refs
}

fn build_summary(services: &[McpService], issues: &[McpSourceIssue]) -> McpSummary {
    McpSummary {
        total: services.len(),
        enabled: services.iter().filter(|s| s.enabled).count(),
        disabled: services.iter().filter(|s| !s.enabled).count(),
        warnings: services.iter().filter(|s| !s.warnings.is_empty()).count() + issues.len(),
        shadowed: services
            .iter()
            .filter(|s| {
                matches!(
                    s.effective_state,
                    McpEffectiveState::Shadowed | McpEffectiveState::PartiallyShadowed
                )
            })
            .count(),
    }
}

fn label_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| path.to_string())
}

/// 返回 (disabled 列表, 解析问题)。sourceId 必须用 canonical 项目路径，不能拼 settings 文件路径。
fn read_disabled_project_names(
    settings_path: &Path,
    project_canon: &str,
) -> (Vec<String>, Option<McpSourceIssue>) {
    match read_doc(settings_path) {
        DocRead::Value(v) => {
            let mut names = Vec::new();
            let mut issue = None;
            match v.get("disabledMcpjsonServers") {
                None => {}
                Some(arr) if arr.is_array() => {
                    for x in arr.as_array().unwrap() {
                        match x.as_str() {
                            Some(s) => names.push(s.to_string()),
                            None => {
                                issue = Some(McpSourceIssue {
                                    source_id: source_id::project_settings(project_canon),
                                    path: settings_path.display().to_string(),
                                    detail: "disabledMcpjsonServers 含非字符串项".into(),
                                });
                                break;
                            }
                        }
                    }
                }
                Some(_) => {
                    issue = Some(McpSourceIssue {
                        source_id: source_id::project_settings(project_canon),
                        path: settings_path.display().to_string(),
                        detail: "disabledMcpjsonServers 不是数组".into(),
                    });
                }
            }
            (names, issue)
        }
        DocRead::Missing => (Vec::new(), None),
        DocRead::Failed(e) => (
            Vec::new(),
            Some(McpSourceIssue {
                source_id: source_id::project_settings(project_canon),
                path: settings_path.display().to_string(),
                detail: e,
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup() -> (McpPaths, Temp) {
        // CI 会并行创建大量测试目录，单靠系统时钟可能在同一时钟刻度内重名。
        // uniq_token 同时包含进程号、时间与进程内原子序号，确保测试之间完全隔离。
        let dir = std::env::temp_dir().join(format!("ccm-mcp-test-{}", uniq_token()));
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
        write_json_transactional(paths, &paths.main_claude_json(), &v).unwrap();
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
    fn proj_target(proj: &str, name: &str) -> McpLocator {
        McpLocator {
            scope: McpScope::Project,
            name: name.into(),
            instance_id: None,
            project_path: Some(proj.into()),
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

    #[test]
    fn path_resolution_three_scopes() {
        let (paths, _t) = setup();
        assert_eq!(paths.main_claude_json(), paths.home.join(".claude.json"));
        assert_eq!(
            paths.instance_claude_json("__main__").unwrap(),
            paths.main_claude_json()
        );
        assert!(paths.instance_claude_json("bad name").is_err());
        assert!(paths.instance_claude_json("..").is_err());
    }

    #[test]
    fn rejects_unregistered_or_file_project() {
        let (paths, _t) = setup();
        assert!(paths.project_mcp_json("E:\\does\\not\\exist").is_err());
        let f = paths.home.join("afile.txt");
        fs::write(&f, "x").unwrap();
        assert!(register_project(&paths, &f.display().to_string()).is_err());
    }

    #[test]
    fn user_save_then_delete_preserves_other_fields() {
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers":{"keep":{"command":"node"}},"otherLogin":"xyz"}),
        );
        let target = user_locator("new");
        apply_action_files(
            &paths,
            &[],
            &save_action(target.clone(), map_of(json!({"command":"go"}))),
        )
        .unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["new"]["command"], "go");
        assert_eq!(doc["mcpServers"]["keep"]["command"], "node");
        assert_eq!(doc["otherLogin"], "xyz");
        apply_action_files(&paths, &[], &McpChangeAction::Delete { target }).unwrap();
        let doc2: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(doc2["mcpServers"].get("new").is_none());
        assert_eq!(doc2["mcpServers"]["keep"]["command"], "node");
        assert_eq!(doc2["otherLogin"], "xyz");
    }

    #[test]
    fn unknown_fields_preserved() {
        let (paths, _t) = setup();
        let target = user_locator("u");
        apply_action_files(
            &paths,
            &[],
            &save_action(
                target,
                map_of(json!({"command":"node","args":["x"],"customField":{"nested":1}})),
            ),
        )
        .unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["u"]["customField"]["nested"], 1);
    }

    #[test]
    fn local_only_modifies_named_instance() {
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let proj = paths.home.join("proj-a");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let p = paths.instance_claude_json("alpha").unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_json_transactional(&paths, &p, &json!({})).unwrap();
        let target = McpLocator {
            scope: McpScope::Local,
            name: "lsrv".into(),
            instance_id: Some("alpha".into()),
            project_path: Some(proj.display().to_string()),
        };
        apply_action_files(
            &paths,
            &["alpha".to_string()],
            &save_action(target, map_of(json!({"command":"node"}))),
        )
        .unwrap();
        let alpha_doc: Value = serde_json::from_str(
            &fs::read_to_string(paths.instance_claude_json("alpha").unwrap()).unwrap(),
        )
        .unwrap();
        let key = alpha_doc["projects"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        assert_eq!(
            alpha_doc["projects"][&key]["mcpServers"]["lsrv"]["command"],
            "node"
        );
        assert!(serde_json::from_str::<Value>(
            &fs::read_to_string(paths.main_claude_json()).unwrap()
        )
        .unwrap()
        .get("projects")
        .is_none());
    }

    #[test]
    fn project_only_modifies_mcp_json() {
        let (paths, _t) = setup();
        let proj = paths.home.join("proj-b");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let target = McpLocator {
            scope: McpScope::Project,
            name: "psrv".into(),
            instance_id: None,
            project_path: Some(proj.display().to_string()),
        };
        apply_action_files(
            &paths,
            &[],
            &save_action(
                target,
                map_of(json!({"type":"http","url":"https://example.com/mcp"})),
            ),
        )
        .unwrap();
        let mcp_doc: Value =
            serde_json::from_str(&fs::read_to_string(proj.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(
            mcp_doc["mcpServers"]["psrv"]["url"],
            "https://example.com/mcp"
        );
        assert!(
            !proj.join(".claude").join("settings.local.json").exists(),
            "启用态 Project 保存不得创建 settings.local.json"
        );
    }

    #[test]
    fn enabled_project_edit_keeps_existing_settings_bytes_unchanged() {
        let (paths, _t) = setup();
        let proj = paths.home.join("proj-enabled-edit");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let target = proj_target(&proj.display().to_string(), "psrv");
        apply_action_files(
            &paths,
            &[],
            &save_action(
                target.clone(),
                map_of(json!({"type":"http","url":"https://example.com/one"})),
            ),
        )
        .unwrap();
        let settings = proj.join(".claude").join("settings.local.json");
        fs::write(&settings, b"{\n  \"unrelated\": true\n}\n").unwrap();
        let before = fs::read(&settings).unwrap();
        let edit = McpChangeAction::Save {
            original: Some(target.clone()),
            target,
            config: map_of(json!({"type":"http","url":"https://example.com/two"})),
            overwrite: false,
        };
        apply_action_files(&paths, &[], &edit).unwrap();
        assert_eq!(fs::read(&settings).unwrap(), before);
    }

    #[test]
    fn disabled_project_edit_preserves_disabled_state() {
        let (paths, _t) = setup();
        let proj = paths.home.join("proj-disabled-edit");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let target = proj_target(&proj.display().to_string(), "psrv");
        apply_action_files(
            &paths,
            &[],
            &save_action(
                target.clone(),
                map_of(json!({"type":"http","url":"https://example.com/one"})),
            ),
        )
        .unwrap();
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let edit = McpChangeAction::Save {
            original: Some(target.clone()),
            target: target.clone(),
            config: map_of(json!({"type":"http","url":"https://example.com/two"})),
            overwrite: false,
        };
        apply_action_files(&paths, &[], &edit).unwrap();
        let settings: Value = serde_json::from_str(
            &fs::read_to_string(proj.join(".claude").join("settings.local.json")).unwrap(),
        )
        .unwrap();
        assert!(settings["disabledMcpjsonServers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("psrv")));
    }

    #[test]
    fn user_disable_then_enable_roundtrip() {
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers":{"d":{"command":"node","env":{"TOKEN":"x"}}}}),
        );
        let target = user_locator("d");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let main: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(main["mcpServers"].as_object().unwrap().is_empty());
        let (store, _) = read_disabled(&paths);
        assert!(store
            .entries
            .iter()
            .any(|e| e.name == "d" && e.config.get("env").is_some()));
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target,
                enabled: true,
            },
        )
        .unwrap();
        let main2: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert_eq!(main2["mcpServers"]["d"]["command"], "node");
        let (store2, _) = read_disabled(&paths);
        assert!(store2.entries.is_empty());
    }

    #[test]
    fn project_disable_only_changes_settings() {
        let (paths, _t) = setup();
        let proj = paths.home.join("proj-c");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let target = McpLocator {
            scope: McpScope::Project,
            name: "psrv".into(),
            instance_id: None,
            project_path: Some(proj.display().to_string()),
        };
        apply_action_files(
            &paths,
            &[],
            &save_action(target.clone(), map_of(json!({"command":"node"}))),
        )
        .unwrap();
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: proj_target(&proj.display().to_string(), "psrv"),
                enabled: false,
            },
        )
        .unwrap();
        let mcp_doc: Value =
            serde_json::from_str(&fs::read_to_string(proj.join(".mcp.json")).unwrap()).unwrap();
        assert!(mcp_doc["mcpServers"].get("psrv").is_some());
        let set_doc: Value = serde_json::from_str(
            &fs::read_to_string(proj.join(".claude").join("settings.local.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(set_doc["disabledMcpjsonServers"][0], "psrv");
    }

    #[test]
    fn revision_change_rejects_apply() {
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"x":{"command":"node"}}}));
        let rev = revision(&paths.main_claude_json());
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_main(&paths, json!({"mcpServers":{"x":{"command":"python"}}}));
        let mut expected = BTreeMap::new();
        expected.insert(source_id::user(MAIN_INSTANCE), rev);
        assert!(
            check_revisions(&paths, &[], &[source_id::user(MAIN_INSTANCE)], &expected).is_err()
        );
    }

    #[test]
    fn revision_rejects_same_length_content_change() {
        // 同长度、同时间粒度下内容变化：hash 不同仍触发 revision 冲突
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"x":{"command":"aaaa"}}}));
        let rev = revision(&paths.main_claude_json());
        write_main(&paths, json!({"mcpServers":{"x":{"command":"bbbb"}}})); // 同长度
        let mut expected = BTreeMap::new();
        expected.insert(source_id::user(MAIN_INSTANCE), rev);
        assert!(
            check_revisions(&paths, &[], &[source_id::user(MAIN_INSTANCE)], &expected).is_err()
        );
    }

    #[test]
    fn list_reflects_shadowing() {
        let (paths, _t) = setup();
        let proj = paths.home.join("proj-d");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        write_main(
            &paths,
            json!({
                "mcpServers": {"shared": {"command": "node"}},
                "projects": { proj.display().to_string(): {} }
            }),
        );
        write_json_transactional(
            &paths,
            &proj.join(".mcp.json"),
            &json!({"mcpServers":{"shared":{"type":"http","url":"https://e.com"}}}),
        )
        .unwrap();
        let state = collect_state(&paths, &[]);
        let user_shared = state
            .services
            .iter()
            .find(|s| s.locator.scope == McpScope::User && s.locator.name == "shared")
            .unwrap();
        assert!(user_shared.shadowed_context_count >= 1);
        let proj_shared = state
            .services
            .iter()
            .find(|s| s.locator.scope == McpScope::Project && s.locator.name == "shared")
            .unwrap();
        assert_eq!(proj_shared.effective_state, McpEffectiveState::Effective);
    }

    #[test]
    fn registered_project_participates_in_user_shadowing_before_discovery() {
        let (paths, _t) = setup();
        let proj = paths.home.join("registered-only-shadow");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        write_main(&paths, json!({"mcpServers":{"shared":{"command":"node"}}}));
        write_json_transactional(
            &paths,
            &proj.join(".mcp.json"),
            &json!({"mcpServers":{"shared":{"type":"http","url":"https://e.com"}}}),
        )
        .unwrap();

        let state = collect_state(&paths, &["alpha".to_string()]);
        let user = state
            .services
            .iter()
            .find(|service| {
                service.locator.scope == McpScope::User && service.locator.name == "shared"
            })
            .unwrap();
        assert_eq!(user.effective_state, McpEffectiveState::Shadowed);
        assert_eq!(user.shadowed_context_count, 2);
    }

    #[test]
    fn disabled_project_does_not_shadow_user() {
        // Project 停用后，同名 User 在该上下文恢复生效
        let (paths, _t) = setup();
        let proj = paths.home.join("proj-e");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        write_main(
            &paths,
            json!({
                "mcpServers": {"shared": {"command": "node"}},
                "projects": { proj.display().to_string(): {} }
            }),
        );
        write_json_transactional(
            &paths,
            &proj.join(".mcp.json"),
            &json!({"mcpServers":{"shared":{"type":"http","url":"https://e.com"}}}),
        )
        .unwrap();
        // 停用 Project shared
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: proj_target(&proj.display().to_string(), "shared"),
                enabled: false,
            },
        )
        .unwrap();
        let state = collect_state(&paths, &[]);
        let user_shared = state
            .services
            .iter()
            .find(|s| s.locator.scope == McpScope::User && s.locator.name == "shared")
            .unwrap();
        assert_eq!(
            user_shared.effective_state,
            McpEffectiveState::Effective,
            "停用 Project 后 User 应恢复生效"
        );
        assert_eq!(user_shared.shadowed_context_count, 0);
    }

    #[test]
    fn backup_rotation_keeps_at_most_five() {
        let (paths, _t) = setup();
        for i in 0..7 {
            write_main(&paths, json!({"i": i}));
        }
        let bak_dir = paths
            .backup_dir()
            .join(source_hash(&paths.main_claude_json()));
        let count = fs::read_dir(&bak_dir).map(|rd| rd.count()).unwrap_or(0);
        assert!(count <= 5, "备份份数 {count} 超过 5");
    }

    #[test]
    fn same_second_multiple_backups_keep_distinct() {
        // 同一秒多次写入：纳秒+计数保证备份名唯一，不互相覆盖
        let (paths, _t) = setup();
        for i in 0..6 {
            write_main(&paths, json!({"i": i, "pad": "_______________________"}));
        }
        let bak_dir = paths
            .backup_dir()
            .join(source_hash(&paths.main_claude_json()));
        let count = fs::read_dir(&bak_dir).map(|rd| rd.count()).unwrap_or(0);
        assert_eq!(count, 5, "同秒多次写入应保留 5 份不同版本，实际 {count}");
    }

    #[test]
    fn batch_save_same_source_keeps_all() {
        // 同一 source（user）批量保存多个服务：全部保留
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let action = McpChangeAction::BatchSave {
            items: vec![
                McpSaveItem {
                    target: user_locator("a"),
                    config: map_of(json!({"command":"node"})),
                    overwrite: false,
                },
                McpSaveItem {
                    target: user_locator("b"),
                    config: map_of(json!({"command":"go"})),
                    overwrite: false,
                },
                McpSaveItem {
                    target: user_locator("c"),
                    config: map_of(json!({"command":"python"})),
                    overwrite: false,
                },
            ],
        };
        apply_action_files(&paths, &[], &action).unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(doc["mcpServers"].get("a").is_some());
        assert!(doc["mcpServers"].get("b").is_some());
        assert!(doc["mcpServers"].get("c").is_some());
    }

    #[test]
    fn same_file_rename_original_disappears() {
        // 同文件重命名：old 消失，new 出现
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"old":{"command":"node"}}}));
        let original = user_locator("old");
        let target = McpLocator {
            scope: McpScope::User,
            name: "new".into(),
            instance_id: None,
            project_path: None,
        };
        let action = McpChangeAction::Save {
            original: Some(original),
            target,
            config: map_of(json!({"command":"node"})),
            overwrite: false,
        };
        apply_action_files(&paths, &[], &action).unwrap();
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(doc["mcpServers"].get("old").is_none(), "old 应已删除");
        assert!(doc["mcpServers"].get("new").is_some());
    }

    #[test]
    fn local_cross_project_move_no_residue() {
        // 同一实例文件内跨 Local 项目移动：旧项目下不残留
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let inst = "alpha";
        let proj1 = paths.home.join("p1");
        let proj2 = paths.home.join("p2");
        fs::create_dir_all(&proj1).unwrap();
        fs::create_dir_all(&proj2).unwrap();
        register_project(&paths, &proj1.display().to_string()).unwrap();
        register_project(&paths, &proj2.display().to_string()).unwrap();
        let p = paths.instance_claude_json(inst).unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_json_transactional(&paths, &p, &json!({})).unwrap();
        // 先在 p1 建 local
        let t1 = McpLocator {
            scope: McpScope::Local,
            name: "mv".into(),
            instance_id: Some(inst.into()),
            project_path: Some(proj1.display().to_string()),
        };
        apply_action_files(
            &paths,
            &[inst.into()],
            &save_action(t1.clone(), map_of(json!({"command":"node"}))),
        )
        .unwrap();
        // 移动到 p2（original=t1，target=p2）
        let t2 = McpLocator {
            scope: McpScope::Local,
            name: "mv".into(),
            instance_id: Some(inst.into()),
            project_path: Some(proj2.display().to_string()),
        };
        let action = McpChangeAction::Save {
            original: Some(t1),
            target: t2,
            config: map_of(json!({"command":"node"})),
            overwrite: false,
        };
        apply_action_files(&paths, &[inst.into()], &action).unwrap();
        let doc: Value = serde_json::from_str(
            &fs::read_to_string(paths.instance_claude_json(inst).unwrap()).unwrap(),
        )
        .unwrap();
        let projs = doc["projects"].as_object().unwrap();
        let p1_key = projs
            .keys()
            .find(|k| norm_path(k) == norm_path(&proj1.display().to_string()))
            .unwrap();
        let p2_key = projs
            .keys()
            .find(|k| norm_path(k) == norm_path(&proj2.display().to_string()))
            .unwrap();
        assert!(
            projs[p1_key]["mcpServers"].get("mv").is_none(),
            "p1 下不应残留"
        );
        assert!(projs[p2_key]["mcpServers"]["mv"]["command"] == "node");
    }

    #[test]
    fn local_disable_enable_roundtrip() {
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let proj = paths.home.join("lp");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let inst = "alpha";
        let p = paths.instance_claude_json(inst).unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_json_transactional(&paths, &p, &json!({})).unwrap();
        let target = McpLocator {
            scope: McpScope::Local,
            name: "lsrv".into(),
            instance_id: Some(inst.into()),
            project_path: Some(proj.display().to_string()),
        };
        apply_action_files(
            &paths,
            &[inst.into()],
            &save_action(
                target.clone(),
                map_of(json!({"command":"node","env":{"TOKEN":"x"}})),
            ),
        )
        .unwrap();
        apply_action_files(
            &paths,
            &[inst.into()],
            &McpChangeAction::SetEnabled {
                target: target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        // 来源移除、仓库有条目
        let doc: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        let key = doc["projects"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        assert!(doc["projects"][&key]["mcpServers"].get("lsrv").is_none());
        let (store, _) = read_disabled(&paths);
        assert!(store
            .entries
            .iter()
            .any(|e| e.name == "lsrv" && e.config.get("env").is_some()));
        // 恢复
        apply_action_files(
            &paths,
            &[inst.into()],
            &McpChangeAction::SetEnabled {
                target,
                enabled: true,
            },
        )
        .unwrap();
        let doc2: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(
            doc2["projects"][&key]["mcpServers"]["lsrv"]["command"],
            "node"
        );
    }

    #[test]
    fn unparseable_existing_file_rejects_overwrite() {
        let (paths, _t) = setup();
        // 主配置损坏：写操作必须拒绝，且不覆盖
        fs::write(paths.main_claude_json(), "not json").unwrap();
        let target = user_locator("x");
        let res = apply_action_files(
            &paths,
            &[],
            &save_action(target, map_of(json!({"command":"node"}))),
        );
        assert!(res.is_err(), "损坏文件应拒绝覆盖");
        // 内容未被覆盖
        assert_eq!(
            fs::read_to_string(paths.main_claude_json()).unwrap(),
            "not json"
        );
        // list 应报告 issue
        let state = collect_state(&paths, &[]);
        assert!(state
            .issues
            .iter()
            .any(|i| i.source_id == source_id::user(MAIN_INSTANCE)));
    }

    #[test]
    fn unregistered_project_locator_rejected() {
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let proj = paths.home.join("unregistered");
        fs::create_dir_all(&proj).unwrap();
        // 不登记，直接 Save Project
        let target = McpLocator {
            scope: McpScope::Project,
            name: "psrv".into(),
            instance_id: None,
            project_path: Some(proj.display().to_string()),
        };
        let res = apply_action_files(
            &paths,
            &[],
            &save_action(target, map_of(json!({"command":"node"}))),
        );
        assert!(res.is_err(), "未登记项目应被拒绝");
        assert!(!proj.join(".mcp.json").exists(), "不应写入任何文件");
    }

    #[test]
    fn illegal_locator_rejected() {
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        // User 带 instance
        let bad = McpLocator {
            scope: McpScope::User,
            name: "x".into(),
            instance_id: Some("alpha".into()),
            project_path: None,
        };
        assert!(apply_action_files(
            &paths,
            &["alpha".to_string()],
            &save_action(bad, map_of(json!({"command":"node"})))
        )
        .is_err());
        // 保留名
        let reserved = user_locator("workspace");
        assert!(apply_action_files(
            &paths,
            &[],
            &save_action(reserved, map_of(json!({"command":"node"})))
        )
        .is_err());
        // 非法名
        let illegal = user_locator("bad name");
        assert!(apply_action_files(
            &paths,
            &[],
            &save_action(illegal, map_of(json!({"command":"node"})))
        )
        .is_err());
        // schema 错误：stdio 空 command
        let badcfg = user_locator("okname");
        assert!(apply_action_files(
            &paths,
            &[],
            &save_action(badcfg, map_of(json!({"type":"stdio","command":""})))
        )
        .is_err());
        // URL 缺 type
        let urlnotype = user_locator("okname2");
        assert!(apply_action_files(
            &paths,
            &[],
            &save_action(urlnotype, map_of(json!({"url":"https://e.com"})))
        )
        .is_err());
    }

    #[test]
    fn legacy_illegal_name_kept_only_when_unchanged() {
        // 旧非法名：仅 original==target 时允许编辑；改名则新名必须合法
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers":{"bad name":{"command":"node"}}}),
        );
        let orig = McpLocator {
            scope: McpScope::User,
            name: "bad name".into(),
            instance_id: None,
            project_path: None,
        };
        // 改 config 但不改名 → 允许
        let keep = McpLocator {
            scope: McpScope::User,
            name: "bad name".into(),
            instance_id: None,
            project_path: None,
        };
        let action = McpChangeAction::Save {
            original: Some(orig.clone()),
            target: keep,
            config: map_of(json!({"command":"go"})),
            overwrite: false,
        };
        assert!(
            apply_action_files(&paths, &[], &action).is_ok(),
            "未改名编辑应允许保留旧非法名"
        );
        // 改名为另一个非法名 → 拒绝
        let still_bad = McpLocator {
            scope: McpScope::User,
            name: "still bad".into(),
            instance_id: None,
            project_path: None,
        };
        let action2 = McpChangeAction::Save {
            original: Some(orig),
            target: still_bad,
            config: map_of(json!({"command":"go"})),
            overwrite: false,
        };
        assert!(
            apply_action_files(&paths, &[], &action2).is_err(),
            "改名后新名仍非法应拒绝"
        );
    }

    #[test]
    fn snapshot_read_failure_zero_writes() {
        // 快照读取失败发生在首次写入之前：第一目标为已有普通文件，后续目标必须在 fs::read
        // 阶段确定失败（目录当作文件读 → 非 NotFound 错误），事务在任何写入前终止。
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"a":{"command":"node"}}}));
        let main_bytes = fs::read(paths.main_claude_json()).unwrap();
        // 目录作为 PlannedWrite 目标：fs::read 目录确定失败（非 NotFound），触发快照阶段终止
        let dir_target = paths.home.join("a-real-dir");
        fs::create_dir_all(&dir_target).unwrap();
        let writes = vec![
            PlannedWrite {
                path: paths.main_claude_json(),
                value: json!({"mcpServers":{"a":{"command":"CHANGED"}}}),
            },
            PlannedWrite {
                path: dir_target.clone(),
                value: json!({}),
            },
        ];
        let res = apply_storage_transaction(&paths, writes);
        assert!(res.is_err(), "快照读取失败应在任何写入前终止");
        // 第一目标原始 bytes 完全不变（连临时文件都不应产生）
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            main_bytes,
            "第一目标原始 bytes 完全不变"
        );
    }

    #[test]
    fn user_delete_not_revived_after_sync() {
        // User 删除后，即便实例副本仍停留旧状态，主账户为空，下次同步不会从空主账户复活已删项
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"rm":{"command":"node"}}}));
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::Delete {
                target: user_locator("rm"),
            },
        )
        .unwrap();
        // 主账户已删
        let main: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(main["mcpServers"].get("rm").is_none());
        // 模拟一个仍带 rm 的实例副本，跑一次同步收敛应把它也删除（不复活）
        let inst = "alpha";
        let p = paths.instance_claude_json(inst).unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_json_transactional(&paths, &p, &json!({"mcpServers":{"rm":{"command":"node"}}}))
            .unwrap();
        // 注：sync_configs_locked 依赖真实 snapshot；这里只验证主账户不会被空实例复活：
        // 主账户仍是删除后状态。
        let main2: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(
            main2["mcpServers"].get("rm").is_none(),
            "主账户删除后不应被实例副本复活"
        );
    }

    #[test]
    fn edit_disabled_service_stays_disabled() {
        // 编辑停用服务：保持停用，只更新 disabled entry，不写入启用来源
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"d":{"command":"node"}}}));
        let target = user_locator("d");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let action = McpChangeAction::Save {
            original: Some(target.clone()),
            target: target.clone(),
            config: map_of(json!({"command":"go"})),
            overwrite: false,
        };
        apply_action_files(&paths, &[], &action).unwrap();
        // 仍未启用
        let main: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(
            main["mcpServers"].get("d").is_none(),
            "编辑停用服务不应意外启用"
        );
        // 仓库中配置已更新
        let (store, _) = read_disabled(&paths);
        let e = store.entries.iter().find(|e| e.name == "d").unwrap();
        assert_eq!(e.config["command"], "go");
    }

    #[test]
    fn disable_nonexistent_service_errors() {
        // User/Local 停用找不到启用配置必须报错，禁止创建幽灵停用项
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: user_locator("ghost"),
                enabled: false,
            },
        );
        assert!(res.is_err(), "停用不存在的服务应报错");
        let (store, _) = read_disabled(&paths);
        assert!(store.entries.is_empty(), "不应创建幽灵停用项");
    }

    #[test]
    fn project_disable_missing_service_errors() {
        // Project 停用前确认 .mcp.json 中确实存在该服务
        let (paths, _t) = setup();
        let proj = paths.home.join("pdm");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: proj_target(&proj.display().to_string(), "nope"),
                enabled: false,
            },
        );
        assert!(res.is_err(), "停用 .mcp.json 中不存在的服务应报错");
    }

    #[test]
    fn project_only_in_secondary_instance_toggle() {
        // 项目仅由次级实例 .claude.json.projects 发现（未登记）时，传入该实例 Project 启停成功
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let inst = "alpha";
        let proj = paths.home.join("sec-proj");
        fs::create_dir_all(&proj).unwrap();
        // 不 register_project，仅在 alpha 的 projects 里登记
        let p = paths.instance_claude_json(inst).unwrap();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_json_transactional(
            &paths,
            &p,
            &json!({"projects": { proj.display().to_string(): {} }}),
        )
        .unwrap();
        let target = proj_target(&proj.display().to_string(), "psrv");
        apply_action_files(
            &paths,
            &[inst.into()],
            &save_action(target.clone(), map_of(json!({"command":"node"}))),
        )
        .unwrap();
        apply_action_files(
            &paths,
            &[inst.into()],
            &McpChangeAction::SetEnabled {
                target,
                enabled: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn malformed_disabled_store_not_overwritten() {
        // 合法 JSON 对象但字段结构错误：启停操作必须报错且不覆盖
        let (paths, _t) = setup();
        fs::write(
            paths.disabled_store(),
            json!({"version":"not-a-number","entries":"oops"}).to_string(),
        )
        .unwrap();
        write_main(&paths, json!({"mcpServers":{"d":{"command":"node"}}}));
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: user_locator("d"),
                enabled: false,
            },
        );
        assert!(res.is_err(), "损坏的 disabled store 应拒绝操作");
        let s = fs::read_to_string(paths.disabled_store()).unwrap();
        assert!(s.contains("not-a-number"), "原内容不应被覆盖");
    }

    #[test]
    fn malformed_disabled_mcpjson_servers_reports_issue() {
        // settings.local.json 的 disabledMcpjsonServers 非数组 → collect_state 报 issue，
        // 且 issue sourceId 必须是 project-settings:<canonical>，不能是完整 settings 文件路径
        let (paths, _t) = setup();
        let proj = paths.home.join("pmm");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        write_json_transactional(
            &paths,
            &proj.join(".mcp.json"),
            &json!({"mcpServers":{"s":{"command":"node"}}}),
        )
        .unwrap();
        let settings = proj.join(".claude").join("settings.local.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(
            &settings,
            json!({"disabledMcpjsonServers":"not-array"}).to_string(),
        )
        .unwrap();
        let state = collect_state(&paths, &[]);
        let issue = state
            .issues
            .iter()
            .find(|i| i.source_id.starts_with("project-settings:"))
            .expect("应有 project-settings issue");
        assert!(
            !issue.source_id.contains("settings.local.json"),
            "sourceId 不能拼入 settings 文件路径"
        );
        assert!(issue.detail.contains("不是数组"));
    }

    #[test]
    fn rollback_recovery_when_target_missing() {
        // 模拟：target→rollback 后、tmp→target 前进程中断（target 缺失、rollback 在）
        // 下次写入应先恢复 rollback 并终止，要求刷新后重试
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"a":{"command":"node"}}}));
        let target = paths.main_claude_json();
        let rollback = target.with_file_name(format!(".ccm-rollback-{}", source_hash(&target)));
        fs::rename(&target, &rollback).unwrap();
        assert!(!target.exists());
        assert!(rollback.exists());
        let res = write_json_transactional(
            &paths,
            &target,
            &json!({"mcpServers":{"b":{"command":"go"}}}),
        );
        assert!(res.is_err(), "应检测到 rollback 并终止当前操作");
        // rollback 已恢复为 target，内容为上次有效配置
        let restored: Value = serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(restored["mcpServers"]["a"]["command"], "node");
        assert!(!rollback.exists(), "rollback 应已被恢复移走");
    }

    #[test]
    fn forged_original_with_illegal_name_rejected() {
        // 空配置环境：伪造 original=target="bad name"（实际不存在）必须拒绝
        let (paths, _t) = setup();
        write_main(&paths, json!({}));
        let bad = McpLocator {
            scope: McpScope::User,
            name: "bad name".into(),
            instance_id: None,
            project_path: None,
        };
        let action = McpChangeAction::Save {
            original: Some(bad.clone()),
            target: bad,
            config: map_of(json!({"command":"node"})),
            overwrite: false,
        };
        assert!(
            apply_action_files(&paths, &[], &action).is_err(),
            "伪造的不存在 original + 非法名必须拒绝"
        );
    }

    #[test]
    fn legacy_illegal_name_real_history_edit_succeeds() {
        // 真实存在的非法名历史服务，不改名编辑应成功
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers":{"bad name":{"command":"node"}}}),
        );
        let bad = McpLocator {
            scope: McpScope::User,
            name: "bad name".into(),
            instance_id: None,
            project_path: None,
        };
        let action = McpChangeAction::Save {
            original: Some(bad.clone()),
            target: bad,
            config: map_of(json!({"command":"go"})),
            overwrite: false,
        };
        assert!(
            apply_action_files(&paths, &[], &action).is_ok(),
            "真实历史服务不改名编辑应成功"
        );
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["bad name"]["command"], "go");
    }

    #[test]
    fn type_wrong_mcp_servers_rejected_and_bytes_unchanged() {
        // mcpServers 存在但不是对象：写盘必须 Err 且源文件 bytes 完全不变
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers":["not","an","object"], "other":"keep"}),
        );
        let bytes = fs::read(paths.main_claude_json()).unwrap();
        let action = save_action(user_locator("new"), map_of(json!({"command":"go"})));
        assert!(
            apply_action_files(&paths, &[], &action).is_err(),
            "mcpServers 类型错误应拒绝写盘"
        );
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            bytes,
            "源文件原始 bytes 不应变化"
        );
        let state = collect_state(&paths, &[]);
        assert!(state
            .issues
            .iter()
            .any(|i| i.source_id == source_id::user(MAIN_INSTANCE)
                && i.detail.contains("mcpServers")));
    }

    #[test]
    fn settings_disable_non_array_rejected() {
        // disabledMcpjsonServers 非数组：停用必须 Err 且 settings 内容不变
        let (paths, _t) = setup();
        let proj = paths.home.join("psd");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        write_json_transactional(
            &paths,
            &proj.join(".mcp.json"),
            &json!({"mcpServers":{"s":{"command":"node"}}}),
        )
        .unwrap();
        let settings = proj.join(".claude").join("settings.local.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(
            &settings,
            json!({"disabledMcpjsonServers":"not-array"}).to_string(),
        )
        .unwrap();
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: proj_target(&proj.display().to_string(), "s"),
                enabled: false,
            },
        );
        assert!(res.is_err(), "disabledMcpjsonServers 非数组应拒绝停用");
        assert!(fs::read_to_string(&settings).unwrap().contains("not-array"));
    }

    #[test]
    fn multi_file_rollback_deterministic() {
        // 确定性多文件事务：第一份写入成功、第二份失败 → 第一份回滚、第二份未创建
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"a":{"command":"node"}}}));
        let blocker = paths.home.join("blocker-file");
        fs::write(&blocker, "x").unwrap(); // blocker 是文件，其下子目录无法创建
        let bad_target = blocker.join("deep").join("x.json");
        let writes = vec![
            PlannedWrite {
                path: paths.main_claude_json(),
                value: json!({"mcpServers":{"a":{"command":"CHANGED"}}}),
            },
            PlannedWrite {
                path: bad_target.clone(),
                value: json!({}),
            },
        ];
        let res = apply_storage_transaction(&paths, writes);
        assert!(res.is_err());
        let main: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert_eq!(
            main["mcpServers"]["a"]["command"], "node",
            "第一份应回滚为原值"
        );
        assert!(!bad_target.exists(), "第二份不应被创建");
    }

    #[test]
    fn transaction_rechecks_expected_revision_before_any_write() {
        let (paths, _t) = setup();
        let target = paths.home.join("revision-race.json");
        fs::write(&target, b"{\"state\":\"preview\"}").unwrap();
        let preview_revision = revision(&target);
        fs::write(&target, b"{\"state\":\"external\"}").unwrap();
        let external_bytes = fs::read(&target).unwrap();

        let writes = vec![PlannedWrite {
            path: target.clone(),
            value: json!({"state":"manager"}),
        }];
        let mut source_ids = BTreeMap::new();
        source_ids.insert(target.clone(), vec!["project:race".to_string()]);
        let mut expected = BTreeMap::new();
        expected.insert("project:race".to_string(), preview_revision);

        let result =
            apply_storage_transaction_ordered(&paths, writes, &source_ids, Some(&expected));
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), external_bytes);
    }

    #[test]
    fn planned_writes_use_stable_source_id_order() {
        let (paths, _t) = setup();
        let first_path = paths.home.join("first.json");
        let second_path = paths.home.join("second.json");
        let writes = vec![
            PlannedWrite {
                path: first_path.clone(),
                value: json!({"n":1}),
            },
            PlannedWrite {
                path: second_path.clone(),
                value: json!({"n":2}),
            },
        ];
        let mut source_ids = BTreeMap::new();
        source_ids.insert(first_path.clone(), vec!["user:z".to_string()]);
        source_ids.insert(second_path.clone(), vec!["manager:disabled".to_string()]);

        let ordered = sort_writes_by_source_id(writes, &source_ids);
        assert_eq!(ordered[0].path, second_path);
        assert_eq!(ordered[1].path, first_path);
    }

    #[test]
    fn affected_normal_save_excludes_disabled() {
        // 普通启用保存：affected 不含 manager:disabled
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"a":{"command":"node"}}}));
        let action = save_action(user_locator("a"), map_of(json!({"command":"go"})));
        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert!(affected
            .iter()
            .any(|s| s == &source_id::user(MAIN_INSTANCE)));
        assert!(
            !affected.contains(&source_id::DISABLED.to_string()),
            "普通启用保存不应包含 manager:disabled"
        );
    }

    #[test]
    fn affected_disabled_edit_includes_disabled() {
        // 编辑停用定义：既校验 locator 的直接 source，也校验 manager:disabled
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"d":{"command":"node"}}}));
        let target = user_locator("d");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let action = McpChangeAction::Save {
            original: Some(target.clone()),
            target,
            config: map_of(json!({"command":"go"})),
            overwrite: false,
        };
        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert!(
            affected.contains(&source_id::DISABLED.to_string()),
            "停用服务编辑应包含 manager:disabled"
        );
        assert!(
            affected.contains(&source_id::user(MAIN_INSTANCE)),
            "停用编辑仍应校验 User source revision"
        );
    }

    #[test]
    fn version_2_registry_rejected() {
        let (paths, _t) = setup();
        fs::write(
            paths.project_registry(),
            json!({"version": 2, "projects": []}).to_string(),
        )
        .unwrap();
        let (_, issue) = read_registry(&paths);
        assert!(issue.is_some(), "version=2 的 registry 必须拒绝");
        let (reg, issue) = read_registry(&paths);
        let _ = (reg, issue);
        assert!(read_registry(&paths).1.is_some());
    }

    #[test]
    fn missing_version_registry_rejected() {
        let (paths, _t) = setup();
        fs::write(
            paths.project_registry(),
            json!({"projects": []}).to_string(),
        )
        .unwrap();
        assert!(
            read_registry(&paths).1.is_some(),
            "缺失 version 的 registry 必须拒绝"
        );
    }

    #[test]
    fn version_2_disabled_rejected() {
        let (paths, _t) = setup();
        fs::write(
            paths.disabled_store(),
            json!({"version": 2, "entries": []}).to_string(),
        )
        .unwrap();
        assert!(
            read_disabled(&paths).1.is_some(),
            "version=2 的 disabled store 必须拒绝"
        );
        // apply 操作也拒绝
        write_main(&paths, json!({"mcpServers":{"d":{"command":"node"}}}));
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: user_locator("d"),
                enabled: false,
            },
        );
        assert!(res.is_err());
    }

    #[test]
    fn project_enable_missing_service_errors() {
        let (paths, _t) = setup();
        let proj = paths.home.join("project-enable-missing");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        let target = proj_target(&proj.display().to_string(), "missing");
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target,
                enabled: true,
            },
        );
        assert!(res.is_err());
        assert!(!proj.join(".claude").join("settings.local.json").exists());
    }

    #[test]
    fn missing_version_disabled_rejected() {
        let (paths, _t) = setup();
        fs::write(paths.disabled_store(), json!({"entries": []}).to_string()).unwrap();
        assert!(
            read_disabled(&paths).1.is_some(),
            "缺失 version 的 disabled store 必须拒绝"
        );
    }

    #[test]
    fn non_object_service_member_issue() {
        // mcpServers[name] 非对象 → collect_state 报 issue 且 read_service_config 返回 Err
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers": {"bad": "not-an-object", "ok": {"command": "node"}}}),
        );
        let state = collect_state(&paths, &[]);
        // ok 服务出现，bad 产生 issue
        assert!(state.services.iter().any(|s| s.locator.name == "ok"));
        assert!(state.services.iter().all(|s| s.locator.name != "bad"));
        assert!(state
            .issues
            .iter()
            .any(|i| i.detail.contains("mcpServers") || i.detail.contains("不是对象")));
        // read_service_config for "bad" → Err
        let res = read_service_config(
            &paths,
            &[],
            &McpLocator {
                scope: McpScope::User,
                name: "bad".into(),
                instance_id: None,
                project_path: None,
            },
        );
        assert!(res.is_err(), "非对象成员应返回 Err");
    }

    #[test]
    fn corrupt_config_apply_rejected() {
        // enabled 配置文件损坏（解析失败）：apply 写盘路径应拒绝且原 bytes 不变
        let (paths, _t) = setup();
        fs::write(paths.main_claude_json(), "not json").unwrap();
        let bytes = fs::read(paths.main_claude_json()).unwrap();
        let target = user_locator("x");
        let res = apply_action_files(
            &paths,
            &[],
            &save_action(target, map_of(json!({"command":"node"}))),
        );
        assert!(res.is_err(), "损坏文件应拒绝写入");
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            bytes,
            "原 bytes 不变"
        );
        let state = collect_state(&paths, &[]);
        assert!(state
            .issues
            .iter()
            .any(|i| i.source_id == source_id::user(MAIN_INSTANCE)));
    }

    #[test]
    fn delete_non_object_member_errors_and_bytes_unchanged() {
        // mcpServers[name] 为非对象时，直接 Delete apply 必须 Err 且源文件 bytes 不变
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers":{"bad":"not-an-object"}, "keep":"v"}),
        );
        let bytes = fs::read(paths.main_claude_json()).unwrap();
        let res = apply_action_files(
            &paths,
            &[],
            &McpChangeAction::Delete {
                target: user_locator("bad"),
            },
        );
        assert!(res.is_err(), "删除非对象成员应报错");
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            bytes,
            "源文件 bytes 不变"
        );
    }

    #[test]
    fn move_from_non_object_member_errors_and_bytes_unchanged() {
        // original 为非对象成员时，移动/重命名 apply 必须 Err 且源文件 bytes 不变
        let (paths, _t) = setup();
        write_main(&paths, json!({"mcpServers":{"bad":"str"}}));
        let bytes = fs::read(paths.main_claude_json()).unwrap();
        let original = user_locator("bad");
        let target = user_locator("new");
        let action = McpChangeAction::Save {
            original: Some(original),
            target,
            config: map_of(json!({"command":"node"})),
            overwrite: false,
        };
        let res = apply_action_files(&paths, &[], &action);
        assert!(res.is_err(), "从非对象成员移动应报错");
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            bytes,
            "源文件 bytes 不变"
        );
    }

    #[test]
    fn corrupt_disabled_store_save_errors_no_enabled_write() {
        // disabled store 损坏：普通 Save 也必须 Err（无法判断 target 是否停用），且不写 enabled 来源
        let (paths, _t) = setup();
        fs::write(
            paths.disabled_store(),
            json!({"version":"not-a-number"}).to_string(),
        )
        .unwrap();
        write_main(&paths, json!({}));
        let main_bytes = fs::read(paths.main_claude_json()).unwrap();
        let res = apply_action_files(
            &paths,
            &[],
            &save_action(user_locator("new"), map_of(json!({"command":"node"}))),
        );
        assert!(res.is_err(), "disabled store 损坏时 Save 应报错");
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            main_bytes,
            "不应写入 enabled 来源"
        );
    }

    #[test]
    fn corrupt_disabled_store_batchsave_errors() {
        // disabled store 损坏：BatchSave 必须 Err，且不写任何 enabled 来源
        let (paths, _t) = setup();
        fs::write(
            paths.disabled_store(),
            json!({"version":"not-a-number"}).to_string(),
        )
        .unwrap();
        write_main(&paths, json!({}));
        let main_bytes = fs::read(paths.main_claude_json()).unwrap();
        let action = McpChangeAction::BatchSave {
            items: vec![McpSaveItem {
                target: user_locator("n"),
                config: map_of(json!({"command":"node"})),
                overwrite: false,
            }],
        };
        let res = apply_action_files(&paths, &[], &action);
        assert!(res.is_err(), "disabled store 损坏时 BatchSave 应报错");
        assert_eq!(
            fs::read(paths.main_claude_json()).unwrap(),
            main_bytes,
            "不应写入 enabled 来源"
        );
    }

    #[test]
    fn collect_state_reports_issue_for_missing_registered_project() {
        // 注册项目后删除目录：collect_state 必须为该登记路径生成来源问题，不静默跳过
        let (paths, _t) = setup();
        let proj = paths.home.join("gone");
        fs::create_dir_all(&proj).unwrap();
        register_project(&paths, &proj.display().to_string()).unwrap();
        fs::remove_dir_all(&proj).unwrap();
        let state = collect_state(&paths, &[]);
        assert!(
            state.issues.iter().any(|i| {
                i.source_id == source_id::PROJECTS && i.detail.contains("目录不存在或不可访问")
            }),
            "登记项目目录消失应生成 PROJECTS 来源问题"
        );
    }

    #[test]
    fn collect_state_reports_issue_for_non_object_projects() {
        // .claude.json 的 projects 不是对象：collect_state 必须生成来源问题
        let (paths, _t) = setup();
        write_main(&paths, json!({"projects": ["not", "an", "object"]}));
        let state = collect_state(&paths, &[]);
        assert!(
            state
                .issues
                .iter()
                .any(|i| i.source_id == source_id::user(MAIN_INSTANCE)
                    && i.detail.contains("projects 不是对象")),
            "projects 非对象应生成来源问题"
        );
    }

    #[test]
    fn collect_state_reports_issue_for_inaccessible_project_key() {
        // projects 中某个项目路径不可访问：collect_state 必须生成来源问题，不静默跳过
        let (paths, _t) = setup();
        write_main(&paths, json!({"projects": { "E:\\does\\not\\exist": {} }}));
        let state = collect_state(&paths, &[]);
        assert!(
            state
                .issues
                .iter()
                .any(|i| i.source_id == source_id::user(MAIN_INSTANCE)
                    && i.detail.contains("无法规范化")),
            "不可访问的项目键应生成来源问题"
        );
    }

    #[test]
    fn project_save_does_not_depend_on_disabled_store() {
        let (paths, _t) = setup();
        let project = paths.home.join("project-independent");
        fs::create_dir_all(&project).unwrap();
        register_project(&paths, &project.display().to_string()).unwrap();
        fs::write(
            paths.disabled_store(),
            json!({"version": "broken", "entries": []}).to_string(),
        )
        .unwrap();
        let disabled_bytes = fs::read(paths.disabled_store()).unwrap();
        let target = proj_target(&project.display().to_string(), "project-service");
        let action = save_action(target, map_of(json!({"command": "node"})));

        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert_eq!(
            affected,
            vec![source_id::project(&project.display().to_string())],
            "Project Save 只能影响 Project source"
        );
        apply_action_files(&paths, &[], &action).unwrap();

        let project_doc: Value =
            serde_json::from_str(&fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(
            project_doc["mcpServers"]["project-service"]["command"],
            "node"
        );
        assert_eq!(
            fs::read(paths.disabled_store()).unwrap(),
            disabled_bytes,
            "Project Save 不得读取后重写 disabled store"
        );
    }

    #[test]
    fn project_entry_in_disabled_store_reports_issue() {
        let (paths, _t) = setup();
        fs::write(
            paths.disabled_store(),
            json!({
                "version": 1,
                "entries": [{
                    "scope": "project",
                    "name": "illegal",
                    "instanceId": null,
                    "projectPath": paths.home.display().to_string(),
                    "config": {"command": "node"},
                    "disabledAt": 1
                }]
            })
            .to_string(),
        )
        .unwrap();

        let state = collect_state(&paths, &[]);
        assert!(state.issues.iter().any(|issue| {
            issue.source_id == source_id::DISABLED && issue.detail.contains("只允许 User/Local")
        }));
        assert!(state
            .services
            .iter()
            .all(|service| service.locator.name != "illegal"));
    }

    #[test]
    fn local_equivalent_project_key_is_reused() {
        let (paths, _t) = setup();
        let instance = "alpha";
        let project = paths.home.join("local-key-project");
        fs::create_dir_all(&project).unwrap();
        let raw_key = format!("{}{}.", project.display(), std::path::MAIN_SEPARATOR);
        let instance_path = paths.instance_claude_json(instance).unwrap();
        fs::create_dir_all(instance_path.parent().unwrap()).unwrap();
        let mut projects = Map::new();
        projects.insert(
            raw_key.clone(),
            json!({
                "mcpServers": {"svc": {"command": "node"}},
                "otherField": "keep"
            }),
        );
        write_json_transactional(
            &paths,
            &instance_path,
            &json!({"projects": Value::Object(projects)}),
        )
        .unwrap();

        let state = collect_state(&paths, &[instance.to_string()]);
        assert!(state.services.iter().any(|service| {
            service.locator.scope == McpScope::Local
                && service.locator.name == "svc"
                && service.locator.instance_id.as_deref() == Some(instance)
        }));

        let canonical = canonicalize_dir(&project.display().to_string()).unwrap();
        let target = McpLocator {
            scope: McpScope::Local,
            name: "svc".into(),
            instance_id: Some(instance.into()),
            project_path: Some(canonical.display().to_string()),
        };
        let action = McpChangeAction::Save {
            original: Some(target.clone()),
            target,
            config: map_of(json!({"command": "go"})),
            overwrite: false,
        };
        apply_action_files(&paths, &[instance.to_string()], &action).unwrap();

        let doc: Value = serde_json::from_str(&fs::read_to_string(instance_path).unwrap()).unwrap();
        let saved_projects = doc["projects"].as_object().unwrap();
        assert_eq!(saved_projects.len(), 1, "不得创建第二个 canonical 项目键");
        assert!(
            saved_projects.contains_key(&raw_key),
            "必须保留原始项目 key"
        );
        assert_eq!(
            saved_projects[&raw_key]["mcpServers"]["svc"]["command"],
            "go"
        );
        assert_eq!(saved_projects[&raw_key]["otherField"], "keep");
    }

    #[test]
    fn enabled_original_overwrites_disabled_target_without_residue() {
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers": {
                "enabled-a": {"command": "node"},
                "disabled-b": {"command": "python"}
            }}),
        );
        let disabled_target = user_locator("disabled-b");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: disabled_target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let action = McpChangeAction::Save {
            original: Some(user_locator("enabled-a")),
            target: disabled_target.clone(),
            config: map_of(json!({"command": "go"})),
            overwrite: true,
        };

        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert!(affected.contains(&source_id::user(MAIN_INSTANCE)));
        assert!(affected.contains(&source_id::DISABLED.to_string()));
        apply_action_files(&paths, &[], &action).unwrap();

        let main: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(main["mcpServers"].get("enabled-a").is_none());
        assert_eq!(main["mcpServers"]["disabled-b"]["command"], "go");
        let (disabled, issue) = read_disabled(&paths);
        assert!(issue.is_none());
        assert!(
            disabled
                .entries
                .iter()
                .all(|entry| !disabled_matches(entry, &disabled_target)),
            "最终 target 只能保留 enabled 定义"
        );
    }

    #[test]
    fn disabled_original_overwrites_enabled_target_without_residue() {
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers": {
                "disabled-a": {"command": "node"},
                "enabled-b": {"command": "python"}
            }}),
        );
        let disabled_original = user_locator("disabled-a");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: disabled_original.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let enabled_target = user_locator("enabled-b");
        let action = McpChangeAction::Save {
            original: Some(disabled_original.clone()),
            target: enabled_target.clone(),
            config: map_of(json!({"command": "go"})),
            overwrite: true,
        };

        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert!(affected.contains(&source_id::user(MAIN_INSTANCE)));
        assert!(affected.contains(&source_id::DISABLED.to_string()));
        apply_action_files(&paths, &[], &action).unwrap();

        let main: Value =
            serde_json::from_str(&fs::read_to_string(paths.main_claude_json()).unwrap()).unwrap();
        assert!(main["mcpServers"].get("disabled-a").is_none());
        assert!(main["mcpServers"].get("enabled-b").is_none());
        let (disabled, issue) = read_disabled(&paths);
        assert!(issue.is_none());
        assert!(disabled
            .entries
            .iter()
            .all(|entry| !disabled_matches(entry, &disabled_original)));
        let target_entry = disabled
            .entries
            .iter()
            .find(|entry| disabled_matches(entry, &enabled_target))
            .expect("最终 target 应保留 disabled 定义");
        assert_eq!(target_entry.config["command"], "go");
    }

    #[test]
    fn disabled_user_moves_to_disabled_project_state() {
        let (paths, _t) = setup();
        let project = paths.home.join("disabled-user-to-project");
        fs::create_dir_all(&project).unwrap();
        register_project(&paths, &project.display().to_string()).unwrap();
        write_main(
            &paths,
            json!({"mcpServers": {"move-me": {"command": "node"}}}),
        );
        let original = user_locator("move-me");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: original.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let target = proj_target(&project.display().to_string(), "moved");
        let action = McpChangeAction::Save {
            original: Some(original.clone()),
            target: target.clone(),
            config: map_of(json!({"command": "go"})),
            overwrite: false,
        };

        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert!(affected.contains(&source_id::DISABLED.to_string()));
        assert!(affected.contains(&source_id::project(&project.display().to_string())));
        assert!(affected.contains(&source_id::project_settings(&project.display().to_string())));
        apply_action_files(&paths, &[], &action).unwrap();

        let project_doc: Value =
            serde_json::from_str(&fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(project_doc["mcpServers"]["moved"]["command"], "go");
        let settings: Value = serde_json::from_str(
            &fs::read_to_string(project.join(".claude").join("settings.local.json")).unwrap(),
        )
        .unwrap();
        assert!(settings["disabledMcpjsonServers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some("moved")));
        let (disabled, issue) = read_disabled(&paths);
        assert!(issue.is_none());
        assert!(disabled
            .entries
            .iter()
            .all(|entry| !disabled_matches(entry, &original)));
    }

    #[test]
    fn disabled_project_moves_to_disabled_user_state() {
        let (paths, _t) = setup();
        let project = paths.home.join("disabled-project-to-user");
        fs::create_dir_all(&project).unwrap();
        register_project(&paths, &project.display().to_string()).unwrap();
        let original = proj_target(&project.display().to_string(), "move-me");
        apply_action_files(
            &paths,
            &[],
            &save_action(original.clone(), map_of(json!({"command": "node"}))),
        )
        .unwrap();
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: original.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let target = user_locator("moved");
        let action = McpChangeAction::Save {
            original: Some(original),
            target: target.clone(),
            config: map_of(json!({"command": "go"})),
            overwrite: false,
        };

        let affected = affected_source_ids(&paths, &[], &action).unwrap();
        assert!(affected.contains(&source_id::user(MAIN_INSTANCE)));
        assert!(affected.contains(&source_id::DISABLED.to_string()));
        assert!(affected.contains(&source_id::project(&project.display().to_string())));
        assert!(affected.contains(&source_id::project_settings(&project.display().to_string())));
        apply_action_files(&paths, &[], &action).unwrap();

        let project_doc: Value =
            serde_json::from_str(&fs::read_to_string(project.join(".mcp.json")).unwrap()).unwrap();
        assert!(project_doc["mcpServers"].get("move-me").is_none());
        if let Ok(text) = fs::read_to_string(paths.main_claude_json()) {
            let main: Value = serde_json::from_str(&text).unwrap();
            assert!(main
                .get("mcpServers")
                .and_then(Value::as_object)
                .and_then(|servers| servers.get("moved"))
                .is_none());
        }
        let (disabled, issue) = read_disabled(&paths);
        assert!(issue.is_none());
        let entry = disabled
            .entries
            .iter()
            .find(|entry| disabled_matches(entry, &target))
            .expect("目标 User 服务应保持停用");
        assert_eq!(entry.config["command"], "go");
    }

    #[test]
    fn batch_save_requires_overwrite_for_disabled_target() {
        let (paths, _t) = setup();
        write_main(
            &paths,
            json!({"mcpServers": {"disabled": {"command": "node"}}}),
        );
        let target = user_locator("disabled");
        apply_action_files(
            &paths,
            &[],
            &McpChangeAction::SetEnabled {
                target: target.clone(),
                enabled: false,
            },
        )
        .unwrap();
        let before = fs::read(paths.disabled_store()).unwrap();
        let action = McpChangeAction::BatchSave {
            items: vec![McpSaveItem {
                target,
                config: map_of(json!({"command": "go"})),
                overwrite: false,
            }],
        };
        assert!(apply_action_files(&paths, &[], &action).is_err());
        assert_eq!(fs::read(paths.disabled_store()).unwrap(), before);
    }
}
