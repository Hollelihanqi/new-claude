//! Skills / Agents 的逐项共享与环境覆盖。
//!
//! 共享库是唯一来源；每个目标保存一份真实副本。状态文件只记录本应用上次写入的
//! 内容哈希，因此能区分“仍在继承”“用户改过”“用户删过”和“环境独有”。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STATE_VERSION: u32 = 2;
const CODEX_TARGET: &str = "__codex__";
const AUTO_IMPORT_INTERVAL_SECS: u64 = 3 * 24 * 60 * 60 + 12 * 60 * 60;
const MAX_PLUGIN_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PLUGIN_PACKAGE_FILES: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Skills,
    Agents,
}

impl Kind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "skills" => Ok(Self::Skills),
            "agents" => Ok(Self::Agents),
            _ => Err("只支持 Skills 或 Agents".into()),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::Agents => "agents",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Skills => "Skill",
            Self::Agents => "Agent",
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Default, Serialize, Deserialize)]
struct AutoImportRecord {
    checked_at: u64,
    added: usize,
    skipped: usize,
    #[serde(default)]
    failures: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct ResourceState {
    #[serde(default)]
    version: u32,
    /// kind → target → item → 上次分发内容哈希
    #[serde(default)]
    distributed: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
    /// kind → target → 被用户明确排除的 item
    #[serde(default)]
    excluded: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
    /// kind → 用户从共享库删除、自动导入不得复活的 item
    #[serde(default)]
    ignored_imports: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    last_auto_import: u64,
    #[serde(default = "default_true")]
    auto_import_enabled: bool,
    #[serde(default)]
    last_auto_import_record: Option<AutoImportRecord>,
    /// 环境 → 不跟随共享插件策略的插件名称。停用不在这里记录。
    #[serde(default)]
    plugin_excluded: BTreeMap<String, BTreeSet<String>>,
}

impl ResourceState {
    fn normalized(mut self) -> Self {
        if self.version != STATE_VERSION {
            // v1 已有的分发台账仍然有效，只补新字段，不能把用户的覆盖/排除记录清空。
            if self.version != 1 {
                self = Self::default();
            }
            self.auto_import_enabled = true;
        }
        self.version = STATE_VERSION;
        self
    }

    fn ledger_mut(&mut self, kind: Kind, target: &str) -> &mut BTreeMap<String, String> {
        self.distributed
            .entry(kind.key().into())
            .or_default()
            .entry(target.into())
            .or_default()
    }

    fn exclusions_mut(&mut self, kind: Kind, target: &str) -> &mut BTreeSet<String> {
        self.excluded
            .entry(kind.key().into())
            .or_default()
            .entry(target.into())
            .or_default()
    }
}

#[derive(Clone)]
struct Target {
    id: String,
    label: String,
    root: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceTargetState {
    target: String,
    label: String,
    state: String,
    reason: String,
    issues: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceItem {
    name: String,
    in_shared: bool,
    in_default_claude: bool,
    targets: Vec<ResourceTargetState>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceOverview {
    kind: String,
    label: String,
    shared_path: String,
    items: Vec<ResourceItem>,
    targets: Vec<ResourceTargetState>,
    auto_import_enabled: bool,
    last_auto_import_at: Option<u64>,
    last_auto_import_added: usize,
    last_auto_import_skipped: usize,
    last_auto_import_failures: Vec<String>,
}

fn state_path() -> PathBuf {
    crate::cfg_dir().join("shared").join("resources.json")
}

fn read_state() -> Result<ResourceState, String> {
    match fs::read_to_string(state_path()) {
        Ok(text) => serde_json::from_str::<ResourceState>(&text)
            .map(ResourceState::normalized)
            .map_err(|e| format!("扩展分发记录损坏，已中止：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(ResourceState::default().normalized())
        }
        Err(e) => Err(format!("读取扩展分发记录失败：{e}")),
    }
}

fn save_state(state: &ResourceState) -> Result<(), String> {
    let value = serde_json::to_value(state).map_err(|e| e.to_string())?;
    crate::sync::write_json_atomic(&state_path(), &value)
        .map_err(|e| format!("保存扩展分发记录失败：{e}"))
}

fn forget_target_from_state(state: &mut ResourceState, target: &str) {
    for targets in state.distributed.values_mut() {
        targets.remove(target);
    }
    for targets in state.excluded.values_mut() {
        targets.remove(target);
    }
    state.plugin_excluded.remove(target);
}

fn state_has_target(state: &ResourceState, target: &str) -> bool {
    state
        .distributed
        .values()
        .any(|targets| targets.contains_key(target))
        || state
            .excluded
            .values()
            .any(|targets| targets.contains_key(target))
        || state.plugin_excluded.contains_key(target)
}

pub(crate) fn forget_target_locked(target: &str) -> Result<(), String> {
    let mut state = read_state()?;
    forget_target_from_state(&mut state, target);
    save_state(&state)
}

pub(crate) fn state_references_target(target: &str) -> Result<bool, String> {
    let state = read_state()?;
    Ok(state_has_target(&state, target))
}

fn shared_root(kind: Kind) -> PathBuf {
    crate::sync::shared_root().join(kind.key())
}

fn default_root(kind: Kind) -> PathBuf {
    crate::home().join(".claude").join(kind.key())
}

fn targets(kind: Kind, names: &[String]) -> Vec<Target> {
    let mut out = names
        .iter()
        .filter(|name| crate::script_safe_name(name) && !name.is_empty())
        .map(|name| Target {
            id: name.clone(),
            label: format!("Claude 环境 {name}"),
            root: crate::sync::instance_dir(name).join(kind.key()),
        })
        .collect::<Vec<_>>();
    if kind == Kind::Skills {
        out.push(Target {
            id: CODEX_TARGET.into(),
            label: "Codex / ChatGPT".into(),
            root: crate::home().join(".codex").join("skills"),
        });
    }
    out
}

fn valid_item_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
}

fn item_names(root: &Path) -> Result<Vec<String>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("读取 {} 失败：{e}", root.display())),
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| valid_item_name(name))
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    Ok(names)
}

fn validate_item(kind: Kind, path: &Path) -> Result<(), String> {
    // Skill/Agent 本身允许是用户已有的链接。Windows RedirectionGuard 下不能通过
    // 原路径跟随 Junction，先读取链接目标，再校验实体内容。
    let resolved = resolve_without_traversal(path)
        .map_err(|e| format!("无法读取 {} 指向的内容：{e}", path.display()))?;
    match kind {
        Kind::Skills => {
            if !resolved.is_dir() || !resolved.join("SKILL.md").is_file() {
                return Err("Skill 必须是包含 SKILL.md 的目录".into());
            }
        }
        Kind::Agents => {
            if !resolved.is_file()
                || !path
                    .extension()
                    .and_then(|v| v.to_str())
                    .is_some_and(|v| v.eq_ignore_ascii_case("md"))
            {
                return Err("Agent 必须是 Markdown 文件".into());
            }
            let text =
                fs::read_to_string(&resolved).map_err(|e| format!("读取 Agent 失败：{e}"))?;
            let trimmed = text.trim_start();
            if !trimmed.starts_with("---") {
                return Err("Agent 缺少开头的配置区（---）".into());
            }
            let rest = trimmed.strip_prefix("---").unwrap_or_default();
            let Some((frontmatter, _)) = rest.split_once("\n---") else {
                return Err("Agent 的配置区没有结束标记（---）".into());
            };
            let name = frontmatter_value(frontmatter, "name");
            let description = frontmatter_value(frontmatter, "description");
            if name.as_deref().is_none_or(str::is_empty) {
                return Err("Agent 配置区缺少 name".into());
            }
            if !name.as_deref().is_some_and(valid_item_name) {
                return Err("Agent 的 name 含有不安全字符".into());
            }
            if description.as_deref().is_none_or(str::is_empty) {
                return Err("Agent 配置区缺少 description".into());
            }
            if let Some(model) = frontmatter_value(frontmatter, "model") {
                if model.chars().any(char::is_whitespace) {
                    return Err("Agent 的 model 必须是单个模型名称".into());
                }
            }
        }
    }
    Ok(())
}

fn remove_entry(path: &Path) -> Result<(), String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if meta.file_type().is_symlink() {
        // Unix 的目录符号链接必须用 remove_file 删除；Windows 的目录 Junction
        // 则必须用 remove_dir。依次尝试两种非递归操作，只删除链接本身。
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(|e| e.to_string())
    } else if meta.is_dir() {
        fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// Windows 的 read_link 对 Junction 可能返回 `\??\C:\...` 或 `\\?\C:\...` 形式的
/// 目标；去掉设备前缀，得到可以直接打开的普通绝对路径。
fn strip_device_prefix(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    let stripped = text
        .strip_prefix(r"\??\")
        .or_else(|| text.strip_prefix(r"\\?\"));
    match stripped {
        Some(rest) => PathBuf::from(rest.to_string()),
        None => path.to_path_buf(),
    }
}

/// 把路径末端的目录链接（Windows Junction / Unix symlink）解析成真实路径。
/// 不能用 fs::canonicalize：它会**打开句柄穿越** reparse point，而进程可能带着
/// Windows RedirectionGuard 缓解策略运行（新版安装器直接运行安装好的应用、或经
/// WebView2 宿主启动时会启用/继承该策略），此时穿越非管理员创建的 Junction 会被
/// 直接拒绝（os error 448，ERROR_UNTRUSTED_MOUNT_POINT）。这里只读取链接自身的
/// reparse 数据（read_link 不穿越），再对解析出的真实目标路径继续操作。
fn resolve_without_traversal(path: &Path) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    for _ in 0..16 {
        let meta = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if !meta.file_type().is_symlink() {
            return Ok(current);
        }
        let target = fs::read_link(&current).map_err(|e| e.to_string())?;
        let target = strip_device_prefix(&target);
        if target == current {
            return Err(format!("检测到循环链接：{}", path.display()));
        }
        current = if target.is_absolute() {
            target
        } else {
            current
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf()
                .join(target)
        };
    }
    Err(format!("链接层级过深：{}", path.display()))
}

fn copy_snapshot(src: &Path, dst: &Path, stack: &mut HashSet<PathBuf>) -> Result<(), String> {
    let resolved = resolve_without_traversal(src)
        .map_err(|e| format!("无法读取 {} 指向的内容：{e}", src.display()))?;
    if !stack.insert(resolved.clone()) {
        return Err(format!("检测到循环链接：{}", src.display()));
    }
    let result = (|| {
        let meta = fs::metadata(&resolved).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            fs::create_dir_all(dst).map_err(|e| e.to_string())?;
            let mut entries = fs::read_dir(&resolved)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                copy_snapshot(&entry.path(), &dst.join(entry.file_name()), stack)?;
            }
        } else if meta.is_file() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&resolved, dst).map_err(|e| e.to_string())?;
        } else {
            return Err(format!("不支持的文件类型：{}", src.display()));
        }
        Ok(())
    })();
    stack.remove(&resolved);
    result
}

