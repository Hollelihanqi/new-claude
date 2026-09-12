//! 共享配置的**单向分发**引擎（决策 7.2，设计见
//! `docs/共享配置分发设计-2026-09-12.md`）。
//!
//! 背景：原先共享靠把 `~/.claude.json` 当**可写副本**与各环境做**双向合并**
//! （`sync.rs` 的 `sync_configs_locked`），于是应用会改写用户直接敲 `claude` 时用的
//! 那份配置。现在改为：应用独占一个共享源文件，**单向**分发给受管理环境，
//! 默认 Claude 完全退出这条链。
//!
//! 决策内核为纯函数，文件层负责严格读取、原子写入与失败回退。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// 两个域的字段名，同时也是台账里的域名。
pub(crate) const FIELD_MCP: &str = "mcpServers";
pub(crate) const FIELD_PLUGINS: &str = "enabledPlugins";

/// 台账格式版本。将来换算法时靠它识别旧格式。
pub(crate) const LEDGER_VERSION: u32 = 1;

/// 分发台账：记录**本应用上次分发给每个环境**的条目值。
///
/// 为什么必须有它：判定"环境里这条是它自己的配置，还是我们发下去的"，
/// **不能只看"目标里已经有这个名称"**。只有记住上次发下去的是什么，
/// 才能把「我们发过、用户没动」「我们发过、用户改过」「从来没发过」三件事分开。
/// （这与 P0-B#13 是同一类缺陷：缺少"上次确认状态"就无法区分
/// 「我们没写成功」与「用户真的改了」。）
#[derive(Default, Debug, Serialize, Deserialize)]
pub(crate) struct Ledger {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub domains: BTreeMap<String, DomainLedger>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub(crate) struct DomainLedger {
    /// 环境名 → (条目名 → 上次分发给它的值)
    #[serde(default)]
    pub envs: BTreeMap<String, Map<String, Value>>,
    #[serde(default)]
    pub explicit_overrides: BTreeMap<String, Vec<String>>,
}

impl Ledger {
    /// 读出来的版本号对不上时按空台账处理（宁可重判一轮，也不用错格式的旧记录）。
    pub(crate) fn normalized(mut self) -> Self {
        if self.version != LEDGER_VERSION {
            self.domains.clear();
        }
        self.version = LEDGER_VERSION;
        self
    }

    pub(crate) fn entries(&self, domain: &str, env: &str) -> Map<String, Value> {
        self.domains
            .get(domain)
            .and_then(|d| d.envs.get(env))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn set_entries(&mut self, domain: &str, env: &str, entries: Map<String, Value>) {
        let d = self.domains.entry(domain.to_string()).or_default();
        if entries.is_empty() {
            d.envs.remove(env);
        } else {
            d.envs.insert(env.to_string(), entries);
        }
    }

    fn mark_override(&mut self, field: &str, env: &str, name: &str, enabled: bool) {
        let domain = self.domains.entry(field.into()).or_default();
        let names = domain.explicit_overrides.entry(env.into()).or_default();
        names.retain(|n| n != name);
        if enabled {
            names.push(name.into());
        }
        if names.is_empty() {
            domain.explicit_overrides.remove(env);
        }
    }

    fn plan(
        &self,
        field: &str,
        env_name: &str,
        shared: &Map<String, Value>,
        env: &Map<String, Value>,
    ) -> EnvPlan {
        let mut plan = plan_env(shared, env, &self.entries(field, env_name));
        if let Some(names) = self
            .domains
            .get(field)
            .and_then(|d| d.explicit_overrides.get(env_name))
        {
            for name in names {
                plan.writes.remove(name);
                plan.removals.retain(|n| n != name);
                plan.in_sync.retain(|n| n != name);
                plan.overrides.retain(|o| &o.name != name);
                plan.overrides.push(OverrideInfo {
                    name: name.clone(),
                    reason: OverrideReason::Independent,
                    shared_value: shared.get(name).cloned(),
                    env_value: env.get(name).cloned(),
                });
            }
        }
        plan
    }
}

/// 环境值为什么胜出（= 为什么没被共享值覆盖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverrideReason {
    /// 我们发过这一条，但之后用户改过它
    ModifiedAfterDistribution,
    /// 台账里没有记录 —— 这是环境自己的配置，只是与共享项重名
    Independent,
    /// 我们发过，环境侧后来把它删了（删除也是用户的表态，不复活）
    DeletedByUser,
}

impl OverrideReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            OverrideReason::ModifiedAfterDistribution => "已覆盖共享配置（你修改过它）",
            OverrideReason::Independent => "已覆盖共享配置",
            OverrideReason::DeletedByUser => "已覆盖共享配置（你在该环境删除了它）",
        }
    }
}

/// 单个条目该做什么。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    /// 共享源里没有、环境里也没有 ⇒ 什么都不用做
    Leave,
    /// 写进环境（新增，或把共享源的新值更新下去）
    Write(Value),
    /// 从环境删掉（共享项被删除，且这条是我们分发下去的）
    Remove,
    /// 环境的值已经等于共享值，不冲突（来源标识 = 共享）
    InSync,
    /// 环境覆盖：保留环境值，界面报冲突
    Override(OverrideReason),
}

/// **单向分发的核心判定**：给定同一个条目名在 共享源 / 环境文件 / 台账 里的三个值，
/// 决定该怎么办。设计文档 §三 的表格就是本函数的全部规则。
pub(crate) fn decide(
    shared: Option<&Value>,
    env: Option<&Value>,
    ledger: Option<&Value>,
) -> Decision {
    match (shared, env) {
        // 共享源有这一条
        (Some(shared), Some(env)) => {
            if env == shared {
                // 同名同内容：不报冲突，方向是"来自共享"
                return Decision::InSync;
            }
            match ledger {
                // 我们发过、用户没动 → 跟随共享源更新
                Some(ledger) if ledger == env => Decision::Write(shared.clone()),
                // 我们发过、但当前值与台账不符 → 用户改过它
                Some(_) => Decision::Override(OverrideReason::ModifiedAfterDistribution),
                // 台账没记录 → 环境自己的配置与共享项重名
                None => Decision::Override(OverrideReason::Independent),
            }
        }
        // 共享源有、环境没有
        (Some(shared), None) => match ledger {
            // 我们发过、后来被删了 → 删除也是用户的表态，不复活
            Some(_) => Decision::Override(OverrideReason::DeletedByUser),
            // 从没发过 → 新增
            None => Decision::Write(shared.clone()),
        },
        // 共享源没有、环境有
        (None, Some(env)) => match ledger {
            // 是我们分发下去的 → 共享项被删了，清掉这条继承值
            Some(previous) if previous == env => Decision::Remove,
            Some(_) => Decision::Override(OverrideReason::ModifiedAfterDistribution),
            // 环境自己的配置 → 不动
            None => Decision::Leave,
        },
        // 两边都没有
        (None, None) => Decision::Leave,
    }
}

