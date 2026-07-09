// 全局共享同步:
// 1) ensure_links —— 把每个实例的 skills/plugins/agents/commands 目录用
//    Junction(Windows)/symlink(Unix) 指向主账户 ~/.claude,零配置、幂等。
// 2) sync_configs —— mcpServers(.claude.json)与 enabledPlugins(settings.json)
//    无法用链接共享(CLI 用临时文件+rename 原子改写会顶掉链接),
//    改为基于快照的三方合并:增/删/改都从任一副本正确扩散到全部副本。
// 该模块被 GUI(sync_all 命令)和 CLI 模式(--sync,由 cc.ps1/cc.sh 在每次
// 启动/退出 claude 时调用)共用,--sync 路径下绝不 panic(release 无控制台)。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SHARED_SUBDIRS: [&str; 4] = ["skills", "plugins", "agents", "commands"];

fn master_dir() -> PathBuf {
    crate::home().join(".claude")
}
fn instance_dir(name: &str) -> PathBuf {
    crate::home().join(".claude-split").join(name).join(".claude")
}
fn snapshot_path() -> PathBuf {
    crate::cfg_dir().join("sync-snapshot.json")
}
fn lock_path() -> PathBuf {
    crate::cfg_dir().join("sync.lock")
}
fn log_path() -> PathBuf {
    crate::cfg_dir().join("sync.log")
}

pub fn log_line(msg: &str) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

// ---------------- 目录链接 ----------------