fn hash_entry(path: &Path) -> Result<String, String> {
    fn walk(
        path: &Path,
        relative: &Path,
        hasher: &mut Sha256,
        stack: &mut HashSet<PathBuf>,
    ) -> Result<(), String> {
        let resolved = resolve_without_traversal(path).map_err(|e| e.to_string())?;
        if !stack.insert(resolved.clone()) {
            return Err(format!("检测到循环链接：{}", path.display()));
        }
        let meta = fs::metadata(&resolved).map_err(|e| e.to_string())?;
        hasher.update(relative.to_string_lossy().as_bytes());
        if meta.is_dir() {
            hasher.update(b"D");
            let mut entries = fs::read_dir(&resolved)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                walk(
                    &entry.path(),
                    &relative.join(entry.file_name()),
                    hasher,
                    stack,
                )?;
            }
        } else {
            hasher.update(b"F");
            let mut file = fs::File::open(&resolved).map_err(|e| e.to_string())?;
            let mut buffer = [0u8; 16 * 1024];
            loop {
                let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        }
        stack.remove(&resolved);
        Ok(())
    }

    let mut hasher = Sha256::new();
    walk(path, Path::new("."), &mut hasher, &mut HashSet::new())?;
    Ok(hex::encode(hasher.finalize()))
}