/// 环境覆盖的报告条目（给界面用）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OverrideInfo {
    pub name: String,
    pub reason: OverrideReason,
    /// 共享源里这条的值；用于"查看两边差异"。共享源没有该条时为 None。
    pub shared_value: Option<Value>,
    /// 环境里这条的值；环境已删除该条时为 None。
    pub env_value: Option<Value>,
}

/// 一个域在一个环境上的完整分发计划。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct EnvPlan {
    /// 要写进环境的值（新增 + 更新）
    pub writes: BTreeMap<String, Value>,
    /// 要从环境删掉的条目名
    pub removals: Vec<String>,
    /// 环境覆盖（保留环境值，界面报出来）
    pub overrides: Vec<OverrideInfo>,
    /// 与共享值一致的条目名（来源标识 = 共享）
    pub in_sync: Vec<String>,
}

impl EnvPlan {
    pub(crate) fn is_noop(&self) -> bool {
        self.writes.is_empty() && self.removals.is_empty()
    }
}

/// 对**一个环境的一个域**算出完整计划。
///
/// 条目名的全集 = 共享源 ∪ 环境 ∪ 台账 —— 只算前两者会漏掉
/// "共享项已删、环境里也没了，但台账还留着记录"这种需要清理的情形。
pub(crate) fn plan_env(
    shared: &Map<String, Value>,
    env: &Map<String, Value>,
    ledger: &Map<String, Value>,
) -> EnvPlan {
    let mut names: Vec<&String> = shared
        .keys()
        .chain(env.keys())
        .chain(ledger.keys())
        .collect();
    names.sort();
    names.dedup();

    let mut plan = EnvPlan::default();
    for name in names {
        let s = shared.get(name);
        let e = env.get(name);
        let l = ledger.get(name);
        match decide(s, e, l) {
            Decision::Leave => {}
            Decision::Write(v) => {
                plan.writes.insert(name.clone(), v);
            }
            Decision::Remove => plan.removals.push(name.clone()),
            Decision::InSync => plan.in_sync.push(name.clone()),
            Decision::Override(reason) => plan.overrides.push(OverrideInfo {
                name: name.clone(),
                reason,
                shared_value: s.cloned(),
                env_value: e.cloned(),
            }),
        }
    }
    plan
}

/// 按计划改写环境文档：**只动该域的、被计划点名的那些条目**，
/// 文档其余部分（登录态、permissions、用户自己的其他插件…）原样保留。
///
/// 返回该域**新的**台账条目（即"这次我们发下去的是什么"）。
pub(crate) fn apply_plan(
    doc: &mut Value,
    field: &str,
    plan: &EnvPlan,
    ledger_before: &Map<String, Value>,
) -> Map<String, Value> {
    let obj = doc.as_object_mut().expect("调用方保证传入对象");
    let entry = obj
        .entry(field.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        // 该字段存在但不是对象（文件被改坏）：整体重建，避免写出非法结构。
        // 这不是"覆盖用户数据"——那个字段本来就不是合法形态。
        *entry = Value::Object(Map::new());
    }
    let map = entry.as_object_mut().expect("上面刚保证是对象");

    for name in &plan.removals {
        map.remove(name);
    }
    for (name, value) in &plan.writes {
        map.insert(name.clone(), value.clone());
    }

    // 台账：**按"我们当前认为分发过什么"重新构建**，而不是在旧台账上打补丁。
    // 这样三类残留会自然消失：共享项已删且环境侧也没了的记录、
    // 以及任何不再被本轮判定点名的记录。
    //
    // 保留记录的唯一情形：这是我们发过的，但现在不该动它 ——
    //   - ModifiedAfterDistribution：用户改过它，但"我们上次发下去的是什么"没变，
    //     丢掉记录会让下一轮把它误判成"环境自己的配置"，从此再也跟随不了共享源；
    //   - DeletedByUser：用户删过它，丢掉记录会让下一轮重新判定成"从没发过"并塞回去。
    // Independent（台账本就没有）与其余情形都不留记录。
    let mut next = Map::new();
    for info in &plan.overrides {
        if info.reason == OverrideReason::Independent {
            continue;
        }
        if let Some(value) = ledger_before.get(&info.name) {
            next.insert(info.name.clone(), value.clone());
        }
    }
    for (name, value) in &plan.writes {
        next.insert(name.clone(), value.clone());
    }
    for name in &plan.in_sync {
        if let Some(value) = map.get(name) {
            next.insert(name.clone(), value.clone());
        }
    }
    next
}

// ---------------- 文件层 ----------------
//
// 上面是纯决策；这一层只负责读写，且**只碰三个自己的文件 + 受管理环境的文件**。
// 默认 Claude 的 `~/.claude.json` / `~/.claude/settings.json` **只被读一次**（首次采集），
// 此后本模块的任何代码路径都不会再碰它们（设计 §四）。

use std::fs;
use std::path::{Path, PathBuf};

/// 应用自建的共享源目录。**与默认 Claude 的 `~/.claude` 无关**。
pub(crate) fn shared_dir() -> PathBuf {
    crate::cfg_dir().join("shared")
}

/// 共享源文件。两个域各占一个文件，与现状的落点一致
/// （`mcpServers` 在 `.claude.json`、`enabledPlugins` 在 `settings.json`）。
pub(crate) fn shared_path(field: &str) -> PathBuf {
    let name = if field == FIELD_MCP {
        "mcp.json"
    } else {
        "plugins.json"
    };
    shared_dir().join(name)
}

pub(crate) fn ledger_path() -> PathBuf {
    shared_dir().join("distribution.json")
}

/// 默认 Claude 的文件 —— **仅供首次采集使用**。
fn default_claude_path(field: &str) -> PathBuf {
    if field == FIELD_MCP {
        crate::home().join(".claude.json")
    } else {
        crate::home().join(".claude").join("settings.json")
    }
}