// 链接目标比较前归一化:Windows read_link 返回 \\?\ 前缀且大小写不敏感
fn normalize(p: &Path) -> String {
    let s = p.display().to_string();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

fn link_points_to(dst: &Path, src: &Path) -> bool {
    match fs::read_link(dst) {
        Ok(t) => normalize(&t) == normalize(src),
        Err(_) => false,
    }
}

// Windows 上目录型 reparse point(Junction)用 remove_dir 删链不删目标
fn remove_link(p: &Path) -> std::io::Result<()> {
    if cfg!(windows) {
        fs::remove_dir(p)
    } else {
        fs::remove_file(p)
    }
}

// 旧的真实目录(隔离时代的安装)→ 内容搬进主目录,同名冲突留在原地;
// 搬空则删目录,搬不空则整体改名备份,腾出路径建链。返回备份路径(如有)。
fn migrate_dir(dst: &Path, src: &Path) -> std::io::Result<Option<PathBuf>> {
    for entry in fs::read_dir(dst)? {
        let entry = entry?;
        let to = src.join(entry.file_name());
        if fs::symlink_metadata(&to).is_err() {
            let _ = fs::rename(entry.path(), &to);
        }
    }
    match fs::remove_dir(dst) {
        Ok(_) => Ok(None),
        Err(_) => {
            let mut bak = dst.with_extension("pre-share.bak");
            let mut i = 1;
            while fs::symlink_metadata(&bak).is_ok() {
                bak = dst.with_extension(format!("pre-share.bak{i}"));
                i += 1;
            }
            fs::rename(dst, &bak)?;
            Ok(Some(bak))
        }
    }
}

// 幂等:链接齐全时零开销(纯 fs 检查,不起 PowerShell)
pub fn ensure_links(names: &[String]) -> Result<Vec<String>, String> {
    let master = master_dir();
    for sub in SHARED_SUBDIRS {
        fs::create_dir_all(master.join(sub))
            .map_err(|e| format!("创建 {} 失败:{e}", master.join(sub).display()))?;
    }
    let mut msgs: Vec<String> = vec![];
    // Windows 建链需要 PowerShell,收集后一次批量执行,省冷启开销
    #[cfg(not(unix))]
    let mut jobs: Vec<(PathBuf, PathBuf)> = vec![];
    for name in names {
        if name.is_empty() {
            continue;
        }
        let inst = instance_dir(name);
        if let Err(e) = fs::create_dir_all(&inst) {
            msgs.push(format!("{name}: 创建实例目录失败:{e}"));
            continue;
        }
        for sub in SHARED_SUBDIRS {
            let src = master.join(sub);
            let dst = inst.join(sub);
            match fs::symlink_metadata(&dst) {
                Err(_) => {}
                Ok(m) if m.file_type().is_symlink() => {
                    if link_points_to(&dst, &src) {
                        continue;
                    }
                    if let Err(e) = remove_link(&dst) {
                        msgs.push(format!("{name}/{sub}: 移除旧链接失败:{e}"));
                        continue;
                    }
                }
                Ok(m) if m.is_dir() => match migrate_dir(&dst, &src) {
                    Ok(Some(bak)) => msgs.push(format!(
                        "{name}/{sub}: 旧内容已并入主账户,同名冲突项备份在 {}",
                        bak.display()
                    )),
                    Ok(None) => {}
                    Err(e) => {
                        msgs.push(format!("{name}/{sub}: 旧目录迁移失败({e}),跳过"));
                        continue;
                    }
                },
                Ok(_) => {
                    msgs.push(format!("{name}/{sub}: 已存在同名文件,跳过"));
                    continue;
                }
            }
            #[cfg(unix)]
            {
                if let Err(e) = std::os::unix::fs::symlink(&src, &dst) {
                    msgs.push(format!("{name}/{sub}: 建链失败:{e}"));
                }
            }
            #[cfg(not(unix))]
            {
                jobs.push((dst, src));
            }
        }
    }
    #[cfg(not(unix))]
    {
        if !jobs.is_empty() {
            let inner = jobs
                .iter()
                .map(|(d, s)| {
                    format!(
                        "New-Item -ItemType Junction -Path {} -Target {} | Out-Null",
                        crate::ps_q(&d.display().to_string()),
                        crate::ps_q(&s.display().to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            let out = crate::ps_command()
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &inner])
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                msgs.push(format!(
                    "建立目录联结失败:{}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
    }
    Ok(msgs)
}

// 健康检查用:找出各实例缺失/指错的共享目录链接,只报告不修复
// (修复由 ensure_links 在下次启动时完成)。逻辑与 ensure_links 的判定分支一一对应。
pub fn broken_links(names: &[String]) -> Vec<String> {
    let master = master_dir();
    let mut probs = vec![];
    for name in names {
        if name.is_empty() {
            continue;
        }
        let inst = instance_dir(name);
        if !inst.exists() {
            continue; // 实例从未启动且未建链,不算异常
        }
        for sub in SHARED_SUBDIRS {
            let dst = inst.join(sub);
            match fs::symlink_metadata(&dst) {
                Err(_) => probs.push(format!("{name}/{sub} 链接缺失")),
                Ok(m) if m.file_type().is_symlink() => {
                    if !link_points_to(&dst, &master.join(sub)) {
                        probs.push(format!("{name}/{sub} 链接指向异常"));
                    }
                }
                Ok(m) if m.is_dir() => probs.push(format!("{name}/{sub} 是独立目录（未共享）")),
                Ok(_) => probs.push(format!("{name}/{sub} 被同名文件占用")),
            }
        }
    }
    probs
}

// ---------------- 配置字段合并同步 ----------------

#[derive(Default, Serialize, Deserialize)]
struct DomainSnap {
    #[serde(default)]
    state: Map<String, Value>,
    // 上轮参与写回的副本;不在列表里的副本(新实例)只贡献增/改、不贡献删,
    // 否则新实例首次同步会被解读为"删除了全部条目"并扩散出去。
    #[serde(default)]
    replicas: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Snapshot {
    #[serde(default)]
    domains: BTreeMap<String, DomainSnap>,
}

struct ReplicaState {
    key: String,
    path: PathBuf,
    exists: bool,
    doc: Option<Value>, // None = 文件缺失或 JSON 解析失败 → 本轮不参与 diff
    state: Map<String, Value>,
    mtime: SystemTime,
}

fn read_replica(key: &str, path: &Path, field: &str) -> ReplicaState {
    let exists = path.is_file();
    let doc = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let state = doc
        .as_ref()
        .and_then(|d| d.get(field))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mtime = fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(UNIX_EPOCH);
    ReplicaState {
        key: key.to_string(),
        path: path.to_path_buf(),
        exists,
        doc,
        state,
        mtime,
    }
}

pub(crate) fn write_json_atomic(path: &Path, v: &Value) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(v)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let tmp = path.with_extension("ccm-tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

enum ChangeKind {
    Upsert(Value),
    Remove,
}

// 单个域(mcpServers / enabledPlugins)的一轮合并;返回新快照与写回份数
fn sync_domain(
    field: &str,
    files: &[(String, PathBuf)],
    old: Option<&DomainSnap>,
) -> (DomainSnap, usize) {
    let replicas: Vec<ReplicaState> = files
        .iter()
        .map(|(k, p)| read_replica(k, p, field))
        .collect();
    let participating: Vec<&ReplicaState> =
        replicas.iter().filter(|r| r.doc.is_some()).collect();

    let mut target: Map<String, Value>;
    match old {
        None => {
            // 首次(或快照丢失):按 mtime 旧→新取并集,不做删除判定,防误删
            let mut sorted = participating.clone();
            sorted.sort_by_key(|r| r.mtime);
            target = Map::new();
            for r in sorted {
                for (k, v) in &r.state {
                    target.insert(k.clone(), v.clone());
                }
            }
        }
        Some(snap) => {
            target = snap.state.clone();
            // (key, 变更, 文件 mtime, 稳定序) —— 按 mtime 旧→新应用,新改动覆盖旧改动
            let mut changes: Vec<(String, ChangeKind, SystemTime, usize)> = vec![];
            for (i, r) in participating.iter().enumerate() {
                let first_seen = !snap.replicas.contains(&r.key);
                for (k, v) in &r.state {
                    if snap.state.get(k) != Some(v) {
                        changes.push((k.clone(), ChangeKind::Upsert(v.clone()), r.mtime, i));
                    }
                }
                if !first_seen {
                    for k in snap.state.keys() {
                        if !r.state.contains_key(k) {
                            changes.push((k.clone(), ChangeKind::Remove, r.mtime, i));
                        }
                    }
                }
            }
            changes.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)));
            for (k, kind, _, _) in changes {
                match kind {
                    ChangeKind::Upsert(v) => {
                        target.insert(k, v);
                    }
                    ChangeKind::Remove => {
                        target.remove(&k);
                    }
                }
            }
        }
    }

    // 写回:只动目标字段,文档其余部分(登录态、permissions 等)原样保留
    let mut written = 0usize;
    let mut ok_replicas: Vec<String> = vec![];
    for r in &replicas {
        let dir_ok = r.path.parent().map(|d| d.is_dir()).unwrap_or(false);
        if !dir_ok {
            continue; // 实例目录不存在(从未启动且未建链)→ 跳过
        }
        if r.doc.is_none() && r.exists {
            continue; // 文件在但解析失败:别覆盖,等它自愈
        }
        if r.doc.is_none() && target.is_empty() {
            continue; // 文件不存在且无内容可分发,不凭空建文件
        }
        let cur_equal = r.doc.is_some() && r.state == target;
        if cur_equal {
            ok_replicas.push(r.key.clone());
            continue;
        }
        // 竞态防御:读后被别的进程改过(如 CLI 正在写)→ 本轮跳过,下轮收敛
        if r.exists {
            let now = fs::metadata(&r.path).and_then(|m| m.modified()).ok();
            if now != Some(r.mtime) {
                continue;
            }
        }
        let mut doc = r.doc.clone().unwrap_or_else(|| Value::Object(Map::new()));
        if !doc.is_object() {
            continue;
        }
        doc[field] = Value::Object(target.clone());
        match write_json_atomic(&r.path, &doc) {
            Ok(_) => {
                written += 1;
                ok_replicas.push(r.key.clone());
            }
            Err(e) => log_line(&format!("写回 {} 失败:{e}", r.path.display())),
        }
    }
    (
        DomainSnap {
            state: target,
            replicas: ok_replicas,
        },
        written,
    )
}

// ---------------- 锁 ----------------

struct LockGuard(PathBuf);
impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn acquire_lock() -> Option<LockGuard> {
    let p = lock_path();
    let _ = fs::create_dir_all(crate::cfg_dir());
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                return Some(LockGuard(p.clone()));
            }
            Err(_) => {
                // 超过 60s 视为上次崩溃遗留的陈旧锁,删掉重试一次
                let stale = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .map(|d| d.as_secs() > 60)
                    .unwrap_or(true);
                if stale {
                    let _ = fs::remove_file(&p);
                    continue;
                }
                return None;
            }
        }
    }
    None
}

// ---------------- 入口 ----------------

pub fn sync_configs(names: &[String]) -> Result<String, String> {
    let _guard = match acquire_lock() {
        Some(g) => g,
        None => return Ok("另一个同步正在进行,本轮跳过".into()),
    };
    let snap: Snapshot = fs::read_to_string(snapshot_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // 副本 = 主账户 + 全部实例;两个域分别落在不同文件
    let mut mcp_files: Vec<(String, PathBuf)> =
        vec![("__main__".into(), crate::home().join(".claude.json"))];
    let mut plugin_files: Vec<(String, PathBuf)> =
        vec![("__main__".into(), master_dir().join("settings.json"))];
    for n in names {
        if n.is_empty() {
            continue;
        }
        mcp_files.push((n.clone(), instance_dir(n).join(".claude.json")));
        plugin_files.push((n.clone(), instance_dir(n).join("settings.json")));
    }

    let mut new_snap = Snapshot::default();
    let mut summary: Vec<String> = vec![];
    for (field, files) in [("mcpServers", &mcp_files), ("enabledPlugins", &plugin_files)] {
        let (dsnap, written) = sync_domain(field, files, snap.domains.get(field));
        summary.push(format!("{field} {} 项/写回 {written} 份", dsnap.state.len()));
        new_snap.domains.insert(field.to_string(), dsnap);
    }
    write_json_atomic(
        &snapshot_path(),
        &serde_json::to_value(&new_snap).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写快照失败:{e}"))?;
    Ok(summary.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    // 每个测试用独立临时目录，避免并行跑测试时互相踩文件；Drop 时自动清理。
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "ccm-sync-test-{tag}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            TmpDir(dir)
        }
        fn file(&self, name: &str, content: &str) -> PathBuf {
            let p = self.0.join(name);
            fs::write(&p, content).unwrap();
            p
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sync_domain_first_run_unions_all_replicas() {
        let dir = TmpDir::new("first-run");
        let a = dir.file("a.json", r#"{"mcpServers":{"x":{"a":1}}}"#);
        let b = dir.file("b.json", r#"{"mcpServers":{"y":{"b":2}}}"#);
        let files = vec![("a".to_string(), a.clone()), ("b".to_string(), b.clone())];

        let (snap, _written) = sync_domain("mcpServers", &files, None);

        assert_eq!(snap.state.len(), 2);
        assert!(snap.state.contains_key("x"));
        assert!(snap.state.contains_key("y"));

        // 首轮应把并集写回两边
        let doc_a: Value = serde_json::from_str(&fs::read_to_string(&a).unwrap()).unwrap();
        let merged_a = doc_a.get("mcpServers").unwrap().as_object().unwrap();
        assert!(merged_a.contains_key("x"));
        assert!(merged_a.contains_key("y"));
    }

    #[test]
    fn sync_domain_propagates_deletion_to_other_replicas() {
        let dir = TmpDir::new("delete");
        // a 已经删除了 y，b 仍停留在旧状态
        let a = dir.file("a.json", r#"{"mcpServers":{"x":{"v":1}}}"#);
        let b = dir.file("b.json", r#"{"mcpServers":{"x":{"v":1},"y":{"v":2}}}"#);
        let files = vec![("a".to_string(), a.clone()), ("b".to_string(), b.clone())];

        let mut old_state = Map::new();
        old_state.insert("x".to_string(), serde_json::json!({"v": 1}));
        old_state.insert("y".to_string(), serde_json::json!({"v": 2}));
        let old = DomainSnap {
            state: old_state,
            replicas: vec!["a".to_string(), "b".to_string()],
        };

        let (snap, written) = sync_domain("mcpServers", &files, Some(&old));

        assert_eq!(written, 1, "只有 b 需要改写，a 已经是目标状态");
        assert!(!snap.state.contains_key("y"), "删除应扩散进最终状态");
        assert!(snap.state.contains_key("x"));

        let doc_b: Value = serde_json::from_str(&fs::read_to_string(&b).unwrap()).unwrap();
        let merged_b = doc_b.get("mcpServers").unwrap().as_object().unwrap();
        assert!(!merged_b.contains_key("y"), "y 应已从 b 中删除");
        assert!(merged_b.contains_key("x"));
    }

    #[test]
    fn sync_domain_new_replica_does_not_delete_inherited_keys() {
        let dir = TmpDir::new("new-replica");
        // a 是旧副本、内容不变；c 是新加入的实例，只带了自己的 key
        let a = dir.file("a.json", r#"{"mcpServers":{"x":{"v":1}}}"#);
        let c = dir.file("c.json", r#"{"mcpServers":{"z":{"v":3}}}"#);
        let files = vec![("a".to_string(), a.clone()), ("c".to_string(), c.clone())];

        let mut old_state = Map::new();
        old_state.insert("x".to_string(), serde_json::json!({"v": 1}));
        let old = DomainSnap {
            state: old_state,
            replicas: vec!["a".to_string()],
        };

        let (snap, _written) = sync_domain("mcpServers", &files, Some(&old));

        assert!(
            snap.state.contains_key("x"),
            "老实例的 key 不该因为新实例没有而被当成删除"
        );
        assert!(snap.state.contains_key("z"), "新实例带来的 key 应被采纳");
    }

    #[test]
    fn sync_domain_skips_replica_whose_file_is_unparseable() {
        let dir = TmpDir::new("bad-json");
        let a = dir.file("a.json", r#"{"mcpServers":{"x":{"v":1}}}"#);
        let broken = dir.file("broken.json", "not json");
        let files = vec![
            ("a".to_string(), a.clone()),
            ("broken".to_string(), broken.clone()),
        ];

        let (snap, _written) = sync_domain("mcpServers", &files, None);

        assert!(snap.state.contains_key("x"));
        // 解析失败的副本不应被覆盖，等它自愈
        assert_eq!(fs::read_to_string(&broken).unwrap(), "not json");
    }
}