fn unique_sibling(path: &Path, suffix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("item");
    parent.join(format!(
        ".{name}.pathmux-{suffix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn replace_with_snapshot(src: &Path, dst: &Path) -> Result<String, String> {
    let parent = dst.parent().ok_or("目标目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = unique_sibling(dst, "next");
    let backup = unique_sibling(dst, "previous");
    copy_snapshot(src, &temp, &mut HashSet::new())?;
    let expected = hash_entry(src)?;
    if hash_entry(&temp)? != expected {
        let _ = remove_entry(&temp);
        return Err("复制后的内容校验不一致，已保持旧版本".into());
    }

    let existed = fs::symlink_metadata(dst).is_ok();
    if existed {
        fs::rename(dst, &backup).map_err(|e| {
            let _ = remove_entry(&temp);
            format!("暂存旧版本失败：{e}")
        })?;
    }
    if let Err(e) = fs::rename(&temp, dst) {
        if existed {
            let _ = fs::rename(&backup, dst);
        }
        let _ = remove_entry(&temp);
        return Err(format!("启用新版本失败，已恢复旧版本：{e}"));
    }
    if existed {
        if let Err(e) = remove_entry(&backup) {
            return Err(format!("新版本已生效，但旧版本备份清理失败：{e}"));
        }
    }
    Ok(expected)
}

struct DirectoryDetach {
    root: PathBuf,
    backup: Option<PathBuf>,
}

impl DirectoryDetach {
    fn commit(mut self) -> Result<(), String> {
        if let Some(backup) = self.backup.take() {
            remove_entry(&backup).map_err(|e| format!("新目录已生效，但旧链接清理失败：{e}"))?;
        }
        Ok(())
    }

    fn rollback(mut self) -> Result<bool, String> {
        let Some(backup) = self.backup.take() else {
            return Ok(false);
        };
        remove_entry(&self.root)?;
        fs::rename(&backup, &self.root).map_err(|e| format!("恢复旧链接失败：{e}"))?;
        Ok(true)
    }
}

/// 准备把升级前的整目录链接转换成真实目录。调用方必须显式 commit 或 rollback。
/// 在此之前旧链接会留在同级备份位置，任何后续步骤失败都能恢复。
fn prepare_directory_detach(root: &Path) -> Result<DirectoryDetach, String> {
    let meta = match fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(root).map_err(|e| e.to_string())?;
            return Ok(DirectoryDetach {
                root: root.to_path_buf(),
                backup: None,
            });
        }
        Err(e) => return Err(e.to_string()),
    };
    if !meta.file_type().is_symlink() {
        if meta.is_dir() {
            return Ok(DirectoryDetach {
                root: root.to_path_buf(),
                backup: None,
            });
        }
        return Err(format!("{} 被同名文件占用", root.display()));
    }
    let temp = unique_sibling(root, "detached");
    copy_snapshot(root, &temp, &mut HashSet::new())?;
    let backup = unique_sibling(root, "link");
    fs::rename(root, &backup).map_err(|e| {
        let _ = remove_entry(&temp);
        format!("暂存旧链接失败：{e}")
    })?;
    if let Err(e) = fs::rename(&temp, root) {
        let _ = fs::rename(&backup, root);
        let _ = remove_entry(&temp);
        return Err(format!("切换逐项共享失败，已恢复旧链接：{e}"));
    }
    Ok(DirectoryDetach {
        root: root.to_path_buf(),
        backup: Some(backup),
    })
}

/// 把升级前的整目录链接安全转换成真实目录。先完整复制，再切换；失败时原链接不动。
fn detach_directory_link(root: &Path) -> Result<(), String> {
    prepare_directory_detach(root)?.commit()
}

fn sync_target(
    kind: Kind,
    shared: &Path,
    target: &Target,
    state: &mut ResourceState,
) -> Result<Vec<String>, String> {
    detach_directory_link(&target.root)?;
    let shared_names = item_names(shared)?;
    let current_names = item_names(&target.root)?;
    let exclusions = state
        .excluded
        .get(kind.key())
        .and_then(|by_target| by_target.get(&target.id))
        .cloned()
        .unwrap_or_default();
    let before = state
        .distributed
        .get(kind.key())
        .and_then(|by_target| by_target.get(&target.id))
        .cloned()
        .unwrap_or_default();
    let mut names = shared_names
        .iter()
        .chain(current_names.iter())
        .chain(before.keys())
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    let mut next = BTreeMap::new();
    let mut warnings = vec![];

    for name in names {
        let src = shared.join(&name);
        let dst = target.root.join(&name);
        let shared_exists = fs::symlink_metadata(&src).is_ok();
        let current_exists = fs::symlink_metadata(&dst).is_ok();
        let shared_hash = if shared_exists {
            match hash_entry(&src) {
                Ok(hash) => Some(hash),
                Err(e) => {
                    warnings.push(format!(
                        "共享 {}「{}」无法读取，本轮未改动 {}：{}",
                        kind.label(),
                        name,
                        target.label,
                        e
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        let current_hash = if current_exists {
            match hash_entry(&dst) {
                Ok(hash) => Some(hash),
                Err(e) => {
                    warnings.push(format!(
                        "{} 的 {}「{}」无法读取，本轮未覆盖：{}",
                        target.label,
                        kind.label(),
                        name,
                        e
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        let previous = before.get(&name);

        if exclusions.contains(&name) {
            if let (Some(current), Some(previous)) = (&current_hash, previous) {
                if current == previous {
                    remove_entry(&dst)?;
                }
            }
            continue;
        }

        match (shared_hash, current_hash, previous) {
            (Some(shared_hash), Some(current_hash), _) if shared_hash == current_hash => {
                next.insert(name, shared_hash);
            }
            (Some(_), Some(current_hash), Some(previous)) if &current_hash == previous => {
                let written = replace_with_snapshot(&src, &dst)?;
                next.insert(name, written);
            }
            (Some(_), Some(_), _) => warnings.push(format!(
                "{} 的 {}「{}」使用自己的版本，共享更新未覆盖",
                target.label,
                kind.label(),
                name
            )),
            (Some(_), None, Some(previous)) => {
                next.insert(name.clone(), previous.clone());
                warnings.push(format!(
                    "{} 已删除 {}「{}」，保持排除状态",
                    target.label,
                    kind.label(),
                    name
                ));
            }
            (Some(_), None, None) => {
                let written = replace_with_snapshot(&src, &dst)?;
                next.insert(name, written);
            }
            (None, Some(current_hash), Some(previous)) if &current_hash == previous => {
                remove_entry(&dst)?;
            }
            (None, Some(_), _) => {}
            (None, None, _) => {}
        }
    }
    *state.ledger_mut(kind, &target.id) = next;
    Ok(warnings)
}

/// 旧迁移会把默认 Claude 中的顶层链接原样重建到共享库。新模型要求共享库拥有
/// 独立快照，因此逐项把这些链接替换为真实文件/目录；默认 Claude 的目标保持不动。
fn materialize_shared_links(kind: Kind, shared: &Path) -> Vec<String> {
    let mut warnings = vec![];
    let names = match item_names(shared) {
        Ok(names) => names,
        Err(e) => return vec![e],
    };
    for name in names {
        let path = shared.join(&name);
        let is_link = fs::symlink_metadata(&path)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if !is_link {
            continue;
        }
        if let Err(e) = replace_with_snapshot(&path, &path) {
            warnings.push(format!(
                "共享 {}「{}」仍是外部链接，转为独立副本失败：{}",
                kind.label(),
                name,
                e
            ));
        }
    }
    warnings
}

fn sync_kind_at(
    kind: Kind,
    shared: &Path,
    targets: &[Target],
    state: &mut ResourceState,
) -> Vec<String> {
    let mut warnings = vec![];
    if let Err(e) = fs::create_dir_all(shared) {
        return vec![format!("创建共享 {} 目录失败：{e}", kind.label())];
    }
    warnings.extend(materialize_shared_links(kind, shared));
    for target in targets {
        match sync_target(kind, shared, target, state) {
            Ok(mut messages) => warnings.append(&mut messages),
            Err(e) => warnings.push(format!("{}：{e}", target.label)),
        }
    }
    warnings
}

#[derive(Default)]
struct ImportRun {
    added: usize,
    skipped: usize,
    failures: Vec<String>,
}

fn auto_import_kind_at(
    kind: Kind,
    source_root: &Path,
    destination_root: &Path,
    ignored: &BTreeSet<String>,
) -> ImportRun {
    let mut run = ImportRun::default();
    if let Err(e) = fs::create_dir_all(destination_root) {
        run.failures
            .push(format!("创建共享 {} 目录失败：{e}", kind.label()));
        return run;
    }
    let names = match item_names(source_root) {
        Ok(names) => names,
        Err(e) => {
            run.failures.push(e);
            return run;
        }
    };
    for name in names {
        if ignored.contains(&name) || fs::symlink_metadata(destination_root.join(&name)).is_ok() {
            run.skipped += 1;
            continue;
        }
        let source = source_root.join(&name);
        if let Err(e) = validate_item(kind, &source)
            .and_then(|_| replace_with_snapshot(&source, &destination_root.join(&name)).map(|_| ()))
        {
            run.failures
                .push(format!("自动导入 {}「{}」失败：{}", kind.label(), name, e));
        } else {
            run.added += 1;
        }
    }
    run
}

pub(crate) fn sync_all_locked(names: &[String]) -> Vec<String> {
    let mut state = match read_state() {
        Ok(state) => state,
        Err(e) => return vec![e],
    };
    let mut warnings = vec![];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if state.auto_import_enabled
        && now.saturating_sub(state.last_auto_import) >= AUTO_IMPORT_INTERVAL_SECS
    {
        let mut record = AutoImportRecord {
            checked_at: now,
            ..Default::default()
        };
        for kind in [Kind::Skills, Kind::Agents] {
            let ignored = state
                .ignored_imports
                .get(kind.key())
                .cloned()
                .unwrap_or_default();
            let run = auto_import_kind_at(kind, &default_root(kind), &shared_root(kind), &ignored);
            record.added += run.added;
            record.skipped += run.skipped;
            record.failures.extend(run.failures);
        }
        if record.failures.is_empty() {
            state.last_auto_import = now;
        }
        warnings.extend(record.failures.iter().cloned());
        state.last_auto_import_record = Some(record);
    }
    for kind in [Kind::Skills, Kind::Agents] {
        warnings.extend(sync_kind_at(
            kind,
            &shared_root(kind),
            &targets(kind, names),
            &mut state,
        ));
    }
    if let Err(e) = save_state(&state) {
        warnings.push(e);
    }
    warnings
}

fn sync_extension_resources_blocking() -> Result<String, String> {
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let warnings = sync_all_locked(&crate::configured_profile_names());
    if warnings.is_empty() {
        Ok("Skills 与 Agents 已更新到所有目标".into())
    } else {
        Ok(format!(
            "更新完成，{} 项需要留意：{}",
            warnings.len(),
            warnings.join("；")
        ))
    }
}

#[tauri::command]
pub(crate) async fn sync_extension_resources() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(sync_extension_resources_blocking)
        .await
        .map_err(|e| format!("扩展同步任务异常：{e}"))?
}

fn status_for(
    kind: Kind,
    name: &str,
    shared: Option<&str>,
    target: &Target,
    state: &ResourceState,
) -> ResourceTargetState {
    let excluded = state
        .excluded
        .get(kind.key())
        .and_then(|v| v.get(&target.id))
        .is_some_and(|v| v.contains(name));
    let current = hash_entry(&target.root.join(name)).ok();
    let previous = state
        .distributed
        .get(kind.key())
        .and_then(|v| v.get(&target.id))
        .and_then(|v| v.get(name));
    let (mut status, mut reason) = if excluded {
        ("excluded", "该目标已明确排除共享版本")
    } else if let (Some(shared), Some(current)) = (shared, current.as_deref()) {
        if shared == current {
            ("inherited", "正在使用共享版本")
        } else {
            ("override", "目标中存在自己的同名版本")
        }
    } else if shared.is_some() && current.is_none() && previous.is_some() {
        ("excluded", "目标删除了应用此前分发的版本")
    } else if shared.is_some() {
        ("missing", "共享版本尚未写入该目标")
    } else if current.is_some() {
        ("local", "仅存在于该目标")
    } else {
        ("missing", "该目标没有此项")
    };
    let mut issues = vec![];
    if kind == Kind::Agents && current.is_some() && status != "excluded" {
        issues = missing_agent_dependencies(&target.root.join(name), &target.root);
        if !issues.is_empty() {
            status = "unavailable";
            reason = "缺少 Agent 需要的 Skill 或 MCP";
        }
    }
    ResourceTargetState {
        target: target.id.clone(),
        label: target.label.clone(),
        state: status.into(),
        reason: reason.into(),
        issues,
    }
}

fn resource_overview_blocking(kind: String) -> Result<ResourceOverview, String> {
    let kind = Kind::parse(&kind)?;
    let shared = shared_root(kind);
    let default = default_root(kind);
    let state = read_state()?;
    let targets = targets(kind, &crate::configured_profile_names());
    let shared_names = item_names(&shared)?;
    let default_names = item_names(&default)?;
    let mut names = shared_names
        .iter()
        .chain(default_names.iter())
        .cloned()
        .collect::<Vec<_>>();
    for target in &targets {
        names.extend(item_names(&target.root)?);
    }
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup();

    let mut items = vec![];
    for name in names {
        let source = shared.join(&name);
        let shared_hash = hash_entry(&source).ok();
        items.push(ResourceItem {
            in_shared: shared_hash.is_some(),
            in_default_claude: default_names.contains(&name),
            targets: targets
                .iter()
                .map(|target| status_for(kind, &name, shared_hash.as_deref(), target, &state))
                .collect(),
            name,
        });
    }

    Ok(ResourceOverview {
        kind: kind.key().into(),
        label: kind.label().into(),
        shared_path: shared.display().to_string(),
        items,
        targets: targets
            .iter()
            .map(|target| ResourceTargetState {
                target: target.id.clone(),
                label: target.label.clone(),
                state: "target".into(),
                reason: target.root.display().to_string(),
                issues: vec![],
            })
            .collect(),
        auto_import_enabled: state.auto_import_enabled,
        last_auto_import_at: state
            .last_auto_import_record
            .as_ref()
            .map(|record| record.checked_at),
        last_auto_import_added: state
            .last_auto_import_record
            .as_ref()
            .map(|record| record.added)
            .unwrap_or_default(),
        last_auto_import_skipped: state
            .last_auto_import_record
            .as_ref()
            .map(|record| record.skipped)
            .unwrap_or_default(),
        last_auto_import_failures: state
            .last_auto_import_record
            .as_ref()
            .map(|record| record.failures.clone())
            .unwrap_or_default(),
    })
}

#[tauri::command]
pub(crate) async fn resource_overview(kind: String) -> Result<ResourceOverview, String> {
    tauri::async_runtime::spawn_blocking(move || resource_overview_blocking(kind))
        .await
        .map_err(|e| format!("扩展读取任务异常：{e}"))?
}

fn frontmatter_value(frontmatter: &str, key: &str) -> Option<String> {
    frontmatter.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&format!("{key}:"))
            .map(str::trim)
            .map(|value| value.trim_matches(['\'', '"']).to_string())
    })
}

fn frontmatter_value_list(text: &str, key: &str) -> Vec<String> {
    let mut values = vec![];
    let mut in_frontmatter = false;
    let mut collecting = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{key}:")) {
            collecting = true;
            let rest = rest.trim().trim_matches(['[', ']']);
            values.extend(
                rest.split(',')
                    .map(|value| value.trim().trim_matches(['\'', '"']))
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            );
            continue;
        }
        if collecting {
            if let Some(value) = trimmed.strip_prefix('-') {
                let value = value.trim().trim_matches(['\'', '"']);
                if !value.is_empty() {
                    values.push(value.into());
                }
            } else if !line.starts_with(' ') && !line.starts_with('\t') {
                collecting = false;
            }
        }
    }
    values.sort();
    values.dedup();
    values
}

fn missing_agent_dependencies(agent: &Path, agents_root: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(agent) else {
        return vec!["Agent 文件无法读取".into()];
    };
    let claude_root = agents_root.parent().unwrap_or(Path::new("."));
    let mut missing = vec![];
    for skill in frontmatter_value_list(&text, "skills") {
        if !claude_root
            .join("skills")
            .join(&skill)
            .join("SKILL.md")
            .is_file()
        {
            missing.push(format!("缺少 Skill：{skill}"));
        }
    }
    let mcp_names = frontmatter_value_list(&text, "tools")
        .into_iter()
        .flat_map(|tool| {
            tool.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter_map(|token| token.strip_prefix("mcp__").map(str::to_string))
        .filter_map(|rest| rest.split("__").next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .collect::<BTreeSet<_>>();
    if !mcp_names.is_empty() {
        let configured = fs::read_to_string(claude_root.join(".claude.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|doc| {
                doc.get("mcpServers")
                    .and_then(|value| value.as_object())
                    .cloned()
            })
            .unwrap_or_default();
        for name in mcp_names {
            if !configured.contains_key(&name) {
                missing.push(format!("缺少 MCP：{name}"));
            }
        }
    }
    missing
}

#[tauri::command]
pub(crate) fn set_resource_auto_import(enabled: bool) -> Result<String, String> {
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let mut state = read_state()?;
    state.auto_import_enabled = enabled;
    save_state(&state)?;
    Ok(if enabled {
        "已开启自动导入；应用启动时每周检查两次，只带回新增项".into()
    } else {
        "已关闭自动导入；手动导入仍可使用".into()
    })
}

#[tauri::command]
pub(crate) fn plugin_targets() -> Vec<String> {
    crate::configured_profile_names()
}

pub(crate) fn plugin_exclusions() -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    Ok(read_state()?.plugin_excluded)
}

fn set_plugin_excluded_blocking(env: String, plugin: String) -> Result<String, String> {
    if !crate::claude_cli::valid_plugin_identifier(&plugin) {
        return Err("插件名称格式不正确".into());
    }
    if !crate::configured_profile_names().contains(&env) {
        return Err("请选择有效的受管理环境".into());
    }
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    if !crate::configured_profile_names().contains(&env) {
        return Err("该环境已被删除，请刷新后重试".into());
    }
    let paths = [state_path(), crate::shared_config::ledger_path()];
    with_files_rollback(&paths, || {
        let mut state = read_state()?;
        crate::shared_config::mark_plugin_override_locked(&env, &plugin)?;
        state
            .plugin_excluded
            .entry(env.clone())
            .or_default()
            .insert(plugin.clone());
        save_state(&state)?;
        Ok(format!("环境「{env}」已排除插件「{plugin}」的共享策略"))
    })
}

fn restore_plugin_policy_blocking(
    env: String,
    plugin: String,
    clear_exclusion: bool,
) -> Result<String, String> {
    if !crate::claude_cli::valid_plugin_identifier(&plugin) {
        return Err("插件名称格式不正确".into());
    }
    if !crate::configured_profile_names().contains(&env) {
        return Err("请选择有效的受管理环境".into());
    }
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    if !crate::configured_profile_names().contains(&env) {
        return Err("该环境已被删除，请刷新后重试".into());
    }
    let shared = crate::shared_config::load_shared(crate::shared_config::FIELD_PLUGINS)?;
    let enabled = shared
        .get(&plugin)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format!("共享策略里已经没有插件「{plugin}」"))?;

    let config_dir = crate::sync::instance_dir(&env);
    let detach = prepare_directory_detach(&config_dir.join("plugins"))?;
    let action = if enabled { "enable" } else { "disable" };
    if let Err(error) = crate::claude_cli::run_plugin_action(&config_dir, action, &plugin) {
        return match detach.rollback() {
            Ok(true) => Err(format!("{error}；该环境的旧插件目录已恢复")),
            Ok(false) => Err(error),
            Err(rollback) => Err(format!("{error}；并且插件目录回退失败：{rollback}")),
        };
    }
    detach.commit()?;

    let paths = [state_path(), crate::shared_config::ledger_path()];
    let metadata = with_files_rollback(&paths, || {
        crate::shared_config::restore_plugin_after_cli_locked(&env, &plugin)?;
        if clear_exclusion {
            let mut state = read_state()?;
            if let Some(items) = state.plugin_excluded.get_mut(&env) {
                items.remove(&plugin);
            }
            state.plugin_excluded.retain(|_, items| !items.is_empty());
            save_state(&state)?;
        }
        Ok(())
    });
    match metadata {
        Ok(()) => Ok(format!(
            "环境「{env}」已通过 Claude Code 官方命令恢复插件「{plugin}」的共享策略"
        )),
        Err(error) => Err(format!(
            "插件已按共享策略{}，但 PathMux 策略台账更新失败：{error}",
            if enabled { "启用" } else { "停用" }
        )),
    }
}

#[tauri::command]
pub(crate) async fn set_plugin_excluded(
    env: String,
    plugin: String,
    excluded: bool,
) -> Result<String, String> {
    if excluded {
        tauri::async_runtime::spawn_blocking(move || set_plugin_excluded_blocking(env, plugin))
            .await
            .map_err(|e| format!("插件策略任务异常：{e}"))?
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            restore_plugin_policy_blocking(env, plugin, true)
        })
        .await
        .map_err(|e| format!("插件策略任务异常：{e}"))?
    }
}

pub(crate) async fn restore_plugin_inheritance(
    env: String,
    plugin: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || restore_plugin_policy_blocking(env, plugin, false))
        .await
        .map_err(|e| format!("插件策略任务异常：{e}"))?
}

fn with_files_rollback<T>(
    paths: &[PathBuf],
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let snapshots = paths
        .iter()
        .map(|path| match fs::read(path) {
            Ok(bytes) => Ok((path.clone(), Some(bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((path.clone(), None)),
            Err(e) => Err(format!("备份 {} 失败：{e}", path.display())),
        })
        .collect::<Result<Vec<_>, String>>()?;
    match action() {
        Ok(value) => Ok(value),
        Err(error) => {
            let failures = snapshots
                .iter()
                .rev()
                .filter_map(|(path, bytes)| restore_optional_file(path, bytes.as_deref()).err())
                .collect::<Vec<_>>();
            if failures.is_empty() {
                Err(format!("{error}；本次策略修改已回退"))
            } else {
                Err(format!("{error}；部分回退失败：{}", failures.join("；")))
            }
        }
    }
}

fn import_default_resource_blocking(
    kind: String,
    name: Option<String>,
    replace: bool,
) -> Result<String, String> {
    let kind = Kind::parse(&kind)?;
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let source_root = default_root(kind);
    let destination_root = shared_root(kind);
    fs::create_dir_all(&destination_root).map_err(|e| e.to_string())?;
    let names = match name {
        Some(name) => {
            if !valid_item_name(&name) {
                return Err("扩展名称不安全".into());
            }
            vec![name]
        }
        None => item_names(&source_root)?,
    };
    let mut state = read_state()?;
    let ignored = state
        .ignored_imports
        .get(kind.key())
        .cloned()
        .unwrap_or_default();
    let mut added = vec![];
    let mut skipped = vec![];
    for name in names {
        if ignored.contains(&name) && !replace {
            skipped.push(name);
            continue;
        }
        let source = source_root.join(&name);
        validate_item(kind, &source)?;
        let destination = destination_root.join(&name);
        if destination.exists() && !replace {
            skipped.push(name);
            continue;
        }
        replace_with_snapshot(&source, &destination)?;
        state
            .ignored_imports
            .entry(kind.key().into())
            .or_default()
            .remove(&name);
        added.push(name);
    }
    let mut warnings = sync_kind_at(
        kind,
        &destination_root,
        &targets(kind, &crate::configured_profile_names()),
        &mut state,
    );
    save_state(&state)?;
    let mut message = format!("已导入 {} 项，跳过 {} 项", added.len(), skipped.len());
    if !warnings.is_empty() {
        message.push_str(&format!(
            "；{} 项目标需要留意：{}",
            warnings.len(),
            warnings.join("；")
        ));
        warnings.clear();
    }
    Ok(message)
}

#[tauri::command]
pub(crate) async fn import_default_resource(
    kind: String,
    name: Option<String>,
    replace: bool,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        import_default_resource_blocking(kind, name, replace)
    })
    .await
    .map_err(|e| format!("默认资源导入任务异常：{e}"))?
}

fn install_resource_from_path_blocking(
    kind: String,
    path: String,
    target_id: Option<String>,
) -> Result<String, String> {
    let kind = Kind::parse(&kind)?;
    let source = PathBuf::from(path);
    validate_item(kind, &source)?;
    let name = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("无法从所选路径确定名称")?
        .to_string();
    if !valid_item_name(&name) {
        return Err("扩展名称不安全".into());
    }
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let mut state = read_state()?;

    if let Some(target_id) = target_id {
        let names = crate::configured_profile_names();
        let available = targets(kind, &names);
        let target = available
            .iter()
            .find(|target| target.id == target_id)
            .ok_or("目标不存在")?;
        detach_directory_link(&target.root)?;
        let destination = target.root.join(&name);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(format!(
                "{} 已存在同名 {}「{}」，未覆盖",
                target.label,
                kind.label(),
                name
            ));
        }
        replace_with_snapshot(&source, &destination)?;
        state.ledger_mut(kind, &target_id).remove(&name);
        state.exclusions_mut(kind, &target_id).remove(&name);
        save_state(&state)?;
        Ok(format!(
            "已把 {}「{}」安装到 {}",
            kind.label(),
            name,
            target.label
        ))
    } else {
        let destination_root = shared_root(kind);
        fs::create_dir_all(&destination_root).map_err(|e| e.to_string())?;
        let destination = destination_root.join(&name);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(format!(
                "共享库已存在同名 {}「{}」，未覆盖",
                kind.label(),
                name
            ));
        }
        replace_with_snapshot(&source, &destination)?;
        state
            .ignored_imports
            .entry(kind.key().into())
            .or_default()
            .remove(&name);
        let warnings = sync_kind_at(
            kind,
            &destination_root,
            &targets(kind, &crate::configured_profile_names()),
            &mut state,
        );
        save_state(&state)?;
        if warnings.is_empty() {
            Ok(format!(
                "已把 {}「{}」安装到共享库并更新所有目标",
                kind.label(),
                name
            ))
        } else {
            Ok(format!(
                "共享 {}「{}」已安装；{}",
                kind.label(),
                name,
                warnings.join("；")
            ))
        }
    }
}

#[tauri::command]
pub(crate) async fn install_resource_from_path(
    kind: String,
    path: String,
    target_id: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        install_resource_from_path_blocking(kind, path, target_id)
    })
    .await
    .map_err(|e| format!("扩展安装任务异常：{e}"))?
}