/// 受管理环境的落点：MCP 在 `.claude.json`、插件在 `settings.json`。
pub(crate) fn env_path(env: &str, field: &str) -> PathBuf {
    let dir = crate::sync::instance_dir(env);
    if field == FIELD_MCP {
        dir.join(".claude.json")
    } else {
        dir.join("settings.json")
    }
}

fn read_document(path: &Path) -> Result<Value, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(serde_json::json!({})),
        Err(e) => return Err(format!("读取 {} 失败：{e}", path.display())),
    };
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} 配置损坏，已中止：{e}", path.display()))?;
    if !doc.is_object() {
        return Err(format!("{} 顶层不是对象，已中止", path.display()));
    }
    Ok(doc)
}

fn read_field(path: &Path, field: &str) -> Result<Map<String, Value>, String> {
    let doc = read_document(path)?;
    match doc.get(field) {
        None => Ok(Map::new()),
        Some(value) => value
            .as_object()
            .cloned()
            .ok_or_else(|| format!("{} 的 {field} 不是对象，已中止", path.display())),
    }
}

pub(crate) fn load_ledger() -> Result<Ledger, String> {
    if !ledger_path().try_exists().map_err(|e| e.to_string())? {
        return Ok(Ledger::default().normalized());
    }
    let ledger: Ledger = serde_json::from_value(read_document(&ledger_path())?)
        .map_err(|e| format!("分发台账损坏：{e}"))?;
    if ledger.version != LEDGER_VERSION {
        return Err("分发台账版本不支持，已中止".into());
    }
    Ok(ledger)
}

pub(crate) fn save_ledger(ledger: &Ledger) -> Result<(), String> {
    fs::create_dir_all(shared_dir()).map_err(|e| format!("创建共享目录失败：{e}"))?;
    let value = serde_json::to_value(ledger).map_err(|e| e.to_string())?;
    crate::sync::write_json_atomic(&ledger_path(), &value)
        .map_err(|e| format!("写分发台账失败：{e}"))
}

/// 严格读取共享源，缺失或损坏时中止，避免误把读取故障当成删除指令。
pub(crate) fn load_shared(field: &str) -> Result<Map<String, Value>, String> {
    if !shared_path(field).try_exists().map_err(|e| e.to_string())? {
        return Err(format!("共享源 {field} 缺失，请先完成共享库初始化"));
    }
    read_field(&shared_path(field), field)
}

pub(crate) fn save_shared(field: &str, entries: &Map<String, Value>) -> Result<(), String> {
    fs::create_dir_all(shared_dir()).map_err(|e| format!("创建共享目录失败：{e}"))?;
    let value = serde_json::json!({ field: entries });
    crate::sync::write_json_atomic(&shared_path(field), &value)
        .map_err(|e| format!("写共享源失败：{e}"))
}

/// **首次迁移**（设计 §四）：把默认 Claude 现有配置**只读采集**成共享源的种子。
///
/// - 只在共享源文件**尚不存在**时执行 ⇒ 天然幂等，不需要额外的版本号文件；
///   用户后来即使把共享项全删光，文件仍在（空对象），不会被重新灌回旧内容。
/// - **只读**默认 Claude，不写回、不移动它的任何东西。
/// - **不碰环境**：各环境现有值一律保留，冲突项不静默统一。
pub(crate) fn seed_shared_if_missing(field: &str) -> Result<Option<usize>, String> {
    if shared_path(field).is_file() {
        return Ok(None);
    }
    let harvested = read_field(&default_claude_path(field), field)?;
    let count = harvested.len();
    save_shared(field, &harvested)?;
    Ok(Some(count))
}

/// 一个环境在一轮分发里的结果。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct EnvOutcome {
    pub env: String,
    pub written: usize,
    pub removed: usize,
    pub overrides: Vec<OverrideInfo>,
}

/// 一轮分发后给界面看的汇总。
#[derive(Debug, Default)]
pub(crate) struct DistributeReport {
    pub envs: Vec<EnvOutcome>,
    pub warnings: Vec<String>,
}

impl DistributeReport {
    pub fn override_count(&self) -> usize {
        self.envs.iter().map(|e| e.overrides.len()).sum()
    }
}

/// 把共享源**单向分发**到全部受管理环境（设计 §三）。
///
/// 只动应用管理的条目；环境文件其余部分（登录态、permissions、用户自己的条目…）
/// 由 `apply_plan` 原样保留。环境目录不存在（从未启动）时跳过，不凭空建文件。
/// **默认 Claude 完全不在分发范围内。**
pub(crate) fn distribute(field: &str, names: &[String]) -> Result<DistributeReport, String> {
    let shared = load_shared(field)?;
    let mut ledger = load_ledger()?;
    let targets: Vec<(String, PathBuf)> = names
        .iter()
        .filter(|n| !n.is_empty())
        .map(|name| {
            let path = env_path(name, field);
            (name.clone(), path)
        })
        .collect();
    let mut paths: Vec<PathBuf> = targets.iter().map(|(_, path)| path.clone()).collect();
    paths.push(ledger_path());
    with_file_rollback(&paths, || {
        let report = distribute_to(field, &shared, &mut ledger, &targets)?;
        save_ledger(&ledger)?;
        Ok(report)
    })
}

/// 持有配置锁期间，文件写入失败时恢复本轮已改变的文件（包含台账）。
fn with_file_rollback<T>(
    paths: &[PathBuf],
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let snapshots = paths
        .iter()
        .map(|path| {
            let bytes = match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(format!("无法备份 {}：{e}", path.display())),
            };
            Ok((path, bytes))
        })
        .collect::<Result<Vec<_>, String>>()?;
    match action() {
        Ok(value) => Ok(value),
        Err(error) => {
            let mut failures = vec![];
            for (path, before) in snapshots.into_iter().rev() {
                if fs::read(path).ok() == before {
                    continue;
                }
                let restored = match before {
                    Some(bytes) => crate::sync::write_bytes_atomic(path, &bytes),
                    None => fs::remove_file(path),
                };
                if let Err(e) = restored {
                    failures.push(format!("{}：{e}", path.display()));
                }
            }
            if failures.is_empty() {
                Err(format!("{error}；本轮文件修改已回退"))
            } else {
                Err(format!(
                    "{error}；部分文件回退失败：{}",
                    failures.join("；")
                ))
            }
        }
    }
}