fn set_resource_excluded_blocking(
    kind: String,
    target_id: String,
    name: String,
    excluded: bool,
) -> Result<String, String> {
    let kind = Kind::parse(&kind)?;
    if !valid_item_name(&name) {
        return Err("扩展名称不安全".into());
    }
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let names = crate::configured_profile_names();
    let available = targets(kind, &names);
    let target = available
        .iter()
        .find(|target| target.id == target_id)
        .ok_or("目标不存在")?;
    let mut state = read_state()?;
    if excluded {
        state.exclusions_mut(kind, &target_id).insert(name.clone());
    } else {
        state.exclusions_mut(kind, &target_id).remove(&name);
    }
    let warnings = sync_target(kind, &shared_root(kind), target, &mut state)?;
    save_state(&state)?;
    if warnings.is_empty() {
        Ok(if excluded {
            "已排除共享版本"
        } else {
            "已恢复共享版本"
        }
        .into())
    } else {
        Ok(warnings.join("；"))
    }
}

#[tauri::command]
pub(crate) async fn set_resource_excluded(
    kind: String,
    target_id: String,
    name: String,
    excluded: bool,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_resource_excluded_blocking(kind, target_id, name, excluded)
    })
    .await
    .map_err(|e| format!("扩展排除任务异常：{e}"))?
}

fn restore_resource_inheritance_blocking(
    kind: String,
    target_id: String,
    name: String,
) -> Result<String, String> {
    let kind = Kind::parse(&kind)?;
    if !valid_item_name(&name) {
        return Err("扩展名称不安全".into());
    }
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let names = crate::configured_profile_names();
    let available = targets(kind, &names);
    let target = available
        .iter()
        .find(|target| target.id == target_id)
        .ok_or("目标不存在")?;
    let source = shared_root(kind).join(&name);
    validate_item(kind, &source)?;
    detach_directory_link(&target.root)?;
    let written = replace_with_snapshot(&source, &target.root.join(&name))?;
    let mut state = read_state()?;
    state.exclusions_mut(kind, &target_id).remove(&name);
    state.ledger_mut(kind, &target_id).insert(name, written);
    save_state(&state)?;
    Ok("已恢复使用共享版本".into())
}

#[tauri::command]
pub(crate) async fn restore_resource_inheritance(
    kind: String,
    target_id: String,
    name: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        restore_resource_inheritance_blocking(kind, target_id, name)
    })
    .await
    .map_err(|e| format!("扩展恢复任务异常：{e}"))?
}

fn delete_shared_resource_blocking(kind: String, name: String) -> Result<String, String> {
    let kind = Kind::parse(&kind)?;
    if !valid_item_name(&name) {
        return Err("扩展名称不安全".into());
    }
    let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let source = shared_root(kind).join(&name);
    if fs::symlink_metadata(&source).is_err() {
        return Err("共享库中不存在这一项".into());
    }
    remove_entry(&source)?;
    let mut state = read_state()?;
    state
        .ignored_imports
        .entry(kind.key().into())
        .or_default()
        .insert(name);
    let warnings = sync_kind_at(
        kind,
        &shared_root(kind),
        &targets(kind, &crate::configured_profile_names()),
        &mut state,
    );
    save_state(&state)?;
    if warnings.is_empty() {
        Ok("已从共享库删除；环境自己的版本保持不变".into())
    } else {
        Ok(format!("共享项已删除；{}", warnings.join("；")))
    }
}

#[tauri::command]
pub(crate) async fn delete_shared_resource(kind: String, name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || delete_shared_resource_blocking(kind, name))
        .await
        .map_err(|e| format!("共享扩展删除任务异常：{e}"))?
}

pub(crate) fn problems(names: &[String]) -> Vec<String> {
    let mut out = vec![];
    for kind in [Kind::Skills, Kind::Agents] {
        for target in targets(kind, names) {
            match fs::symlink_metadata(&target.root) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    out.push(format!("{} 仍使用升级前的整目录链接", target.label))
                }
                Ok(meta) if !meta.is_dir() => out.push(format!(
                    "{} 的 {} 目录被同名文件占用",
                    target.label,
                    kind.label()
                )),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    out.push(format!("{} 尚未建立 {} 目录", target.label, kind.label()))
                }
                Err(e) => out.push(format!("{}：{e}", target.label)),
                _ => {}
            }
        }
    }
    for name in names {
        let plugins = crate::sync::instance_dir(name).join("plugins");
        if fs::symlink_metadata(&plugins)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            out.push(format!(
                "Claude 环境 {name} 仍使用升级前的共享插件目录，应用会在后台重装并迁移"
            ));
        }
    }
    out
}

fn installed_plugin_names(config_dir: &Path) -> Result<Vec<String>, String> {
    let path = config_dir.join("plugins").join("installed_plugins.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("读取旧插件安装记录失败：{e}")),
    };
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("旧插件安装记录不是有效 JSON：{e}"))?;
    let plugins = doc
        .get("plugins")
        .and_then(serde_json::Value::as_object)
        .ok_or("旧插件安装记录缺少 plugins 对象")?;
    let mut names = plugins.keys().cloned().collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn restore_optional_file(path: &Path, before: Option<&[u8]>) -> Result<(), String> {
    match before {
        Some(bytes) => crate::sync::write_bytes_atomic(path, bytes)
            .map_err(|e| format!("恢复 {} 失败：{e}", path.display())),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("清理 {} 失败：{e}", path.display())),
        },
    }
}

fn migrate_legacy_plugin_root<F>(config_dir: &Path, mut install: F) -> Result<bool, String>
where
    F: FnMut(&Path, &str) -> Result<String, String>,
{
    let plugin_root = config_dir.join("plugins");
    let is_link = fs::symlink_metadata(&plugin_root)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok(false);
    }

    let settings_path = config_dir.join("settings.json");
    let settings_before = fs::read(&settings_path).ok();
    let detach = prepare_directory_detach(&plugin_root)?;
    let result = (|| -> Result<(), String> {
        let names = installed_plugin_names(config_dir)?;
        if names.is_empty() {
            return Ok(());
        }

        // 旧账本可能把 installPath 指向另一个环境。暂存它，让 Claude Code 按当前
        // CLAUDE_CONFIG_DIR 重新建立真实账本；旧缓存保留，可由官方安装器自行复用。
        let ledger = plugin_root.join("installed_plugins.json");
        let legacy_ledger = unique_sibling(&ledger, "legacy-ledger");
        fs::rename(&ledger, &legacy_ledger).map_err(|e| format!("暂存旧插件账本失败：{e}"))?;
        for name in &names {
            install(config_dir, name)?;
        }
        let installed = installed_plugin_names(config_dir)?;
        let missing = names
            .iter()
            .filter(|name| !installed.contains(name))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!("官方安装完成后仍缺少：{}", missing.join("、")));
        }
        restore_optional_file(&settings_path, settings_before.as_deref())?;
        remove_entry(&legacy_ledger)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            detach.commit()?;
            Ok(true)
        }
        Err(error) => {
            let settings_rollback =
                restore_optional_file(&settings_path, settings_before.as_deref());
            let directory_rollback = detach.rollback();
            match (settings_rollback, directory_rollback) {
                (Ok(()), Ok(true)) => Err(format!("{error}；已恢复升级前的插件目录")),
                (Ok(()), Ok(false)) => Err(error),
                (settings, directory) => Err(format!(
                    "{error}；回退不完整：设置={}，目录={}",
                    settings.err().unwrap_or_else(|| "成功".into()),
                    directory.err().unwrap_or_else(|| "成功".into())
                )),
            }
        }
    }
}

fn migrate_legacy_plugin_env(env: &str) -> Result<bool, String> {
    let config_dir = crate::sync::instance_dir(env);
    migrate_legacy_plugin_root(&config_dir, |config_dir, plugin| {
        crate::claude_cli::run_plugin_action(config_dir, "install", plugin)
    })
}

/// 软件升级后的后台迁移。每个环境独立执行、独立回退；失败不会影响其他环境，
/// 下次启动会再次尝试，直到所有插件目录都成为独立目录。
pub(crate) fn migrate_legacy_plugins_blocking() -> Vec<String> {
    let mut notes = vec![];
    for env in crate::configured_profile_names() {
        let Some(_guard) = crate::sync::acquire_config_lock() else {
            notes.push(format!("环境 {env} 插件迁移等待其他配置操作，下次启动重试"));
            continue;
        };
        if !crate::configured_profile_names().contains(&env) {
            continue;
        }
        match migrate_legacy_plugin_env(&env) {
            Ok(true) => notes.push(format!("环境 {env} 已退出旧共享插件目录")),
            Ok(false) => {}
            Err(e) => notes.push(format!("环境 {env} 插件迁移未完成：{e}")),
        }
    }
    notes
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginActionOutcome {
    env: String,
    ok: bool,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginActionReport {
    action: String,
    plugin: String,
    results: Vec<PluginActionOutcome>,
    reload_hint: String,
    policy_warning: Option<String>,
}

#[derive(Deserialize)]
struct ImportedPluginManifest {
    name: String,
}

struct PluginPackageTemp {
    root: PathBuf,
}

impl PluginPackageTemp {
    fn new(label: &str) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "pathmux-plugin-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| format!("系统时间异常：{e}"))?
                .as_nanos()
        ));
        fs::create_dir_all(&root).map_err(|e| format!("创建插件临时目录失败：{e}"))?;
        Ok(Self { root })
    }
}

impl Drop for PluginPackageTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn validate_plugin_package_url(value: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(value).map_err(|_| "插件地址格式不正确")?;
    if parsed.scheme() != "https" {
        return Err("远程插件包只允许使用 HTTPS 地址".into());
    }
    match parsed.host().ok_or("插件地址缺少主机名")? {
        url::Host::Domain(host)
            if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") =>
        {
            return Err("插件地址不能指向本机或局域网主机".into());
        }
        url::Host::Ipv4(ip) if ip.is_private() || ip.is_loopback() || ip.is_link_local() => {
            return Err("插件地址不能指向本机或私有网络".into());
        }
        url::Host::Ipv6(ip) if ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local() => {
            return Err("插件地址不能指向本机或私有网络".into());
        }
        _ => {}
    }
    Ok(parsed)
}

fn download_plugin_package(url: &str, temp: &PluginPackageTemp) -> Result<PathBuf, String> {
    let requested = validate_plugin_package_url(url)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("创建插件下载请求失败：{e}"))?;
    let mut response = client
        .get(requested)
        .send()
        .map_err(|e| format!("下载插件包失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("下载插件包失败：{e}"))?;
    validate_plugin_package_url(response.url().as_str())?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_PLUGIN_PACKAGE_BYTES)
    {
        return Err("插件包超过 256 MiB，已拒绝下载".into());
    }
    let path = temp.root.join("plugin.zip");
    let mut file = fs::File::create(&path).map_err(|e| format!("创建插件临时文件失败：{e}"))?;
    let copied = std::io::copy(
        &mut response.by_ref().take(MAX_PLUGIN_PACKAGE_BYTES + 1),
        &mut file,
    )
    .map_err(|e| format!("保存插件包失败：{e}"))?;
    if copied > MAX_PLUGIN_PACKAGE_BYTES {
        return Err("插件包超过 256 MiB，已停止保存".into());
    }
    file.flush().map_err(|e| format!("保存插件包失败：{e}"))?;
    Ok(path)
}

fn extract_plugin_package(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive_path).map_err(|e| format!("读取插件包失败：{e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("插件包不是有效的 ZIP：{e}"))?;
    if archive.len() > MAX_PLUGIN_PACKAGE_FILES {
        return Err("插件包文件数量过多，已拒绝解压".into());
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("读取插件包条目失败：{e}"))?;
        let relative = entry
            .enclosed_name()
            .ok_or("插件包包含不安全的文件路径")?
            .to_path_buf();
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("插件包不能包含符号链接".into());
        }
        total = total.saturating_add(entry.size());
        if total > MAX_PLUGIN_PACKAGE_BYTES {
            return Err("插件包解压后超过 256 MiB，已停止处理".into());
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|e| format!("创建插件目录失败：{e}"))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建插件目录失败：{e}"))?;
        }
        let mut target = fs::File::create(&output).map_err(|e| format!("创建插件文件失败：{e}"))?;
        std::io::copy(&mut entry, &mut target).map_err(|e| format!("解压插件文件失败：{e}"))?;
    }
    Ok(())
}

fn find_imported_plugin_root(root: &Path) -> Result<PathBuf, String> {
    let direct = root.join(".claude-plugin").join("plugin.json");
    if direct.is_file() {
        return Ok(root.to_path_buf());
    }
    let mut matches = vec![];
    for entry in fs::read_dir(root).map_err(|e| format!("读取插件包失败：{e}"))? {
        let path = entry.map_err(|e| format!("读取插件包失败：{e}"))?.path();
        if path.is_dir() && path.join(".claude-plugin").join("plugin.json").is_file() {
            matches.push(path);
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err("插件包内未找到 .claude-plugin/plugin.json".into()),
        _ => Err("插件包内包含多个插件，请分别打包后安装".into()),
    }
}

fn copy_plugin_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|e| format!("创建插件缓存目录失败：{e}"))?;
    for entry in fs::read_dir(source).map_err(|e| format!("读取插件目录失败：{e}"))? {
        let entry = entry.map_err(|e| format!("读取插件目录失败：{e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("读取插件文件类型失败：{e}"))?;
        let destination_path = destination.join(entry.file_name());
        if kind.is_symlink() {
            return Err("插件包不能包含符号链接".into());
        }
        if kind.is_dir() {
            copy_plugin_tree(&entry.path(), &destination_path)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination_path)
                .map_err(|e| format!("复制插件文件失败：{e}"))?;
        }
    }
    Ok(())
}