/// 首次迁移入口：两个域各采集一次（幂等）。在 GUI 启动、`--sync`、
/// 以及任何分发之前调用都安全 —— 共享源已存在时它就是一次文件存在性检查。
pub(crate) fn ensure_seeded() -> Vec<String> {
    let mut notes = vec![];
    for field in [FIELD_MCP, FIELD_PLUGINS] {
        match seed_shared_if_missing(field) {
            Ok(Some(count)) => notes.push(format!(
                "{field}：已从现有配置只读采集 {count} 项到应用共享库"
            )),
            Ok(None) => {}
            Err(e) => notes.push(format!("{field}：共享库初始化失败：{e}")),
        }
    }
    notes
}

/// 「恢复使用共享配置」/「恢复继承」：把环境里被覆盖的那一条改回共享值，并更新台账。
///
/// **只有用户显式执行才会走到这里** —— 分发自身永远不会替用户撤销覆盖
/// （那正是"用户改过的条目不得被覆盖"的反面：也不该被自动改回去）。
pub(crate) fn restore_entry(field: &str, env: &str, name: &str) -> Result<String, String> {
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    validate_env(env)?;
    let mut ledger = load_ledger()?;
    let shared = load_shared(field)?;
    let Some(value) = shared.get(name).cloned() else {
        return Err(format!(
            "共享库里已经没有「{name}」了，无法恢复。它可能已被删除 —— 如果这个环境也不需要它，请直接在该环境里删除。"
        ));
    };
    let path = env_path(env, field);
    let text = fs::read_to_string(&path).map_err(|e| format!("读取环境配置失败：{e}"))?;
    let mut doc: Value =
        serde_json::from_str(&text).map_err(|e| format!("环境配置不是有效 JSON：{e}"))?;
    let obj = doc
        .as_object_mut()
        .ok_or("环境配置顶层不是对象，已中止（不覆盖可疑文件）")?;
    obj.entry(field.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(format!("{field} 不是对象，已中止"))?
        .insert(name.to_string(), value.clone());
    let mut entries = ledger.entries(field, env);
    entries.insert(name.to_string(), value);
    ledger.set_entries(field, env, entries);
    ledger.mark_override(field, env, name, false);
    with_file_rollback(&[path.clone(), ledger_path()], || {
        crate::sync::write_json_atomic(&path, &doc).map_err(|e| format!("写回失败：{e}"))?;
        save_ledger(&ledger)
    })?;
    Ok(format!("已把环境「{env}」的「{name}」恢复为共享配置。"))
}

/// 在**应用共享库**里设置某个条目（并立刻分发）。
pub(crate) fn set_shared_entry(field: &str, name: &str, value: Value) -> Result<String, String> {
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let mut shared = load_shared(field)?;
    shared.insert(name.to_string(), value);
    let names = crate::profile_names(&crate::load());
    let report = with_file_rollback(&[shared_path(field)], || {
        save_shared(field, &shared)?;
        distribute(field, &names)
    })?;
    let written: usize = report.envs.iter().map(|e| e.written).sum();
    let overridden = report.override_count();
    let mut msg = format!("共享库已更新「{name}」，分发到 {written} 处");
    if overridden > 0 {
        msg.push_str(&format!(
            "；另有 {overridden} 处被环境自身的设置覆盖、未被改写"
        ));
    }
    if !report.warnings.is_empty() {
        msg.push_str(&format!("；{}", report.warnings.join("；")));
    }
    Ok(msg)
}

fn validate_env(env: &str) -> Result<(), String> {
    if env == crate::MAIN_PROFILE_KEY
        || env == "."
        || env == ".."
        || !crate::script_safe_name(env)
        || !crate::load().iter().any(|p| p.name == env)
    {
        return Err("未找到可编辑的受管理环境".into());
    }
    Ok(())
}

/// 在**某个环境**里设置某个条目（= 该环境的独立覆盖）。
///
/// 不改上次分发值，另记显式覆盖意图；即使值暂时与共享值相同，也不会丢失独立设置。
pub(crate) fn set_env_entry(
    field: &str,
    env: &str,
    name: &str,
    value: Value,
) -> Result<String, String> {
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    validate_env(env)?;
    let mut ledger = load_ledger()?;
    let path = env_path(env, field);
    let mut doc = read_document(&path)?;
    set_field_value(&mut doc, field, name, value)?;
    ledger.mark_override(field, env, name, true);
    with_file_rollback(&[path.clone(), ledger_path()], || {
        crate::sync::write_json_atomic(&path, &doc).map_err(|e| format!("写回失败：{e}"))?;
        save_ledger(&ledger)
    })?;
    Ok(format!("已把环境「{env}」的「{name}」设为独立设置。"))
}

/// 把一个条目写进文档的某个域，**只动这个域里的这一个条目**。
///
/// 这是 `settings.json` / `.claude.json` 这类**属于用户**的文件，所以字段类型不对时
/// **报错中止**，而不是像分发路径那样重建它 —— 宁可失败也不覆盖可疑内容。
fn set_field_value(doc: &mut Value, field: &str, name: &str, value: Value) -> Result<(), String> {
    let obj = doc
        .as_object_mut()
        .ok_or("配置顶层不是对象，已中止（不覆盖可疑文件）")?;
    obj.entry(field.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(format!("{field} 不是对象，已中止（不覆盖可疑内容）"))?
        .insert(name.to_string(), value);
    Ok(())
}

// ---------------- 插件启用状态：界面用的总览 ----------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginEnvState {
    pub env: String,
    /// 该环境当前的值；没设置时为 None
    pub value: Option<Value>,
    /// true = 与共享值一致（继承）；false = 覆盖
    pub inherited: bool,
    /// 覆盖原因（继承时为空串）
    pub reason: String,
}

/// 一个环境在 `plugins_overview` 里的中间态：它的值 + 逐条的继承/覆盖判定。
type EnvPluginState = (String, Map<String, Value>, Vec<(String, bool, String)>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginRow {
    pub name: String,
    /// 共享库里的值
    pub shared: Value,
    /// 默认 Claude 的值 —— **只展示，不可编辑**（约束 4：应用对默认 Claude 只读）
    pub default_claude: Option<Value>,
    pub envs: Vec<PluginEnvState>,
}

/// 插件启用状态的总览：共享库有哪些、每个环境是继承还是覆盖、默认 Claude 是什么。
///
/// 判定复用分发引擎的纯函数 `plan_env`，**不另写一套比较** —— 两套判据必然漂移。
pub(crate) fn plugins_overview() -> Result<Vec<PluginRow>, String> {
    let shared = load_shared(FIELD_PLUGINS)?;
    let ledger = load_ledger()?;
    let envs = crate::profile_names(&crate::load());

    // 默认 Claude 的那份：只读展示
    let default_claude = read_field(&default_claude_path(FIELD_PLUGINS), FIELD_PLUGINS)?;

    let mut per_env: Vec<EnvPluginState> = vec![];
    for env in &envs {
        let values = read_field(&env_path(env, FIELD_PLUGINS), FIELD_PLUGINS)?;
        let plan = ledger.plan(FIELD_PLUGINS, env, &shared, &values);
        let states = plan
            .overrides
            .iter()
            .map(|o| (o.name.clone(), false, o.reason.label().to_string()))
            .chain(
                plan.in_sync
                    .iter()
                    .map(|n| (n.clone(), true, String::new())),
            )
            .collect();
        per_env.push((env.clone(), values, states));
    }

    let mut names: Vec<String> = shared.keys().cloned().collect();
    for (_, values, states) in &per_env {
        for n in values.keys().chain(states.iter().map(|(n, _, _)| n)) {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
    }
    names.sort();
    names.dedup();

    Ok(names
        .into_iter()
        .map(|name| PluginRow {
            shared: shared.get(&name).cloned().unwrap_or(Value::Null),
            default_claude: default_claude.get(&name).cloned(),
            envs: per_env
                .iter()
                .map(|(env, values, states)| {
                    let hit = states.iter().find(|(n, _, _)| *n == name);
                    PluginEnvState {
                        env: env.clone(),
                        value: values.get(&name).cloned(),
                        inherited: hit.map(|(_, i, _)| *i).unwrap_or(false),
                        reason: hit.map(|(_, _, r)| r.clone()).unwrap_or_default(),
                    }
                })
                .collect(),
            name,
        })
        .collect())
}

/// 环境被删除后，把它在分发台账里的记录一并清掉。
///
/// 不清的后果是**实打实的**：将来重建一个同名环境时，台账里那条"我们分发过 X"
/// 还在，于是 X 会被判成"我们发过的、用户改过"或"用户删过"，用户怎么改都收敛不到共享值。
pub(crate) fn forget_env(name: &str) -> Result<(), String> {
    let mut ledger = load_ledger()?;
    let mut changed = false;
    for domain in ledger.domains.values_mut() {
        if domain.explicit_overrides.remove(name).is_some() {
            changed = true;
        }
        if domain.envs.remove(name).is_some() {
            changed = true;
        }
    }
    if changed {
        save_ledger(&ledger)?;
    }
    Ok(())
}

/// 台账里是否还留着某个环境的记录（删除后的残留核验用）。
pub(crate) fn ledger_references_env(name: &str) -> Result<bool, String> {
    Ok(load_ledger()?
        .domains
        .values()
        .any(|d| d.envs.contains_key(name) || d.explicit_overrides.contains_key(name)))
}

/// **可测内核**：显式给定共享源、台账与"环境 → 文件路径"，跑完一轮分发并就地更新台账。
///
/// 抽成显式参数版本（照本项目已有的 `*_at` 惯例）是因为 `distribute` 直接读真实
/// `home()`/`cfg_dir()`，无法单测 —— 而这段恰恰是整块功能的核心。
pub(crate) fn distribute_to(
    field: &str,
    shared: &Map<String, Value>,
    ledger: &mut Ledger,
    targets: &[(String, PathBuf)],
) -> Result<DistributeReport, String> {
    let mut report = DistributeReport::default();

    for (env, path) in targets {
        // 环境目录不存在（从未启动）→ 跳过，不凭空建文件
        if !path.parent().map(|d| d.is_dir()).unwrap_or(false) {
            continue;
        }
        let before = ledger.entries(field, env);
        // **文件存在但读不了 / 解析不了时绝不覆盖。**
        // 这里踩过一次：原先写成 `read_to_string(..).ok().and_then(parse).unwrap_or(默认空对象)`，
        // 于是"解析失败"和"文件不存在"被混成同一种情况 —— 环境那份 `.claude.json` 一旦
        // 暂时不可解析（正在被写、被别的进程改坏），本轮就会把它**整个换掉**，
        // 连同登录态、项目记录一起丢。宁可跳过并告警，等它自愈。
        let mut doc: Value = match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) if v.is_object() => v,
                Ok(_) => {
                    report.warnings.push(format!(
                        "{}（{}）顶层不是 JSON 对象，本轮未分发",
                        env,
                        path.display()
                    ));
                    continue;
                }
                Err(e) => {
                    report.warnings.push(format!(
                        "{}（{}）解析失败（{e}），本轮未分发、也未覆盖",
                        env,
                        path.display()
                    ));
                    continue;
                }
            },
            // 文件还不存在（环境刚建、还没启动过）→ 从空对象开始，只写该域
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
            Err(e) => {
                report.warnings.push(format!(
                    "{}（{}）读取失败（{e}），本轮未分发",
                    env,
                    path.display()
                ));
                continue;
            }
        };
        if doc.get(field).is_some_and(|v| !v.is_object()) {
            report
                .warnings
                .push(format!("{env} 的 {field} 不是对象，本轮未分发"));
            continue;
        }
        let env_entries = doc
            .get(field)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let plan = ledger.plan(field, env, shared, &env_entries);
        let after = apply_plan(&mut doc, field, &plan, &before);
        let outcome = EnvOutcome {
            env: env.clone(),
            written: plan.writes.len(),
            removed: plan.removals.len(),
            overrides: plan.overrides.clone(),
        };

        if !plan.is_noop() {
            crate::sync::write_json_atomic(path, &doc)
                .map_err(|e| format!("写回 {} 失败：{e}", path.display()))?;
        }
        ledger.set_entries(field, env, after);
        report.envs.push(outcome);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    // ---- 文件层（用临时目录，任一平台可跑）----

    #[test]
    fn corrupted_source_and_invalid_fields_are_never_treated_as_empty() {
        let dir = TmpDir::new("strict-read");
        for text in ["{broken", "[]", r#"{"enabledPlugins":false}"#] {
            let path = dir.env_file("bad", text);
            assert!(read_field(&path, FIELD_PLUGINS).is_err());
            assert_eq!(fs::read_to_string(path).unwrap(), text);
        }
    }

    #[test]
    fn removing_shared_entry_preserves_modified_environment_value() {
        let dir = TmpDir::new("remove-override");
        let path = dir.env_file("corp", r#"{"mcpServers":{"A":{"v":2}}}"#);
        let mut ledger = Ledger::default().normalized();
        ledger.set_entries(FIELD_MCP, "corp", obj(json!({"A":{"v":1}})));
        let report = distribute_to(
            FIELD_MCP,
            &Map::new(),
            &mut ledger,
            &[("corp".into(), path.clone())],
        )
        .unwrap();
        assert_eq!(report.override_count(), 1);
        assert_eq!(dir.read(&path)[FIELD_MCP]["A"]["v"], 2);
    }

    #[test]
    fn explicit_plugin_override_survives_equal_shared_value_and_later_updates() {
        let dir = TmpDir::new("explicit-override");
        let path = dir.env_file("corp", r#"{"enabledPlugins":{"p@m":false}}"#);
        let mut ledger = Ledger::default().normalized();
        ledger.mark_override(FIELD_PLUGINS, "corp", "p@m", true);
        for value in [false, true, false, true] {
            let report = distribute_to(
                FIELD_PLUGINS,
                &obj(json!({"p@m": value})),
                &mut ledger,
                &[("corp".into(), path.clone())],
            )
            .unwrap();
            assert_eq!(dir.read(&path)[FIELD_PLUGINS]["p@m"], false);
            assert_eq!(report.override_count(), 1);
        }
        ledger.mark_override(FIELD_PLUGINS, "corp", "p@m", false);
        ledger.set_entries(FIELD_PLUGINS, "corp", obj(json!({"p@m":false})));
        distribute_to(
            FIELD_PLUGINS,
            &obj(json!({"p@m":true})),
            &mut ledger,
            &[("corp".into(), path.clone())],
        )
        .unwrap();
        assert_eq!(dir.read(&path)[FIELD_PLUGINS]["p@m"], true);
    }

    #[test]
    fn distribution_preserves_wrong_type_field_and_reports_it() {
        let dir = TmpDir::new("field-type");
        let text = r#"{"enabledPlugins":["keep"],"theme":"dark"}"#;
        let path = dir.env_file("corp", text);
        let report = distribute_to(
            FIELD_PLUGINS,
            &obj(json!({"p@m":true})),
            &mut Ledger::default(),
            &[("corp".into(), path.clone())],
        )
        .unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }

    #[test]
    fn failed_batch_restores_exact_bytes_and_removes_new_files() {
        let dir = TmpDir::new("rollback");
        let old = dir.env_file("corp", "{ \"theme\" : \"dark\" }\n");
        let new = old.with_file_name("new.json");
        let result: Result<(), String> = with_file_rollback(&[old.clone(), new.clone()], || {
            crate::sync::write_json_atomic(&old, &json!({"changed":true})).unwrap();
            crate::sync::write_json_atomic(&new, &json!({})).unwrap();
            Err("模拟台账落盘失败".into())
        });
        assert!(result.unwrap_err().contains("已回退"));
        assert_eq!(
            fs::read_to_string(old).unwrap(),
            "{ \"theme\" : \"dark\" }\n"
        );
        assert!(!new.exists());
    }

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ccm-shared-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&dir).unwrap();
            TmpDir(dir)
        }
        /// 造一个"已启动过的环境"目录 + 文件
        fn env_file(&self, env: &str, content: &str) -> PathBuf {
            let dir = self.0.join(env);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join(".claude.json");
            fs::write(&path, content).unwrap();
            path
        }
        fn read(&self, path: &Path) -> Value {
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn distribute_writes_shared_entries_into_every_environment() {
        let dir = TmpDir::new("write");
        let a = dir.env_file(
            "a",
            r#"{"mcpServers":{"mine":{"cmd":"node"}},"theme":"dark"}"#,
        );
        let b = dir.env_file("b", r#"{"mcpServers":{}}"#);
        let shared = obj(json!({"A": {"cmd": "npx"}}));
        let targets = vec![("a".to_string(), a.clone()), ("b".to_string(), b.clone())];
        let mut ledger = Ledger::default();

        let report = distribute_to(FIELD_MCP, &shared, &mut ledger, &targets).unwrap();

        assert_eq!(report.envs.len(), 2);
        for path in [&a, &b] {
            assert_eq!(dir.read(path)["mcpServers"]["A"], json!({"cmd": "npx"}));
        }
        // 验收 9：其余字段与环境自己的条目原样保留
        assert_eq!(dir.read(&a)["theme"], "dark");
        assert_eq!(dir.read(&a)["mcpServers"]["mine"], json!({"cmd": "node"}));
        // 台账记下了发下去的内容
        assert_eq!(ledger.entries(FIELD_MCP, "a")["A"], json!({"cmd": "npx"}));
    }

    #[test]
    fn second_distribution_run_writes_nothing() {
        // 验收 8：重复分发结果稳定
        let dir = TmpDir::new("idem");
        let a = dir.env_file("a", "{}");
        let shared = obj(json!({"A": {"cmd": "npx"}}));
        let targets = vec![("a".to_string(), a.clone())];
        let mut ledger = Ledger::default();

        distribute_to(FIELD_MCP, &shared, &mut ledger, &targets).unwrap();
        let after_first = dir.read(&a);
        let report = distribute_to(FIELD_MCP, &shared, &mut ledger, &targets).unwrap();

        assert_eq!(report.envs[0].written, 0, "第二遍还在写");
        assert_eq!(report.envs[0].removed, 0);
        assert_eq!(dir.read(&a), after_first, "内容被改动了");
    }

    #[test]
    fn distribution_skips_environments_that_never_launched() {
        // 环境目录不存在 ⇒ 跳过，绝不凭空建文件
        let dir = TmpDir::new("skip");
        let ghost = dir.0.join("ghost").join(".claude.json");
        let targets = vec![("ghost".to_string(), ghost.clone())];
        let mut ledger = Ledger::default();

        let report =
            distribute_to(FIELD_MCP, &obj(json!({"A": {}})), &mut ledger, &targets).unwrap();

        assert!(report.envs.is_empty());
        assert!(!ghost.exists(), "为从未启动的环境建了文件");
    }

    #[test]
    fn distribution_reports_an_unparseable_environment_file_instead_of_overwriting_it() {
        let dir = TmpDir::new("bad");
        let broken = dir.env_file("broken", "not json");
        let targets = vec![("broken".to_string(), broken.clone())];
        let mut ledger = Ledger::default();

        let report =
            distribute_to(FIELD_MCP, &obj(json!({"A": {}})), &mut ledger, &targets).unwrap();

        assert!(report.envs.is_empty());
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert_eq!(fs::read_to_string(&broken).unwrap(), "not json");
    }

    #[test]
    fn environment_files_live_under_the_environment_and_never_are_the_default_claude_file() {
        // 结构不变量：分发目标永远在 `~/.claude-split/<环境>/…` 下。
        // 这条保证"默认 Claude 完全不在分发范围内"不是靠自觉，而是路径上就不可能出现。
        let home = crate::home();
        let default_claude = home.join(".claude.json");
        let default_settings = home.join(".claude").join("settings.json");
        for field in [FIELD_MCP, FIELD_PLUGINS] {
            for env in ["corp", "test", "我的环境"] {
                let path = env_path(env, field);
                assert_ne!(path, default_claude, "{field}/{env} 指到了默认 Claude");
                assert_ne!(path, default_settings, "{field}/{env} 指到了默认 Claude");
                assert!(
                    path.starts_with(home.join(".claude-split").join(env)),
                    "{field}/{env} 跑出了环境目录：{}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn set_field_value_only_touches_its_own_field_and_entry() {
        // 这是**用户自己的** settings.json：只允许动目标域里的那一个条目
        let mut doc = json!({
            "enabledPlugins": {"a@m": true},
            "permissions": {"allow": ["Bash"]},
            "theme": "dark"
        });

        set_field_value(&mut doc, FIELD_PLUGINS, "b@m", json!(false)).unwrap();

        assert_eq!(doc["enabledPlugins"]["b@m"], json!(false));
        assert_eq!(doc["enabledPlugins"]["a@m"], json!(true), "别的插件被动了");
        assert_eq!(doc["permissions"]["allow"][0], "Bash", "无关字段被动了");
        assert_eq!(doc["theme"], "dark");
    }

    #[test]
    fn set_field_value_refuses_to_overwrite_a_corrupted_field() {
        // 与分发路径**相反**：这里宁可报错也不重建 —— 文件是用户的，
        // 把 array 静默换成 object 会连带丢掉他写在里面的东西。
        let mut doc = json!({"enabledPlugins": ["not", "an", "object"]});
        let err = set_field_value(&mut doc, FIELD_PLUGINS, "a@m", json!(true)).unwrap_err();
        assert!(err.contains("不是对象"), "{err}");
        assert_eq!(
            doc["enabledPlugins"],
            json!(["not", "an", "object"]),
            "损坏的字段被改写了"
        );
    }

    #[test]
    fn new_shared_entry_is_distributed_to_every_environment() {
        let shared = obj(json!({"A": {"cmd": "npx"}}));
        let plan = plan_env(&shared, &Map::new(), &Map::new());
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.writes["A"], json!({"cmd": "npx"}));
        assert!(plan.removals.is_empty() && plan.overrides.is_empty());
    }

    #[test]
    fn identical_values_are_in_sync_not_a_conflict() {
        // 验收：同名、内容相同 → 不报冲突
        let shared = obj(json!({"A": {"cmd": "npx"}}));
        let env = obj(json!({"A": {"cmd": "npx"}}));
        let plan = plan_env(&shared, &env, &Map::new());
        assert!(plan.is_noop(), "{plan:?}");
        assert!(plan.overrides.is_empty(), "同内容不该报冲突：{plan:?}");
        assert_eq!(plan.in_sync, vec!["A"]);
    }

    #[test]
    fn independently_configured_entry_with_the_same_name_wins() {
        // 验收 1：环境独立配置与共享项同名、内容不同 → 保留环境配置 + 报"已覆盖"
        let shared = obj(json!({"A": {"cmd": "npx", "args": ["shared"]}}));
        let env = obj(json!({"A": {"cmd": "npx", "args": ["mine"]}}));
        let plan = plan_env(&shared, &env, &Map::new());
        assert!(plan.is_noop(), "不能覆盖环境的配置：{plan:?}");
        assert_eq!(plan.overrides.len(), 1);
        assert_eq!(plan.overrides[0].reason, OverrideReason::Independent);
        // 可查看两边差异：两个值都要带出来
        assert_eq!(
            plan.overrides[0].env_value,
            Some(json!({"cmd": "npx", "args": ["mine"]}))
        );
        assert_eq!(
            plan.overrides[0].shared_value,
            Some(json!({"cmd": "npx", "args": ["shared"]}))
        );
    }

    #[test]
    fn entry_we_distributed_follows_the_shared_source() {
        // 验收 3：此前由应用分发的条目，共享源更新时必须正常更新 ——
        // **不能**因为"目标里已经有这个名字"就误判成环境独立配置
        let old = json!({"cmd": "npx", "args": ["v1"]});
        let new = json!({"cmd": "npx", "args": ["v2"]});
        let shared = obj(json!({"A": new.clone()}));
        let env = obj(json!({"A": old.clone()}));
        let ledger = obj(json!({"A": old.clone()}));
        let plan = plan_env(&shared, &env, &ledger);
        assert_eq!(plan.writes["A"], new, "{plan:?}");
        assert!(plan.overrides.is_empty(), "{plan:?}");
    }

    #[test]
    fn entry_the_user_edited_is_never_overwritten() {
        // 验收 4：用户修改过已分发的条目 → 识别为环境覆盖，后续分发不得覆盖
        let distributed = json!({"cmd": "npx", "args": ["v1"]});
        let user_edited = json!({"cmd": "npx", "args": ["v1", "--mine"]});
        let shared = obj(json!({"A": {"cmd": "npx", "args": ["v2"]}}));
        let env = obj(json!({"A": user_edited.clone()}));
        let ledger = obj(json!({"A": distributed}));
        let plan = plan_env(&shared, &env, &ledger);
        assert!(plan.is_noop(), "用户的修改被覆盖了：{plan:?}");
        assert_eq!(
            plan.overrides[0].reason,
            OverrideReason::ModifiedAfterDistribution
        );
    }

    #[test]
    fn a_deleted_inherited_entry_is_not_resurrected() {
        // 我们发过、环境侧后来删了：删除也是用户的表态，不复活（且要报出来）
        let shared = obj(json!({"A": {"cmd": "npx"}}));
        let ledger = obj(json!({"A": {"cmd": "npx"}}));
        let plan = plan_env(&shared, &Map::new(), &ledger);
        assert!(plan.is_noop(), "不该复活：{plan:?}");
        assert_eq!(plan.overrides[0].reason, OverrideReason::DeletedByUser);
    }

    #[test]
    fn removing_a_shared_entry_cleans_only_what_we_distributed() {
        // 验收 10：删除共享项 → 只清理继承该项的状态，保留环境自己的覆盖设置
        let env = obj(json!({
            "inherited": {"cmd": "npx"},
            "mine": {"cmd": "node"}
        }));
        let ledger = obj(json!({"inherited": {"cmd": "npx"}}));
        let plan = plan_env(&Map::new(), &env, &ledger);
        assert_eq!(plan.removals, vec!["inherited"]);
        assert!(
            !plan.removals.contains(&"mine".to_string()),
            "环境自己的条目被误删：{plan:?}"
        );
        assert!(plan.writes.is_empty());
    }

    #[test]
    fn environment_only_entries_are_left_alone() {
        let env = obj(json!({"mine": {"cmd": "node"}}));
        let plan = plan_env(&Map::new(), &env, &Map::new());
        assert!(plan.is_noop(), "{plan:?}");
        assert!(
            plan.overrides.is_empty(),
            "共享源里没有它，不该报冲突：{plan:?}"
        );
    }

    #[test]
    fn stray_ledger_records_are_cleaned_up() {
        // 共享项已删、环境里也没了，但台账仍留着记录 —— 必须清掉，否则
        // 用户将来重新加同名条目时会被误判成"我们发过的"
        let ledger = obj(json!({"gone": {"cmd": "npx"}}));
        let plan = plan_env(&Map::new(), &Map::new(), &ledger);
        let mut doc = json!({});
        let field = FIELD_MCP;
        let next = apply_plan(&mut doc, field, &plan, &ledger.clone());
        assert!(next.is_empty(), "台账残留没清：{next:?}");
    }

    #[test]
    fn explicit_false_is_preserved_not_treated_as_unset() {
        // 验收 7（插件域）：环境把某个插件显式设为 false，
        // 必须原样保留，不能被当成"没设置"而被共享值覆盖
        let shared = obj(json!({"p@m": true}));
        let env = obj(json!({"p@m": false}));
        let ledger = obj(json!({"p@m": true}));
        let plan = plan_env(&shared, &env, &ledger);
        assert!(plan.is_noop(), "显式 false 被覆盖了：{plan:?}");
        assert_eq!(
            plan.overrides[0].reason,
            OverrideReason::ModifiedAfterDistribution
        );
    }

    #[test]
    fn distribution_is_idempotent() {
        // 验收 8：重复分发结果稳定 —— 跑完一遍后立刻再跑，必须零写入。
        let shared = obj(json!({"A": {"v": 1}, "B": {"v": 2}}));
        let mut doc = json!({"mcpServers": {}, "other": {"keep": true}});

        let plan1 = plan_env(&shared, &Map::new(), &Map::new());
        let ledger1 = apply_plan(&mut doc, FIELD_MCP, &plan1, &Map::new());

        let env_after = doc[FIELD_MCP].as_object().unwrap().clone();
        let plan2 = plan_env(&shared, &env_after, &ledger1);
        assert!(plan2.is_noop(), "第二遍还在写：{plan2:?}");
        assert_eq!(plan2.in_sync.len(), 2);

        let ledger2 = apply_plan(&mut doc, FIELD_MCP, &plan2, &ledger1);
        assert_eq!(ledger2, ledger1, "台账在空跑一轮后变了");
    }

    #[test]
    fn apply_plan_keeps_unrelated_fields_and_entries() {
        // 验收 9：只动应用管理的条目，文档其余字段与用户自己的条目原样保留
        let mut doc = json!({
            "mcpServers": {"mine": {"cmd": "node"}, "stale": {"cmd": "old"}},
            "permissions": {"allow": ["Bash"]},
            "theme": "dark"
        });
        let shared = obj(json!({"shared": {"cmd": "npx"}}));
        let env = obj(json!({"mine": {"cmd": "node"}, "stale": {"cmd": "old"}}));
        let ledger = obj(json!({"stale": {"cmd": "old"}}));

        let plan = plan_env(&shared, &env, &ledger);
        let _ = apply_plan(&mut doc, FIELD_MCP, &plan, &ledger);

        let map = doc[FIELD_MCP].as_object().unwrap();
        assert!(map.contains_key("mine"), "用户自己的条目被弄丢了");
        assert!(map.contains_key("shared"), "共享条目没写进去");
        assert!(!map.contains_key("stale"), "共享项被删后没清理继承值");
        assert_eq!(doc["permissions"]["allow"][0], "Bash");
        assert_eq!(doc["theme"], "dark");
    }

    #[test]
    fn apply_plan_rebuilds_a_corrupted_field_instead_of_writing_garbage() {
        // 该字段存在但不是对象（文件被改坏）：重建为对象，
        // 而不是把新条目塞进数组/字符串里写出非法结构
        let mut doc = json!({"mcpServers": ["not", "an", "object"]});
        let shared = obj(json!({"A": {"v": 1}}));
        let plan = plan_env(&shared, &Map::new(), &Map::new());
        let _ = apply_plan(&mut doc, FIELD_MCP, &plan, &Map::new());
        assert_eq!(doc[FIELD_MCP], json!({"A": {"v": 1}}));
    }

    #[test]
    fn ledger_with_an_unknown_version_is_treated_as_empty() {
        // 将来换算法时，旧格式的台账不能拿来做判定 —— 宁可重判一轮
        let stale = Ledger {
            version: 0,
            domains: BTreeMap::from([(
                FIELD_MCP.to_string(),
                DomainLedger {
                    envs: BTreeMap::from([("corp".to_string(), obj(json!({"A": {"v": 1}})))]),
                    ..Default::default()
                },
            )]),
        };
        let normalized = stale.normalized();
        assert_eq!(normalized.version, LEDGER_VERSION);
        assert!(normalized.entries(FIELD_MCP, "corp").is_empty());
    }
}