fn stage_imported_plugin(source: &str) -> Result<(String, String, PathBuf), String> {
    let temp = PluginPackageTemp::new("import")?;
    let archive = if url::Url::parse(source)
        .is_ok_and(|parsed| parsed.scheme().eq_ignore_ascii_case("https"))
    {
        download_plugin_package(source, &temp)?
    } else {
        let path = PathBuf::from(source);
        if !path.is_file() {
            return Err("本地插件包不存在或不是文件".into());
        }
        if path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("zip"))
            != Some(true)
        {
            return Err("本地插件包必须是 .zip 文件".into());
        }
        path
    };
    let extracted = temp.root.join("extracted");
    fs::create_dir_all(&extracted).map_err(|e| format!("创建插件解压目录失败：{e}"))?;
    extract_plugin_package(&archive, &extracted)?;
    let plugin_root = find_imported_plugin_root(&extracted)?;
    let manifest_path = plugin_root.join(".claude-plugin").join("plugin.json");
    let manifest: ImportedPluginManifest = serde_json::from_str(
        &fs::read_to_string(&manifest_path).map_err(|e| format!("读取 plugin.json 失败：{e}"))?,
    )
    .map_err(|e| format!("plugin.json 格式不正确：{e}"))?;
    if !crate::claude_cli::valid_plugin_identifier(&manifest.name)
        || manifest.name.contains(['@', '/'])
    {
        return Err("plugin.json 中的插件名称格式不正确".into());
    }
    let digest = hex::encode(Sha256::digest(
        format!("{source}\n{}", manifest.name).as_bytes(),
    ));
    let marketplace = format!("pathmux-import-{}", &digest[..12]);
    let root = crate::cfg_dir().join("shared").join("plugin-marketplaces");
    fs::create_dir_all(&root).map_err(|e| format!("创建插件 Marketplace 目录失败：{e}"))?;
    let final_dir = root.join(&marketplace);
    let staging = root.join(format!(".{marketplace}.staging"));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| format!("清理插件暂存目录失败：{e}"))?;
    }
    let staged_plugin = staging.join("plugin");
    copy_plugin_tree(&plugin_root, &staged_plugin)?;
    let catalog_dir = staging.join(".claude-plugin");
    fs::create_dir_all(&catalog_dir).map_err(|e| format!("创建 Marketplace 清单目录失败：{e}"))?;
    let catalog = serde_json::json!({
        "name": marketplace.clone(),
        "owner": { "name": "PathMux local import" },
        "plugins": [{ "name": manifest.name.clone(), "source": "./plugin" }]
    });
    fs::write(
        catalog_dir.join("marketplace.json"),
        serde_json::to_vec_pretty(&catalog)
            .map_err(|e| format!("生成 Marketplace 清单失败：{e}"))?,
    )
    .map_err(|e| format!("保存 Marketplace 清单失败：{e}"))?;
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|e| format!("替换旧插件缓存失败：{e}"))?;
    }
    fs::rename(&staging, &final_dir).map_err(|e| format!("启用插件缓存失败：{e}"))?;
    Ok((manifest.name, marketplace, final_dir))
}

fn install_plugin_package_blocking(
    source: String,
    envs: Vec<String>,
    shared_scope: bool,
) -> Result<PluginActionReport, String> {
    let configured = crate::configured_profile_names();
    let mut requested = envs;
    requested.sort();
    requested.dedup();
    if requested.is_empty() || requested.iter().any(|env| !configured.contains(env)) {
        return Err("请选择有效的受管理环境".into());
    }
    let covers_all =
        requested.len() == configured.len() && configured.iter().all(|env| requested.contains(env));
    if shared_scope && !covers_all {
        return Err("所有环境安装的目标列表不完整，请刷新后重试".into());
    }
    let (name, marketplace, marketplace_dir) = stage_imported_plugin(source.trim())?;
    let plugin = format!("{name}@{marketplace}");

    let mut prepared = vec![];
    let mut failures = vec![];
    for env in &requested {
        let result = (|| -> Result<(), String> {
            let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
            let config_dir = crate::sync::instance_dir(env);
            let _ = crate::claude_cli::remove_plugin_marketplace(&config_dir, &marketplace);
            crate::claude_cli::add_plugin_marketplace(&config_dir, &marketplace_dir)?;
            Ok(())
        })();
        match result {
            Ok(()) => prepared.push(env.clone()),
            Err(detail) => failures.push(PluginActionOutcome {
                env: env.clone(),
                ok: false,
                detail,
            }),
        }
    }

    let mut report = if prepared.is_empty() {
        PluginActionReport {
            action: "install".into(),
            plugin: plugin.clone(),
            results: vec![],
            reload_hint: plugin_reload_hint("install").into(),
            policy_warning: None,
        }
    } else {
        manage_plugin_blocking(
            "install".into(),
            plugin.clone(),
            prepared,
            shared_scope && failures.is_empty(),
        )?
    };
    report.results.extend(failures);
    report
        .results
        .sort_by(|left, right| left.env.cmp(&right.env));
    Ok(report)
}

fn manage_plugin_blocking(
    action: String,
    plugin: String,
    envs: Vec<String>,
    shared_scope: bool,
) -> Result<PluginActionReport, String> {
    if !matches!(
        action.as_str(),
        "install" | "update" | "uninstall" | "enable" | "disable"
    ) {
        return Err("不支持的插件操作".into());
    }
    if !crate::claude_cli::valid_plugin_identifier(&plugin) {
        return Err("插件名称格式不正确".into());
    }
    let configured = crate::configured_profile_names();
    let mut requested = envs;
    requested.sort();
    requested.dedup();
    if requested.is_empty() || requested.iter().any(|env| !configured.contains(env)) {
        return Err("请选择有效的受管理环境".into());
    }

    let covers_all =
        requested.len() == configured.len() && configured.iter().all(|env| requested.contains(env));
    if shared_scope && !covers_all {
        return Err("所有环境操作的目标列表不完整，请刷新后重试".into());
    }
    let exclusions = read_state()?.plugin_excluded;
    let mut results = vec![];
    let mut applied_envs = vec![];
    for env in &requested {
        if should_skip_shared_plugin_action(&exclusions, shared_scope, env, &plugin) {
            results.push(PluginActionOutcome {
                env: env.clone(),
                ok: true,
                detail: "该环境已排除共享策略，本次批量操作已跳过".into(),
            });
            continue;
        }
        let result = (|| -> Result<String, String> {
            let _guard = crate::sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
            if !crate::configured_profile_names().contains(env) {
                return Err("该环境已被删除，请刷新后重试".into());
            }
            let config_dir = crate::sync::instance_dir(env);
            // 存量版本可能把 plugins 整个目录链接到共享位置。先复制成该环境自己的
            // 真实目录并保留旧链接备份。官方命令失败时恢复旧结构，不留下半迁移状态。
            let detach = prepare_directory_detach(&config_dir.join("plugins"))?;
            match crate::claude_cli::run_plugin_action(&config_dir, &action, &plugin) {
                Ok(detail) => {
                    detach.commit()?;
                    Ok(detail)
                }
                Err(error) => match detach.rollback() {
                    Ok(true) => Err(format!("{error}；该环境的旧插件目录已恢复")),
                    Ok(false) => Err(error),
                    Err(rollback) => Err(format!("{error}；并且插件目录回退失败：{rollback}")),
                },
            }
        })();
        match result {
            Ok(detail) => {
                applied_envs.push(env.clone());
                results.push(PluginActionOutcome {
                    env: env.clone(),
                    ok: true,
                    detail,
                });
            }
            Err(detail) => results.push(PluginActionOutcome {
                env: env.clone(),
                ok: false,
                detail,
            }),
        }
    }

    let all_succeeded = results.iter().all(|result| result.ok);
    let mut policy_warning = None;
    if all_succeeded {
        let enabled = matches!(action.as_str(), "install" | "enable" | "update");
        if matches!(
            action.as_str(),
            "install" | "enable" | "disable" | "uninstall"
        ) {
            if shared_scope {
                if let Err(e) = crate::shared_config::record_shared_plugin_after_cli(
                    &plugin,
                    enabled && action != "uninstall",
                    &applied_envs,
                ) {
                    policy_warning = Some(format!("插件操作成功，但共享启停策略更新失败：{e}"));
                }
            } else {
                for env in &applied_envs {
                    if let Err(e) = crate::shared_config::record_env_plugin_after_cli(
                        env,
                        &plugin,
                        enabled && action != "uninstall",
                    ) {
                        policy_warning = Some(format!(
                            "插件操作成功，但环境 {env} 的启停策略更新失败：{e}"
                        ));
                        break;
                    }
                }
            }
        }
        // 明确指定某个环境执行插件操作，表示用户正在为这个环境作独立决定；
        // 操作成功后解除旧的“排除共享批量操作”标记。所有环境操作则保留排除。
        if !shared_scope {
            let clear_result = (|| -> Result<(), String> {
                let _guard =
                    crate::sync::acquire_config_lock().ok_or("配置正在同步，排除状态暂未更新")?;
                let mut state = read_state()?;
                for env in &requested {
                    if let Some(items) = state.plugin_excluded.get_mut(env) {
                        items.remove(&plugin);
                    }
                }
                state.plugin_excluded.retain(|_, items| !items.is_empty());
                save_state(&state)
            })();
            if let Err(e) = clear_result {
                let next = format!("插件操作成功，但排除状态更新失败：{e}");
                policy_warning = Some(match policy_warning.take() {
                    Some(previous) => format!("{previous}；{next}"),
                    None => next,
                });
            }
        }
    }

    Ok(PluginActionReport {
        reload_hint: plugin_reload_hint(&action).into(),
        action,
        plugin,
        results,
        policy_warning,
    })
}

fn should_skip_shared_plugin_action(
    exclusions: &BTreeMap<String, BTreeSet<String>>,
    all_selected: bool,
    env: &str,
    plugin: &str,
) -> bool {
    all_selected
        && exclusions
            .get(env)
            .is_some_and(|plugins| plugins.contains(plugin))
}

fn plugin_reload_hint(action: &str) -> &'static str {
    if action == "update" {
        "插件已更新；Claude Code 官方要求重启会话后应用新版本。"
    } else {
        "新会话直接生效；已经打开的会话执行 /reload-plugins。包含 monitors 的插件需要重启 Claude Code 会话。"
    }
}

#[tauri::command]
pub(crate) async fn manage_plugin(
    action: String,
    plugin: String,
    envs: Vec<String>,
    shared_scope: bool,
) -> Result<PluginActionReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        manage_plugin_blocking(action, plugin, envs, shared_scope)
    })
    .await
    .map_err(|e| format!("插件任务异常：{e}"))?
}

#[tauri::command]
pub(crate) async fn install_plugin_package(
    source: String,
    envs: Vec<String>,
    shared_scope: bool,
) -> Result<PluginActionReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        install_plugin_package_blocking(source, envs, shared_scope)
    })
    .await
    .map_err(|e| format!("插件安装任务异常：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "pathmux-extensions-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn skill(root: &Path, name: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        for (name, body) in entries {
            archive
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(body.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn plugin_package_urls_require_public_https_hosts() {
        assert!(validate_plugin_package_url("https://plugins.example.com/demo.zip").is_ok());
        let private_v4 = format!("https://{}.{}.{}.{}/demo.zip", 192, 168, 1, 20);
        for blocked in [
            "http://plugins.example.com/demo.zip".into(),
            "https://localhost/demo.zip".into(),
            "https://devbox.local/demo.zip".into(),
            "https://127.0.0.1/demo.zip".into(),
            private_v4,
            "https://[::1]/demo.zip".into(),
        ] {
            assert!(
                validate_plugin_package_url(&blocked).is_err(),
                "should reject {blocked}"
            );
        }
    }

    #[test]
    fn plugin_package_extraction_rejects_path_traversal() {
        let temp = Temp::new("plugin-zip-traversal");
        let archive = temp.0.join("plugin.zip");
        write_zip(&archive, &[("../escaped.txt", "unsafe")]);
        let destination = temp.0.join("extracted");
        fs::create_dir_all(&destination).unwrap();

        let error = extract_plugin_package(&archive, &destination).unwrap_err();

        assert!(error.contains("不安全"));
        assert!(!temp.0.join("escaped.txt").exists());
    }

    #[test]
    fn plugin_package_finds_a_single_wrapped_manifest() {
        let temp = Temp::new("plugin-zip-valid");
        let archive = temp.0.join("plugin.zip");
        write_zip(
            &archive,
            &[
                ("demo/.claude-plugin/plugin.json", r#"{"name":"demo"}"#),
                ("demo/README.md", "Demo plugin"),
            ],
        );
        let destination = temp.0.join("extracted");
        fs::create_dir_all(&destination).unwrap();

        extract_plugin_package(&archive, &destination).unwrap();
        let root = find_imported_plugin_root(&destination).unwrap();

        assert_eq!(root, destination.join("demo"));
        assert!(root.join(".claude-plugin/plugin.json").is_file());
    }

    #[test]
    fn shared_update_follows_inherited_but_preserves_override() {
        let temp = Temp::new("override");
        let shared = temp.0.join("shared");
        let a = temp.0.join("a");
        let b = temp.0.join("b");
        skill(&shared, "reviewer", "v1");
        let targets = vec![
            Target {
                id: "a".into(),
                label: "a".into(),
                root: a.clone(),
            },
            Target {
                id: "b".into(),
                label: "b".into(),
                root: b.clone(),
            },
        ];
        let mut state = ResourceState::default().normalized();
        assert!(sync_kind_at(Kind::Skills, &shared, &targets, &mut state).is_empty());
        fs::write(b.join("reviewer/SKILL.md"), "mine").unwrap();
        fs::write(shared.join("reviewer/SKILL.md"), "v2").unwrap();

        let warnings = sync_kind_at(Kind::Skills, &shared, &targets, &mut state);
        assert_eq!(
            fs::read_to_string(a.join("reviewer/SKILL.md")).unwrap(),
            "v2"
        );
        assert_eq!(
            fs::read_to_string(b.join("reviewer/SKILL.md")).unwrap(),
            "mine"
        );
        assert!(warnings
            .iter()
            .any(|message| message.contains("自己的版本")));
    }

    #[test]
    fn explicit_exclusion_removes_only_an_inherited_copy() {
        let temp = Temp::new("exclude");
        let shared = temp.0.join("shared");
        let target_root = temp.0.join("target");
        skill(&shared, "one", "shared");
        let target = Target {
            id: "env".into(),
            label: "env".into(),
            root: target_root.clone(),
        };
        let mut state = ResourceState::default().normalized();
        sync_target(Kind::Skills, &shared, &target, &mut state).unwrap();
        state
            .exclusions_mut(Kind::Skills, "env")
            .insert("one".into());
        sync_target(Kind::Skills, &shared, &target, &mut state).unwrap();
        assert!(!target_root.join("one").exists());

        skill(&target_root, "one", "local");
        sync_target(Kind::Skills, &shared, &target, &mut state).unwrap();
        assert_eq!(
            fs::read_to_string(target_root.join("one/SKILL.md")).unwrap(),
            "local"
        );
    }

    #[test]
    fn deleting_shared_removes_inherited_and_keeps_local() {
        let temp = Temp::new("delete");
        let shared = temp.0.join("shared");
        let inherited = temp.0.join("inherited");
        let local = temp.0.join("local");
        skill(&shared, "one", "shared");
        let targets = vec![
            Target {
                id: "a".into(),
                label: "a".into(),
                root: inherited.clone(),
            },
            Target {
                id: "b".into(),
                label: "b".into(),
                root: local.clone(),
            },
        ];
        let mut state = ResourceState::default().normalized();
        sync_kind_at(Kind::Skills, &shared, &targets, &mut state);
        fs::write(local.join("one/SKILL.md"), "local").unwrap();
        fs::remove_dir_all(shared.join("one")).unwrap();
        sync_kind_at(Kind::Skills, &shared, &targets, &mut state);
        assert!(!inherited.join("one").exists());
        assert_eq!(
            fs::read_to_string(local.join("one/SKILL.md")).unwrap(),
            "local"
        );
    }

    #[test]
    fn snapshot_follows_source_link_without_recreating_external_link() {
        let temp = Temp::new("snapshot-link");
        let external = temp.0.join("external");
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        skill(&external, "linked", "body");
        fs::create_dir_all(&source).unwrap();
        crate::sync::create_directory_link(&external.join("linked"), &source.join("linked"))
            .unwrap();
        replace_with_snapshot(&source.join("linked"), &destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "body"
        );
        assert!(fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_dir());
        assert!(!fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn directory_detach_can_rollback_and_commit_without_touching_source() {
        let temp = Temp::new("detach-transaction");
        let source = temp.0.join("source");
        let target = temp.0.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("data.txt"), "original").unwrap();
        crate::sync::create_directory_link(&source, &target).unwrap();

        let detach = prepare_directory_detach(&target).unwrap();
        assert!(!fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::write(target.join("data.txt"), "changed").unwrap();
        detach.rollback().unwrap();
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(source.join("data.txt")).unwrap(),
            "original"
        );

        let detach = prepare_directory_detach(&target).unwrap();
        detach.commit().unwrap();
        assert!(!fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(target.join("data.txt")).unwrap(),
            "original"
        );
        assert_eq!(
            fs::read_to_string(source.join("data.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    fn legacy_plugin_migration_reinstalls_into_independent_directory_and_restores_settings() {
        let temp = Temp::new("plugin-migration");
        let source = temp.0.join("old-shared-plugins");
        let config = temp.0.join("environment/.claude");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::write(
            source.join("installed_plugins.json"),
            r#"{"version":2,"plugins":{"demo@market":[{"version":"1.0.0","installPath":"old"}]}}"#,
        )
        .unwrap();
        fs::write(
            config.join("settings.json"),
            r#"{"enabledPlugins":{"demo@market":false}}"#,
        )
        .unwrap();
        crate::sync::create_directory_link(&source, &config.join("plugins")).unwrap();

        let migrated = migrate_legacy_plugin_root(&config, |target, plugin| {
            assert_eq!(plugin, "demo@market");
            fs::write(target.join("settings.json"), r#"{"enabledPlugins":{"demo@market":true}}"#)
                .unwrap();
            fs::write(
                target.join("plugins/installed_plugins.json"),
                r#"{"version":2,"plugins":{"demo@market":[{"version":"1.0.0","installPath":"new"}]}}"#,
            )
            .unwrap();
            Ok("installed".into())
        })
        .unwrap();

        assert!(migrated);
        assert!(!fs::symlink_metadata(config.join("plugins"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(source.join("installed_plugins.json").is_file());
        assert_eq!(
            fs::read_to_string(config.join("settings.json")).unwrap(),
            r#"{"enabledPlugins":{"demo@market":false}}"#
        );
    }

    #[test]
    fn legacy_plugin_migration_rolls_back_link_when_ledger_is_invalid() {
        let temp = Temp::new("plugin-migration-rollback");
        let source = temp.0.join("old-shared-plugins");
        let config = temp.0.join("environment/.claude");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::write(source.join("installed_plugins.json"), "not json").unwrap();
        crate::sync::create_directory_link(&source, &config.join("plugins")).unwrap();

        let error =
            migrate_legacy_plugin_root(&config, |_, _| Ok("unexpected".into())).unwrap_err();
        assert!(error.contains("有效 JSON"));
        assert!(fs::symlink_metadata(config.join("plugins"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(config.join("plugins/installed_plugins.json")).unwrap(),
            "not json"
        );
    }

    #[test]
    fn legacy_shared_link_is_materialized_without_changing_its_source() {
        let temp = Temp::new("materialize");
        let external = temp.0.join("external");
        let shared = temp.0.join("shared");
        skill(&external, "linked", "original");
        fs::create_dir_all(&shared).unwrap();
        crate::sync::create_directory_link(&external.join("linked"), &shared.join("linked"))
            .unwrap();

        assert!(materialize_shared_links(Kind::Skills, &shared).is_empty());
        assert!(!fs::symlink_metadata(shared.join("linked"))
            .unwrap()
            .file_type()
            .is_symlink());
        fs::write(external.join("linked/SKILL.md"), "changed outside").unwrap();
        assert_eq!(
            fs::read_to_string(shared.join("linked/SKILL.md")).unwrap(),
            "original"
        );
    }

    #[test]
    fn unreadable_shared_link_never_looks_like_a_delete() {
        let temp = Temp::new("dangling-shared");
        let external = temp.0.join("external");
        let shared = temp.0.join("shared");
        let target_root = temp.0.join("target");
        skill(&external, "one", "shared");
        fs::create_dir_all(&shared).unwrap();
        crate::sync::create_directory_link(&external.join("one"), &shared.join("one")).unwrap();
        skill(&target_root, "one", "shared");
        let previous = hash_entry(&target_root.join("one")).unwrap();
        let mut state = ResourceState::default().normalized();
        state
            .ledger_mut(Kind::Skills, "env")
            .insert("one".into(), previous);
        fs::remove_dir_all(&external).unwrap();

        let target = Target {
            id: "env".into(),
            label: "env".into(),
            root: target_root.clone(),
        };
        let warnings = sync_target(Kind::Skills, &shared, &target, &mut state).unwrap();
        assert!(target_root.join("one/SKILL.md").is_file());
        assert!(
            warnings.iter().any(|message| message.contains("无法读取")),
            "{warnings:?}"
        );
    }

    #[test]
    fn automatic_import_adds_only_missing_and_respects_ignored_items() {
        let temp = Temp::new("auto-import");
        let source = temp.0.join("default");
        let shared = temp.0.join("shared");
        skill(&source, "new", "from default");
        skill(&source, "existing", "default version");
        skill(&source, "ignored", "do not restore");
        skill(&shared, "existing", "shared version");
        let ignored = BTreeSet::from(["ignored".to_string()]);

        let run = auto_import_kind_at(Kind::Skills, &source, &shared, &ignored);
        assert!(run.failures.is_empty());
        assert_eq!(run.added, 1);
        assert_eq!(run.skipped, 2);
        assert_eq!(
            fs::read_to_string(shared.join("new/SKILL.md")).unwrap(),
            "from default"
        );
        assert_eq!(
            fs::read_to_string(shared.join("existing/SKILL.md")).unwrap(),
            "shared version"
        );
        assert!(!shared.join("ignored").exists());
    }

    #[test]
    fn v1_state_keeps_distribution_records_and_enables_auto_import() {
        let mut state = ResourceState {
            version: 1,
            ..ResourceState::default()
        };
        state
            .ledger_mut(Kind::Skills, "corp")
            .insert("reviewer".into(), "hash".into());
        let state = state.normalized();
        assert_eq!(state.version, STATE_VERSION);
        assert!(state.auto_import_enabled);
        assert_eq!(state.distributed["skills"]["corp"]["reviewer"], "hash");
    }

    #[test]
    fn plugin_exclusion_only_skips_shared_batch_operations() {
        let exclusions = BTreeMap::from([(
            "corp".to_string(),
            BTreeSet::from(["demo@market".to_string()]),
        )]);
        assert!(should_skip_shared_plugin_action(
            &exclusions,
            true,
            "corp",
            "demo@market"
        ));
        assert!(!should_skip_shared_plugin_action(
            &exclusions,
            false,
            "corp",
            "demo@market"
        ));
        assert!(!should_skip_shared_plugin_action(
            &exclusions,
            true,
            "test",
            "demo@market"
        ));
    }

    #[test]
    fn plugin_reload_hint_requires_restart_for_updates() {
        assert!(plugin_reload_hint("update").contains("重启会话"));
        assert!(!plugin_reload_hint("update").contains("/reload-plugins"));
        assert!(plugin_reload_hint("install").contains("/reload-plugins"));
        assert!(plugin_reload_hint("enable").contains("/reload-plugins"));
    }

    #[test]
    fn plugin_policy_restore_runs_official_cli_before_updating_its_ledger() {
        let source = include_str!("extensions.rs");
        let start = source.find("fn restore_plugin_policy_blocking").unwrap();
        let end = source[start..]
            .find("#[tauri::command]")
            .map(|offset| start + offset)
            .unwrap();
        let body = &source[start..end];
        let official = body.find("run_plugin_action").unwrap();
        let ledger = body.find("restore_plugin_after_cli_locked").unwrap();
        assert!(official < ledger, "必须先完成官方插件命令，再更新策略台账");

        let cancel_exclusion = &source[end..];
        assert!(cancel_exclusion.contains("restore_plugin_policy_blocking(env, plugin, true)"));
    }

    #[test]
    fn file_scanning_commands_remain_off_the_ui_runtime() {
        let source = include_str!("extensions.rs");
        for name in [
            "sync_extension_resources",
            "resource_overview",
            "import_default_resource",
            "install_resource_from_path",
            "set_resource_excluded",
            "restore_resource_inheritance",
            "delete_shared_resource",
        ] {
            assert!(
                source.contains(&format!("pub(crate) async fn {name}")),
                "{name} 必须保持异步命令"
            );
        }
    }

    #[test]
    fn removing_environment_clears_all_extension_records_for_that_name() {
        let mut state = ResourceState::default().normalized();
        state
            .ledger_mut(Kind::Skills, "te")
            .insert("reviewer".into(), "hash".into());
        state
            .exclusions_mut(Kind::Agents, "te")
            .insert("tester.md".into());
        state
            .plugin_excluded
            .entry("te".into())
            .or_default()
            .insert("demo@market".into());
        state
            .ledger_mut(Kind::Skills, "keep")
            .insert("x".into(), "y".into());
        assert!(state_has_target(&state, "te"));

        forget_target_from_state(&mut state, "te");

        assert!(!state_has_target(&state, "te"));
        assert!(state_has_target(&state, "keep"));
    }

    #[test]
    fn agent_dependencies_are_reported_without_installing_anything() {
        let temp = Temp::new("agent-deps");
        let claude = temp.0.join(".claude");
        fs::create_dir_all(claude.join("agents")).unwrap();
        fs::write(
            claude.join("agents/reviewer.md"),
            "---\ndescription: review\nskills: [review-skill]\ntools: [mcp__git__status]\n---\n",
        )
        .unwrap();
        let missing =
            missing_agent_dependencies(&claude.join("agents/reviewer.md"), &claude.join("agents"));
        assert!(missing.contains(&"缺少 Skill：review-skill".to_string()));
        assert!(missing.contains(&"缺少 MCP：git".to_string()));
        assert!(
            !claude.join("skills/review-skill").exists(),
            "检查不得自动安装 Skill"
        );

        skill(&claude.join("skills"), "review-skill", "body");
        fs::write(claude.join(".claude.json"), r#"{"mcpServers":{"git":{}}}"#).unwrap();
        assert!(missing_agent_dependencies(
            &claude.join("agents/reviewer.md"),
            &claude.join("agents")
        )
        .is_empty());
    }

    // 用户手建的目录链接可能用 cmd mklink /J 这类原始方式创建（不经 PowerShell
    // 校验），迁移器必须同样认得。
    fn create_raw_directory_link(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let out = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "mklink 失败：{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    // Windows 进程带 RedirectionGuard 缓解策略时（安装器直接运行新装应用等场景），
    // 穿越非管理员创建的 Junction 会得到 os error 448。迁移器必须改走
    // “读链接目标 → 操作真实路径”，以下测试锁定该解析行为。
    #[test]
    fn resolve_without_traversal_resolves_links_to_real_paths() {
        let temp = Temp::new("resolve-link");
        let real = temp.0.join("real");
        skill(&real, "demo", "body");
        let link = temp.0.join("link");
        create_raw_directory_link(&real, &link);
        let resolved = resolve_without_traversal(&link).unwrap();
        // 不能按文本断言相等：Windows Runner 的 8.3 短路径与 read_link 返回的
        // 长路径可能写法不同，按“已是真实目录且内容可达”断言。
        assert!(!fs::symlink_metadata(&resolved)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(resolved.join("demo").join("SKILL.md").is_file());
        assert_eq!(
            resolve_without_traversal(&real.join("demo")).unwrap(),
            real.join("demo")
        );
    }

    #[test]
    fn resolve_without_traversal_follows_two_hop_links() {
        let temp = Temp::new("resolve-chain");
        let real = temp.0.join("real");
        skill(&real, "demo", "body");
        let second = temp.0.join("second");
        let first = temp.0.join("first");
        create_raw_directory_link(&real, &second);
        create_raw_directory_link(&second, &first);
        let resolved = resolve_without_traversal(&first).unwrap();
        assert!(!fs::symlink_metadata(&resolved)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(resolved.join("demo").join("SKILL.md").is_file());
    }

    #[test]
    fn resolve_without_traversal_rejects_dangling_links() {
        let temp = Temp::new("resolve-dangling");
        let dangling = temp.0.join("dangling");
        create_raw_directory_link(&temp.0.join("missing"), &dangling);
        assert!(resolve_without_traversal(&dangling).is_err());
    }

    #[test]
    fn resolve_without_traversal_rejects_link_cycles() {
        let temp = Temp::new("resolve-cycle");
        let a = temp.0.join("a");
        let b = temp.0.join("b");
        create_raw_directory_link(&b, &a);
        create_raw_directory_link(&a, &b);
        let err = resolve_without_traversal(&a).unwrap_err();
        assert!(err.contains("循环") || err.contains("层级过深"), "{err}");
    }

    #[test]
    fn hash_entry_reads_linked_source_without_traversal() {
        // 升级前的整目录链接迁移要求 copy_snapshot / hash_entry 都接受
        // “源本身是目录链接”的输入。
        let temp = Temp::new("hash-link");
        let real = temp.0.join("real");
        skill(&real, "demo", "body");
        let link = temp.0.join("link");
        create_raw_directory_link(&real, &link);
        assert_eq!(hash_entry(&link).unwrap(), hash_entry(&real).unwrap());
    }

    #[test]
    fn validate_item_reads_linked_skill_without_traversal() {
        let temp = Temp::new("validate-link");
        let real = temp.0.join("real");
        skill(&real, "demo", "body");
        let link = temp.0.join("linked-skill");
        create_raw_directory_link(&real.join("demo"), &link);

        validate_item(Kind::Skills, &link).expect("链接形式的 Skill 应从真实目标校验");
        assert_eq!(
            fs::read_to_string(real.join("demo/SKILL.md")).unwrap(),
            "body",
            "校验不得修改链接目标"
        );
    }

    #[test]
    fn detach_converts_raw_user_made_junction_without_touching_source() {
        // 复刻用户真实场景：plugins 是手建 Junction，且进程可能无法穿越它。
        // 迁移后 link 位置变成真实目录，源目录内容原样保留。
        let temp = Temp::new("detach-raw");
        let real = temp.0.join("real-plugins");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("installed_plugins.json"), "{}").unwrap();
        let link = temp.0.join("plugins");
        create_raw_directory_link(&real, &link);
        detach_directory_link(&link).unwrap();
        assert!(!fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read_to_string(link.join("installed_plugins.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(real.join("installed_plugins.json")).unwrap(),
            "{}",
            "源目录必须原样保留"
        );
    }
}
