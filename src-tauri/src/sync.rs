// 配置同步与旧版共享迁移辅助代码。
//
// `skills/plugins/agents/commands` 整目录链接属于 v2.3.8 及更早版本的迁移结构。
// 新版启动流程不再调用这段迁移；Skills / Agents 由 `crate::extensions` 逐项分发，
// Plugins 按环境交给 Claude Code 官方 CLI 管理，Commands 只保留存量兼容。
//
// sync_configs —— mcpServers 无法用链接共享（CLI 用临时文件+rename 原子改写
//    会顶掉链接），改由 `crate::shared_config` 从**应用自建共享源单向分发**到
//    各环境（决策 7.2）。插件目录和启停由 Claude Code 官方 CLI 管理，不能
//    混进每次 `--sync` 的 JSON 直写流程。
//    原先这里是"基于快照的三方双向合并"，已随该决策退役 —— 它会让单个环境的
//    覆盖传播到其他环境，与"各环境可独立覆盖"冲突。
// 该模块被 GUI(sync_all 命令)和 CLI 模式(--sync,由 cc.ps1/cc.sh 在每次
// 启动/退出**环境**时调用)共用,--sync 路径下绝不 panic(release 无控制台)。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
#[cfg(not(test))]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(not(test))]
use std::time::UNIX_EPOCH;
use std::time::{Duration, SystemTime};

pub const SHARED_SUBDIRS: [&str; 4] = ["skills", "plugins", "agents", "commands"];

/// 应用自建的共享资源根（决策 7.3）。
///
/// 与默认 Claude 的 `~/.claude` **从此互相独立**：默认 Claude 继续用它自己的那份，
/// 受管理环境共享这里的这份。两者之间不再自动互相打通。
pub(crate) fn shared_root() -> PathBuf {
    crate::cfg_dir().join("shared")
}

/// 迁移完成的标记文件。**它同时是"切换开关"**：
/// 只有标记存在时，链接才指向新根 —— 这样"复制没成功"永远不会让环境突然看不到内容。
fn migration_marker() -> PathBuf {
    crate::cfg_dir().join("shared-migrated")
}

#[allow(dead_code)]
pub(crate) fn shared_migration_done() -> bool {
    migration_marker().is_file()
}

/// **首次迁移**（决策 7.3）：把共享资源从默认 Claude 的 `~/.claude` **复制**到应用共享根，
/// 再把各受管理环境的链接切过去。
///
/// 四条硬要求，缺一条就会伤到存量用户：
/// 1. **只复制、不移动、不删除** `~/.claude/` 里的任何东西 —— 那是用户自己的资源，
///    应用对默认 Claude 是只读的；
/// 2. **同名的处理**：内容相同去重；**内容不同不覆盖**，只报告冲突（那可能是用户自己的东西）；
/// 3. **可回滚**：任一环境切链失败就立刻全部切回旧根，保持原状；
/// 4. **复制或切换没成功就不写标记** —— 标记既是"做过"的记录，也是 `master_dir()` 的开关，
///    不写就等于什么都没发生过，下次启动重试。
///
/// 返回给用户看的提示（无事发生时是 None）。
#[allow(dead_code)]
pub(crate) fn migrate_shared_resources_once() -> Option<String> {
    let Some(_guard) = acquire_config_lock() else {
        return Some("共享资源迁移等待其他配置操作完成，下次启动重试".into());
    };
    migrate_shared_resources_at(
        &crate::home().join(".claude"),
        &shared_root(),
        &migration_marker(),
        &crate::home().join(".claude-split"),
        &crate::profile_names(&crate::load()),
    )
}

/// 迁移的**完整实现**，所有路径显式给出 —— 这样它能在临时目录上真跑一遍
/// （含真实建链/切链/回滚），不必拿用户的真实 `~/.claude` 做第一次试验。
pub(crate) fn migrate_shared_resources_at(
    legacy: &Path,
    new_root: &Path,
    marker: &Path,
    instance_root: &Path,
    names: &[String],
) -> Option<String> {
    if marker.is_file() {
        return None;
    }

    // 旧根不存在（全新用户，没有任何东西要迁）→ 直接标记，避免每次启动都重跑
    if !legacy.is_dir() {
        if let Err(e) = fs::create_dir_all(new_root).and_then(|_| fs::write(marker, "1")) {
            return Some(format!("共享资源初始化失败，未完成迁移：{e}"));
        }
        return None;
    }

    let mut conflicts: Vec<String> = vec![];
    let mut skipped: Vec<String> = vec![];
    let mut copied = 0usize;
    let mut deduped = 0usize;
    for sub in SHARED_SUBDIRS {
        let src = legacy.join(sub);
        if !src.is_dir() {
            continue;
        }
        match copy_tree_merge(&src, &new_root.join(sub), &mut conflicts, &mut skipped) {
            Ok((c, d)) => {
                copied += c;
                deduped += d;
            }
            Err(e) => {
                log_line(&format!(
                    "共享资源迁移中止（{sub} 复制失败：{e}），下次启动重试"
                ));
                return Some(format!("共享资源迁移失败、已保持原状：{sub}：{e}"));
            }
        }
    }

    // 有东西没能原样搬过去（例如某个链接重建失败）→ **不切换**。
    // 切换后环境会看不到这些条目，而用户完全不知道少了什么 —— 保持旧状态更安全。
    if !skipped.is_empty() {
        log_line(&format!(
            "共享资源迁移未切换（{} 项无法复制）：{}",
            skipped.len(),
            skipped.join("、")
        ));
        return Some(format!(
            "共享资源迁移未完成、已保持原状：{} 项无法复制到共享库（{}）。\
             修好这些条目后下次启动会重试。",
            skipped.len(),
            skipped.join("、")
        ));
    }

    if !conflicts.is_empty() {
        return Some(format!(
            "共享资源同名但内容不同，未覆盖、未切换，已保持原状：{}",
            conflicts.join("、")
        ));
    }
    if let Err(e) = switch_links_transaction(new_root, instance_root, names, marker) {
        return Some(format!("共享资源迁移未完成：{e}"));
    }
    let mut note = format!(
        "已将现有扩展复制到应用共享库：新增 {copied} 项、去重 {deduped} 项。\
         默认 Claude 的原有文件保持不变；受管理环境今后使用独立的共享资源。"
    );
    if !conflicts.is_empty() {
        note.push_str(&format!(
            " 有 {} 项同名但内容不同，「已保留原文件、未覆盖」，请自行核对：{}",
            conflicts.len(),
            conflicts.join("、")
        ));
    }
    Some(note)
}

/// 共享资源当前该指向哪里。迁移完成前保持旧根（`~/.claude`），行为与改造前完全一致。
#[allow(dead_code)]
fn master_dir() -> PathBuf {
    if shared_migration_done() {
        shared_root()
    } else {
        crate::home().join(".claude")
    }
}

/// **把 src 目录的内容合并复制到 dst**（不移动、不删除源）。
///
/// 同名策略（决策 7.3）：
/// - 目标不存在 → 复制
/// - 存在且内容相同 → 去重跳过
/// - 存在但内容不同 → **不覆盖**，记进 `conflicts`
///
/// **链接会被原样重建，而不是跟随**。
///
/// 这一条是**真机跑出来的**：本机 `~/.claude/skills` 里全是用户自己建的符号链接
/// （指向 `~/.agents/skills/…` 与 `E:\…`）。用 `entry.metadata()` 会**跟随链接**，
/// 于是递归进了 `E:` 上那个目录、撞到"拒绝访问"，**整场迁移被一条链接拖停**。
/// 跟随链接还有更坏的一面：会把用户放在别处的技能**复制成一份副本**，
/// 此后他改原文、副本不再跟随 —— 静默地破坏他的既有安排。
///
/// 单条失败只记进 `skipped`，**不中止整场迁移**：一个奇怪的条目不该让所有人卡住。
/// 返回 (新复制数, 去重数)。
fn copy_tree_merge(
    src: &Path,
    dst: &Path,
    conflicts: &mut Vec<String>,
    skipped: &mut Vec<String>,
) -> std::io::Result<(usize, usize)> {
    let mut copied = 0usize;
    let mut deduped = 0usize;
    if fs::symlink_metadata(dst).is_ok_and(|m| m.file_type().is_symlink() || !m.is_dir()) {
        conflicts.push(dst.display().to_string());
        return Ok((0, 0));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let file_type = entry.file_type()?; // **不跟随链接**

        if file_type.is_symlink() {
            if fs::symlink_metadata(&to).is_ok() {
                if resolved_link_target(&from).ok() == resolved_link_target(&to).ok()
                    && fs::read_link(&to).is_ok()
                {
                    deduped += 1;
                } else {
                    conflicts.push(to.display().to_string());
                }
                continue;
            }
            match recreate_link(&from, &to) {
                Ok(()) => copied += 1,
                Err(e) => skipped.push(format!("{}（{e}）", from.display())),
            }
            continue;
        }
        if file_type.is_dir() {
            let (c, d) = copy_tree_merge(&from, &to, conflicts, skipped)?;
            copied += c;
            deduped += d;
            continue;
        }
        if fs::symlink_metadata(&to).is_ok() {
            let same = match (fs::read(&from), fs::read(&to)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            };
            if same {
                deduped += 1;
            } else {
                conflicts.push(to.display().to_string());
            }
            continue;
        }
        if let Err(e) = fs::copy(&from, &to) {
            skipped.push(format!("{}（{e}）", from.display()));
            continue;
        }
        copied += 1;
    }
    Ok((copied, deduped))
}

/// 在目标处重建与源**相同指向**的链接。
fn resolved_link_target(path: &Path) -> std::io::Result<PathBuf> {
    let target = fs::read_link(path)?;
    Ok(if target.is_absolute() {
        target
    } else {
        path.parent().unwrap_or(Path::new(".")).join(target)
    })
}

pub(crate) fn create_directory_link(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        let script = format!("$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path {} -Target {} | Out-Null",
            crate::ps_q(&link.display().to_string()), crate::ps_q(&target.display().to_string()));
        let out = crate::ps_command()
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
    }
    if !link_points_to(link, target) {
        return Err(format!("链接核验失败：{}", link.display()));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct MigrationLink {
    path: PathBuf,
    backup: PathBuf,
    existed: bool,
}

fn restore_migration_links(entries: &[MigrationLink]) -> Result<(), String> {
    let mut errors = vec![];
    for entry in entries.iter().rev() {
        let result = (|| -> Result<(), String> {
            let has_backup = fs::symlink_metadata(&entry.backup).is_ok();
            if !has_backup && entry.existed {
                return Ok(());
            }
            if let Ok(meta) = fs::symlink_metadata(&entry.path) {
                if !meta.file_type().is_symlink() {
                    return Err(format!(
                        "回滚时路径已被其他程序改动：{}",
                        entry.path.display()
                    ));
                }
                remove_link(&entry.path).map_err(|e| e.to_string())?;
            }
            if has_backup {
                fs::rename(&entry.backup, &entry.path).map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

fn switch_links_transaction(
    master: &Path,
    instance_root: &Path,
    names: &[String],
    marker: &Path,
) -> Result<(), String> {
    let journal = marker.with_extension("pending.json");
    if journal.exists() {
        let entries: Vec<MigrationLink> =
            serde_json::from_slice(&fs::read(&journal).map_err(|e| e.to_string())?)
                .map_err(|e| format!("迁移日志损坏：{e}"))?;
        restore_migration_links(&entries)?;
        fs::remove_file(&journal).map_err(|e| e.to_string())?;
    }
    let mut entries = vec![];
    for name in names {
        if !crate::script_safe_name(name) || name == "." || name == ".." {
            return Err("迁移环境名不安全".into());
        }
        for sub in SHARED_SUBDIRS {
            let path = instance_root.join(name).join(".claude").join(sub);
            let backup = path.with_extension("pre-migration");
            if fs::symlink_metadata(&backup).is_ok() {
                return Err(format!("迁移备份已存在：{}", backup.display()));
            }
            let existed = match fs::symlink_metadata(&path) {
                Ok(m) if m.file_type().is_symlink() => true,
                Ok(m) if m.is_dir() => {
                    let mut conflicts = vec![];
                    let mut skipped = vec![];
                    copy_tree_merge(&path, &master.join(sub), &mut conflicts, &mut skipped)
                        .map_err(|e| e.to_string())?;
                    if !conflicts.is_empty() || !skipped.is_empty() {
                        return Err(format!("环境 {name}/{sub} 有复制冲突，已保持原状"));
                    }
                    true
                }
                Ok(_) => return Err(format!("{} 是文件，已保持原状", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.to_string()),
            };
            entries.push(MigrationLink {
                path,
                backup,
                existed,
            });
        }
    }
    fs::write(
        &journal,
        serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let change = (|| -> Result<(), String> {
        for entry in &entries {
            fs::create_dir_all(entry.path.parent().unwrap()).map_err(|e| e.to_string())?;
            let sub = entry.path.file_name().unwrap();
            fs::create_dir_all(master.join(sub)).map_err(|e| e.to_string())?;
            if entry.existed {
                fs::rename(&entry.path, &entry.backup).map_err(|e| e.to_string())?;
            }
            create_directory_link(&master.join(sub), &entry.path)?;
        }
        fs::write(marker, "1").map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = change {
        restore_migration_links(&entries).map_err(|r| format!("{e}；回滚异常：{r}"))?;
        fs::remove_file(&journal).map_err(|r| r.to_string())?;
        return Err(format!("{e}；已恢复原链接及目录"));
    }
    for entry in entries {
        if fs::read_link(&entry.backup).is_ok() {
            remove_link(&entry.backup).map_err(|e| e.to_string())?;
        }
        // 原有真实目录作为备份保留，禁止递归删除用户内容。
    }
    fs::remove_file(journal).map_err(|e| e.to_string())?;
    Ok(())
}

fn recreate_link(from: &Path, to: &Path) -> std::io::Result<()> {
    let target = resolved_link_target(from)?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, to)
    }
    #[cfg(not(unix))]
    {
        // Windows：目录链接用 Junction（不需要管理员/开发者模式），
        // 文件链接用 symlink_file；两者都不行就交给调用方记进 skipped。
        let is_dir = fs::metadata(from).map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
            let inner = format!(
                "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path {} -Target {} | Out-Null",
                crate::ps_q(&to.display().to_string()),
                crate::ps_q(&target.display().to_string())
            );
            let out = crate::ps_command()
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &inner,
                ])
                .output()?;
            if !out.status.success() {
                return Err(std::io::Error::other(
                    String::from_utf8_lossy(&out.stderr).trim().to_string(),
                ));
            }
            Ok(())
        } else {
            std::os::windows::fs::symlink_file(&target, to)
        }
    }
}
pub(crate) fn instance_dir(name: &str) -> PathBuf {
    crate::home()
        .join(".claude-split")
        .join(name)
        .join(".claude")
}
/// 含**明文凭据**的 MCP 配置文件，供权限检查使用。
///
/// `.claude.json`（默认 Claude 一份、每个环境一份）：`mcpServers` 的 `env` / `headers`
/// 是明文密钥。**默认 Claude 那份已不由应用写入**（决策 7.2 起「用户级」落在
/// `~/.cc-manager/shared/mcp.json`），但它仍含用户自己的明文密钥，
/// 权限过松同样意味着同机其他用户能读到 —— 所以继续列入检查。
///
/// 同步快照 `sync-snapshot.json` 已随双向合并一起退役，不再需要检查。
pub(crate) fn credential_file_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        crate::home().join(".claude.json"),
        crate::shared_config::shared_path(crate::shared_config::FIELD_MCP),
        crate::shared_config::ledger_path(),
        crate::cfg_dir().join("sync-snapshot.json"),
    ];
    for name in crate::profile_names(&crate::load()) {
        paths.push(instance_dir(&name).join(".claude.json"));
    }
    paths
}

fn lock_path() -> PathBuf {
    crate::cfg_dir().join("sync.lock")
}
#[cfg(not(test))]
fn log_path() -> PathBuf {
    crate::cfg_dir().join("sync.log")
}

#[cfg(not(test))]
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

/// 单元测试中的迁移夹具会故意制造失败并调用日志入口。测试进程不能把这些
/// `ccm-sync-test-*` 记录写进当前用户的真实诊断日志。
#[cfg(test)]
pub fn log_line(_msg: &str) {}

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
pub(crate) fn remove_link(p: &Path) -> std::io::Result<()> {
    if cfg!(windows) {
        fs::remove_dir(p)
    } else {
        fs::remove_file(p)
    }
}

// 旧的真实目录(隔离时代的安装)→ 内容搬进主目录,同名冲突留在原地;
// 搬空则删目录,搬不空则整体改名备份,腾出路径建链。返回备份路径(如有)。
#[cfg(test)]
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

/// 最内层：共享根与环境目录父目录都显式给定。
/// 有了它，迁移的**完整流程**（建链 → 切链 → 失败回滚）可以在临时目录上真跑一遍，
/// 不必拿用户的真实 `~/.claude` 做试验。
#[cfg(test)]
pub(crate) fn ensure_links_in(
    master: &Path,
    instance_root: &Path,
    names: &[String],
) -> Result<Vec<String>, String> {
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
        let inst = instance_root.join(name).join(".claude");
        if let Err(e) = fs::create_dir_all(&inst) {
            msgs.push(format!("{name}: 创建环境目录失败:{e}"));
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
                    // 迁移后共享根是应用共享库，不再是默认 Claude 的 ~/.claude ——
                    // 文案必须跟着说对，否则用户会以为自己的资源被搬进了默认 Claude。
                    Ok(Some(bak)) => msgs.push(format!(
                        "{name}/{sub}: 旧内容已并入共享资源库,同名冲突项备份在 {}",
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
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &inner,
                ])
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

pub(crate) fn write_json_atomic(path: &Path, v: &Value) -> std::io::Result<()> {
    let text = serde_json::to_vec_pretty(v).map_err(std::io::Error::other)?;
    write_bytes_atomic(path, &text)
}

pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "ccm-{}-{}-{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    // 这些文件可能含**明文凭据**（副本 `.claude.json` 的 MCP env/headers、以及
    // 同步快照里同一份配置）。不显式指定权限的话，umask 022 下会创建成 0644 ——
    // 而且 `rename` 是**覆盖**目标，会把原本 0600 的文件**降级**成 0644。
    // 所以必须在创建时就定权限，不能指望继承。
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    let result = (|| {
        std::io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        // 复核一次：`rename` 是否保留临时文件权限在各平台/文件系统上的保证不一致，
        // 显式收紧一遍最稳（顺带修正历史上已是 0644 的存量文件）。
        restrict_credential_permissions(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Unix 下把文件收紧到 0600（仅本人可读写）。
///
/// Windows 分支不做事：威胁边界只到"同机其他普通用户"，用户目录的继承 ACL 已覆盖；
/// 管理员/SYSTEM 不属于文件权限能可靠防御的范围 —— 见
/// `docs/凭证分域清单-2026-09-12.md` 的威胁边界定义。
fn restrict_credential_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
// ---------------- 锁 ----------------
//
// 协议（P0 审查 F2 修定）。原实现有两个真缺陷：
//   1) 按 **mtime > 60s** 回收 —— 一个合法的长操作会被抢锁，于是两个写者并发；
//   2) `Drop` **无条件删文件** —— 旧持有者退出时会删掉**新持有者**的锁。
//
// 现在的协议：
//   - 锁文件内容 = `{ token, pid, at }`（at 是最后一次心跳时间）
//   - **释放按 token 校验**：不是自己的锁绝不删
//   - **持有期间心跳续期**：长操作不会被误判成"崩溃遗留"
//   - 回收只在**心跳停了 LOCK_STALE_SECS 以上**时发生，且先原子取走再判断

const LOCK_HEARTBEAT_SECS: u64 = 30;
/// 心跳停多久才认为是遗留锁。必须**显著大于**心跳间隔，否则正常操作会被抢。
const LOCK_STALE_SECS: u64 = 180;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
struct LockInfo {
    token: String,
    pid: u32,
    /// 最后一次心跳的 epoch 秒
    at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 进程内唯一 token（pid + 纳秒 + 计数器）。锁与临时文件名共用。
pub(crate) fn unique_token() -> String {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        nanos,
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn read_lock(path: &Path) -> Option<LockInfo> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write_lock(path: &Path, info: &LockInfo) -> std::io::Result<()> {
    fs::write(
        path,
        serde_json::to_vec(info).map_err(std::io::Error::other)?,
    )
}

pub(crate) struct ConfigLockGuard {
    path: PathBuf,
    token: String,
    stop: Arc<AtomicBool>,
    heart: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ConfigLockGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.heart.take() {
            let _ = handle.join();
        }
        // **只在锁仍属于自己时才删**。否则"旧持有者退出"会把新持有者的锁删掉，
        // 第三个进程随即也能进来 —— 两个写者并发。
        if read_lock(&self.path).is_some_and(|info| info.token == self.token) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// 心跳线程：定期把 at 刷新成当前时间。用可中断的小步睡眠，
/// 否则 Drop 里的 join 要等满一个心跳周期。
fn spawn_heartbeat(
    path: PathBuf,
    token: String,
    stop: Arc<AtomicBool>,
) -> Option<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("ccm-config-lock".into())
        .spawn(move || {
            let steps = LOCK_HEARTBEAT_SECS * 10;
            loop {
                for _ in 0..steps {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                // 只给自己续期。锁已易主就停 —— 再写会把别人的锁覆盖掉。
                if !read_lock(&path).is_some_and(|info| info.token == token) {
                    return;
                }
                let _ = write_lock(
                    &path,
                    &LockInfo {
                        token: token.clone(),
                        pid: std::process::id(),
                        at: now_secs(),
                    },
                );
            }
        })
        .ok()
}

/// 只在"心跳停了太久"时回收。先把锁文件**原子取走**再判断，
/// 避免和并发的回收者互相踩：取走失败说明别人已经先动手了。
fn try_reclaim(path: &Path) -> bool {
    let taken = path.with_extension(format!("reclaim-{}", unique_token()));
    if fs::rename(path, &taken).is_err() {
        return false;
    }
    let stale = match read_lock(&taken) {
        Some(info) => now_secs().saturating_sub(info.at) > LOCK_STALE_SECS,
        // 读不出内容：可能是"刚 create_new 还没来得及写内容"的瞬间。
        // 用 mtime 兜底，且同样要求它确实很旧 —— 刚创建的锁 mtime 是现在，不会被误回收。
        None => fs::metadata(&taken)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| d.as_secs() > LOCK_STALE_SECS)
            // 连 mtime 都拿不到就**不回收** —— 误抢锁比留一个遗留锁危险得多
            .unwrap_or(false),
    };
    if stale {
        let _ = fs::remove_file(&taken);
        true
    } else {
        let _ = fs::rename(&taken, path);
        false
    }
}

pub(crate) fn acquire_config_lock() -> Option<ConfigLockGuard> {
    let p = lock_path();
    let _ = fs::create_dir_all(crate::cfg_dir());
    acquire_config_lock_at(&p)
}

fn acquire_config_lock_at(p: &Path) -> Option<ConfigLockGuard> {
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(p) {
            Ok(_) => {
                let token = unique_token();
                let info = LockInfo {
                    token: token.clone(),
                    pid: std::process::id(),
                    at: now_secs(),
                };
                if write_lock(p, &info).is_err() {
                    let _ = fs::remove_file(p);
                    return None;
                }
                let stop = Arc::new(AtomicBool::new(false));
                let heart = spawn_heartbeat(p.to_path_buf(), token.clone(), stop.clone());
                return Some(ConfigLockGuard {
                    path: p.to_path_buf(),
                    token,
                    stop,
                    heart,
                });
            }
            Err(_) => {
                if try_reclaim(p) {
                    continue;
                }
                return None;
            }
        }
    }
    None
}

// ---------------- 入口 ----------------

// 调用方须已持有 acquire_config_lock()。GUI 启动走 sync_configs（自带锁），
// mcp 的 User/Local 写入在 apply_mcp_change 内持同一把锁后调用本函数，避免与 CLI --sync 竞态。
pub(crate) struct SyncOutcome {
    pub summary: String,
    pub warnings: Vec<String>,
}

/// 共享配置的落地：**单向分发**（决策 7.2），不再是双向合并。
///
/// 原先这里把 `~/.claude.json` / `~/.claude/settings.json` 与各环境当**可写副本**
/// 做三方合并（含快照）。那有两个问题：
/// 一是应用会改写用户直接敲 `claude` 时用的那份配置；二是只要环境之间还在互相
/// 合并，**单个环境的覆盖会被传播到其他环境** —— 与决策 7.2 要求的「各环境可
/// 独立覆盖」直接冲突。
/// 现在改成：共享源（`~/.cc-manager/shared/*.json`）→ 各环境，**单向**；
/// 判定与台账在 `crate::shared_config`。
pub(crate) fn sync_configs_locked(names: &[String]) -> Result<SyncOutcome, String> {
    let mut summary: Vec<String> = vec![];
    let mut warnings: Vec<String> = vec![];
    warnings.extend(crate::shared_config::ensure_shared_sources());

    let field = crate::shared_config::FIELD_MCP;
    let report = crate::shared_config::distribute(field, names)?;
    let written: usize = report.envs.iter().map(|e| e.written).sum();
    let removed: usize = report.envs.iter().map(|e| e.removed).sum();
    summary.push(format!(
        "{field}：{} 个环境，下发 {written} 条 / 清理 {removed} 条",
        report.envs.len()
    ));
    warnings.extend(report.warnings.iter().cloned());
    // 环境覆盖必须报出来 —— 静默保留会让用户以为共享值已经生效
    for env in &report.envs {
        for info in &env.overrides {
            warnings.push(format!(
                "{field}：环境「{}」的「{}」{}，已保留环境自身的值",
                env.env,
                info.name,
                info.reason.label()
            ));
        }
    }
    Ok(SyncOutcome {
        summary: summary.join(";"),
        warnings,
    })
}

/// 返回**完整的** `SyncOutcome`（含 warnings），而不是只给 summary。
///
/// 原先把 warnings 丢在这里，于是"同步并修复"界面只说"写了 N 份"，
/// 用户看不到"哪个域被跳过 / 哪次写失败" —— 恰恰是最该知道的部分。
pub fn sync_configs(names: &[String]) -> Result<SyncOutcome, String> {
    let _guard = match acquire_config_lock() {
        Some(g) => g,
        None => {
            return Ok(SyncOutcome {
                summary: "另一个同步正在进行，本轮跳过".into(),
                warnings: vec!["未执行：同一时刻只允许一个同步在跑，请稍后重试。".into()],
            })
        }
    };
    sync_configs_locked(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // ---- 共享资源迁移（决策 7.3）----

    #[test]
    fn migration_marker_failure_restores_original_directory_and_link_targets() {
        let root = TmpDir::new("rollback-marker");
        let shared = root.0.join("shared");
        let instances = root.0.join("split");
        let original = root.0.join("custom-target");
        let env = instances.join("corp/.claude");
        fs::create_dir_all(&original).unwrap();
        fs::create_dir_all(env.join("agents")).unwrap();
        fs::write(env.join("agents/mine.md"), b"original").unwrap();
        create_directory_link(&original, &env.join("skills")).unwrap();
        let marker = root.0.join("marker-is-directory");
        fs::create_dir(&marker).unwrap();
        let result = switch_links_transaction(&shared, &instances, &["corp".into()], &marker);
        assert!(result.unwrap_err().contains("已恢复"));
        assert!(link_points_to(&env.join("skills"), &original));
        assert_eq!(fs::read(env.join("agents/mine.md")).unwrap(), b"original");
        assert!(fs::read_link(env.join("agents")).is_err());
        assert!(!env.join("plugins").exists());
        assert!(!marker.with_extension("pending.json").exists());
    }

    #[test]
    fn migration_never_copies_through_a_destination_junction() {
        let root = TmpDir::new("destination-link");
        let src = root.0.join("source");
        let target = root.0.join("external");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(src.join("file.txt"), b"source").unwrap();
        let dest = root.0.join("destination");
        create_directory_link(&target, &dest).unwrap();
        let mut conflicts = vec![];
        copy_tree_merge(&src, &dest, &mut conflicts, &mut vec![]).unwrap();
        assert!(!conflicts.is_empty());
        assert!(!target.join("file.txt").exists());
    }

    #[test]
    fn copy_tree_merge_dedupes_identical_and_never_overwrites_conflicts() {
        // 迁移只会**复制**到共享根；同名不同内容**不覆盖**，
        // 因为目标里那份可能是用户自己放的东西。
        let src = TmpDir::new("copy-src");
        let dst = TmpDir::new("copy-dst");
        fs::create_dir_all(src.0.join("sub")).unwrap();
        fs::write(src.0.join("same.txt"), b"identical").unwrap();
        fs::write(src.0.join("only-in-src.txt"), b"new").unwrap();
        fs::write(src.0.join("sub/nested.txt"), b"nested").unwrap();
        fs::create_dir_all(dst.0.join("sub")).unwrap();
        fs::write(dst.0.join("same.txt"), b"identical").unwrap();
        fs::write(dst.0.join("conflict.txt"), b"mine").unwrap();
        // 源里也有 conflict.txt，但内容不同
        fs::write(src.0.join("conflict.txt"), b"theirs").unwrap();

        let mut conflicts: Vec<String> = vec![];
        let mut skipped: Vec<String> = vec![];
        let (copied, deduped) =
            copy_tree_merge(&src.0, &dst.0, &mut conflicts, &mut skipped).unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");

        assert_eq!(copied, 2, "只有两个新文件该被复制");
        assert_eq!(deduped, 1, "内容相同的那个应被去重");
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        // 关键断言：冲突文件保持目标侧原样，**没有被源覆盖**
        assert_eq!(
            fs::read_to_string(dst.0.join("conflict.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_to_string(dst.0.join("sub/nested.txt")).unwrap(),
            "nested"
        );
        // 源目录**一个都没少**（只复制、不移动）
        assert!(src.0.join("same.txt").is_file());
        assert!(src.0.join("conflict.txt").is_file());
        assert!(src.0.join("only-in-src.txt").is_file());
    }

    #[test]
    fn copy_tree_merge_is_idempotent() {
        let src = TmpDir::new("copy-idem-src");
        let dst = TmpDir::new("copy-idem-dst");
        fs::write(src.0.join("a.txt"), b"a").unwrap();
        let mut conflicts = vec![];
        let mut skipped = vec![];
        let (first, _) = copy_tree_merge(&src.0, &dst.0, &mut conflicts, &mut skipped).unwrap();
        let (second, deduped) =
            copy_tree_merge(&src.0, &dst.0, &mut conflicts, &mut skipped).unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 0, "第二遍不该再复制");
        assert_eq!(deduped, 1);
        assert!(conflicts.is_empty());
    }

    /// **完整迁移端到端**：在临时目录上真跑一遍，含**真实建链与切链**
    /// （Windows 走 PowerShell 建 Junction）。不拿用户的真实 `~/.claude` 做试验。
    #[test]
    fn migration_copies_shares_switches_links_and_leaves_the_legacy_root_untouched() {
        let root = TmpDir::new("migrate-e2e");
        let legacy = root.0.join("legacy-claude");
        let new_root = root.0.join("shared");
        let instance_root = root.0.join("split");
        let marker = root.0.join("migrated");

        // 用户原有的共享资源（含嵌套目录）
        for sub in SHARED_SUBDIRS {
            fs::create_dir_all(legacy.join(sub)).unwrap();
        }
        fs::create_dir_all(legacy.join("skills/alpha")).unwrap();
        fs::write(legacy.join("skills/alpha/SKILL.md"), b"hello").unwrap();
        fs::write(legacy.join("agents/a.md"), b"agent").unwrap();
        fs::write(legacy.join("commands/c.md"), b"cmd").unwrap();
        fs::write(legacy.join("plugins/p.md"), b"plugin").unwrap();
        // **链接必须被原样重建，而不是跟随**：真机上 ~/.claude/skills 里全是用户
        // 自己建的链接（指向别处）。跟随它们会去复制目标内容，甚至撞上无权访问的盘
        // 把整场迁移拖停 —— 这正是第一次真机跑时发生的事。
        let linked_target = root.0.join("external-skill");
        fs::create_dir_all(&linked_target).unwrap();
        fs::write(linked_target.join("S.md"), b"linked").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&linked_target, legacy.join("skills/linked-skill")).unwrap();
        #[cfg(not(unix))]
        {
            let inner = format!(
                "New-Item -ItemType Junction -Path {} -Target {} | Out-Null",
                crate::ps_q(&legacy.join("skills/linked-skill").display().to_string()),
                crate::ps_q(&linked_target.display().to_string())
            );
            assert!(crate::ps_command()
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &inner
                ])
                .output()
                .unwrap()
                .status
                .success());
        }

        // 迁移前的状态：两个环境的链接指向**旧根**
        let names = vec!["corp".to_string(), "test".to_string()];
        let before = ensure_links_in(&legacy, &instance_root, &names).unwrap();
        assert!(before.is_empty(), "先得把迁移前的状态建出来：{before:?}");
        let legacy_bytes: Vec<(PathBuf, Vec<u8>)> =
            ["skills/alpha/SKILL.md", "agents/a.md", "commands/c.md"]
                .iter()
                .map(|p| (legacy.join(p), fs::read(legacy.join(p)).unwrap()))
                .collect();

        let note = migrate_shared_resources_at(&legacy, &new_root, &marker, &instance_root, &names);
        assert!(note.is_some(), "应当给出迁移提示");

        // ① 复制过去了
        assert_eq!(
            fs::read(new_root.join("skills/alpha/SKILL.md")).unwrap(),
            b"hello"
        );
        // 链接是**重建的链接**，不是拷贝出来的目录：指向必须与源一致
        assert!(
            link_points_to(&new_root.join("skills/linked-skill"), &linked_target),
            "链接没有被原样重建：{:?}",
            fs::read_link(new_root.join("skills/linked-skill"))
        );
        assert!(
            !new_root.join("skills/linked-skill/S.md").exists()
                || fs::read_link(new_root.join("skills/linked-skill")).is_ok(),
            "跟随了链接、把目标内容拷了一份"
        );
        // ② **旧根一个字节都没变**（应用对默认 Claude 只读）
        for (path, bytes) in &legacy_bytes {
            assert_eq!(
                &fs::read(path).unwrap(),
                bytes,
                "旧文件被动了：{}",
                path.display()
            );
        }
        // ③ 两个环境的链接都切到了新根
        for name in &names {
            let inst = instance_root.join(name).join(".claude");
            for sub in SHARED_SUBDIRS {
                let dst = inst.join(sub);
                assert!(
                    link_points_to(&dst, &new_root.join(sub)),
                    "{name}/{sub} 没有指向新共享根：{:?}",
                    fs::read_link(&dst)
                );
            }
        }
        // ④ 标记写下了（它同时是切换开关）
        assert!(marker.is_file());

        // ⑤ 幂等：再跑一次什么都不做
        assert!(
            migrate_shared_resources_at(&legacy, &new_root, &marker, &instance_root, &names)
                .is_none()
        );
    }

    /// 真机那条失败的**最小复现**：`~/.claude/skills` 里有链接指向**已不存在的目标**
    /// （真机上是另一块盘上的路径，读取时 `拒绝访问 (os error 5)`）。
    ///
    /// 旧实现用 `entry.metadata()`（跟随链接）→ 直接报错 → **整场迁移中止、连复制都没做完**。
    ///
    /// 修好后的行为分两段，测试对两段都断言：
    /// 1. **复制阶段不再被链接拖停** —— 正常文件照样复制过去；
    /// 2. 但**切换链接要停下**（`不切换、保持原状、下次重试`）：那个链接在 Windows 上
    ///    重建不了（Junction 要求目标存在、file symlink 要提权），若硬切过去，
    ///    环境就会失去这个条目 —— 外接盘重新挂上后也回不来。宁可保持旧状态。
    #[test]
    fn migration_survives_a_dangling_link() {
        let root = TmpDir::new("migrate-dangling");
        let legacy = root.0.join("legacy-claude");
        let new_root = root.0.join("shared");
        let marker = root.0.join("migrated");
        let instance_root = root.0.join("split");
        for sub in SHARED_SUBDIRS {
            fs::create_dir_all(legacy.join(sub)).unwrap();
        }
        fs::write(legacy.join("skills/real.md"), b"real").unwrap();

        // 先建链接，再把目标删掉 → 悬空链接
        let target = root.0.join("gone-target");
        fs::create_dir_all(&target).unwrap();
        let link = legacy.join("skills/dangling");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        {
            let inner = format!(
                "New-Item -ItemType Junction -Path {} -Target {} | Out-Null",
                crate::ps_q(&link.display().to_string()),
                crate::ps_q(&target.display().to_string())
            );
            assert!(crate::ps_command()
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &inner
                ])
                .output()
                .unwrap()
                .status
                .success());
        }
        fs::remove_dir_all(&target).unwrap();

        let note = migrate_shared_resources_at(&legacy, &new_root, &marker, &instance_root, &[])
            .expect("应当给出明确说明，而不是静默无事发生");

        // ① 复制阶段没被链接拖停：真实文件照样复制过去了
        assert_eq!(fs::read(new_root.join("skills/real.md")).unwrap(), b"real");
        // ② 但**没有切换**：说明里点名了保持原状与那个条目
        assert!(note.contains("已保持原状"), "{note}");
        assert!(note.contains("dangling"), "要点名是哪个条目卡住了：{note}");
        // ③ 不写标记 ⇒ 下次启动会重试（验收要求里的「失败重试」）
        assert!(
            !marker.is_file(),
            "没切换成功就不该写标记，否则再也不会重试"
        );
        // ④ 旧根一个字节没动
        assert_eq!(fs::read(legacy.join("skills/real.md")).unwrap(), b"real");
    }

    #[test]
    fn migration_is_skipped_when_there_is_nothing_to_migrate() {
        let root = TmpDir::new("migrate-empty");
        let legacy = root.0.join("no-such-claude");
        let new_root = root.0.join("shared");
        let marker = root.0.join("migrated");
        let instance_root = root.0.join("split");

        let note = migrate_shared_resources_at(&legacy, &new_root, &marker, &instance_root, &[]);

        assert!(note.is_none(), "没有东西可迁时不该打扰用户");
        assert!(marker.is_file(), "但也要标记，免得每次启动都重跑");
    }

    #[test]
    fn migration_reports_conflicts_without_overwriting() {
        let root = TmpDir::new("migrate-conflict");
        let legacy = root.0.join("legacy-claude");
        let new_root = root.0.join("shared");
        let marker = root.0.join("migrated");
        let instance_root = root.0.join("split");
        fs::create_dir_all(legacy.join("skills")).unwrap();
        fs::write(legacy.join("skills/x.md"), b"theirs").unwrap();
        // 共享库里已经有一份同名但内容不同的（可能是用户自己放的）
        fs::create_dir_all(new_root.join("skills")).unwrap();
        fs::write(new_root.join("skills/x.md"), b"mine").unwrap();

        let note =
            migrate_shared_resources_at(&legacy, &new_root, &marker, &instance_root, &[]).unwrap();

        assert_eq!(
            fs::read_to_string(new_root.join("skills/x.md")).unwrap(),
            "mine",
            "冲突文件被覆盖了"
        );
        assert!(note.contains("同名但内容不同"), "{note}");
        assert!(note.contains("未覆盖"), "{note}");
    }

    #[test]
    fn shared_root_and_master_switch_are_consistent() {
        // 迁移前：共享根仍是默认 Claude 的 ~/.claude（行为与改造前一致）
        // 迁移后：指向 ~/.cc-manager/shared
        // 这里只钉住两者的关系，避免有人把开关和路径写岔
        assert_eq!(shared_root(), crate::cfg_dir().join("shared"));
        if shared_migration_done() {
            assert_ne!(master_dir(), crate::home().join(".claude"));
        } else {
            assert_eq!(master_dir(), crate::home().join(".claude"));
        }
    }

    #[test]
    fn shell_sync_never_writes_plugin_state_directly() {
        let source = include_str!("sync.rs");
        let start = source.find("pub(crate) fn sync_configs_locked").unwrap();
        let end = source[start..]
            .find("pub fn sync_configs")
            .map(|offset| start + offset)
            .unwrap();
        let body = &source[start..end];
        assert!(body.contains("FIELD_MCP"));
        assert!(!body.contains("FIELD_PLUGINS"));
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    #[test]
    fn config_lock_excludes_other_writers_and_releases_on_drop() {
        let dir = TmpDir::new("lock");
        let path = dir.0.join("sync.lock");
        let first = acquire_config_lock_at(&path).unwrap();
        assert!(acquire_config_lock_at(&path).is_none());
        drop(first);
        assert!(acquire_config_lock_at(&path).is_some());
    }

    #[test]
    fn lock_release_does_not_remove_a_lock_taken_over_by_someone_else() {
        // 回归（审查 F2）：原先 Drop **无条件**删锁文件。
        // 于是"旧持有者退出"会把**新持有者**的锁删掉，第三个进程随即也能进来 ——
        // 两个写者并发，丢更新。
        let dir = TmpDir::new("lock-takeover");
        let path = dir.0.join("sync.lock");
        let guard = acquire_config_lock_at(&path).unwrap();

        // 模拟"旧锁被回收后别人重新拿到"：锁文件换成另一个 token
        write_lock(
            &path,
            &LockInfo {
                token: "someone-else".into(),
                pid: 999_999,
                at: now_secs(),
            },
        )
        .unwrap();

        drop(guard);
        assert!(path.exists(), "旧持有者退出时删掉了新持有者的锁");
        assert_eq!(read_lock(&path).unwrap().token, "someone-else");
    }

    #[test]
    fn stale_lock_is_reclaimed_but_fresh_lock_is_not() {
        // 回归（审查 F2）：原先按 **mtime > 60s** 回收 —— 合法的长操作会被抢锁。
        // 现在只认"心跳停了太久"，而持有期间是有心跳的。
        let dir = TmpDir::new("lock-stale");
        let path = dir.0.join("sync.lock");

        // 心跳还新鲜：不能被抢
        write_lock(
            &path,
            &LockInfo {
                token: "fresh".into(),
                pid: 1,
                at: now_secs(),
            },
        )
        .unwrap();
        assert!(
            acquire_config_lock_at(&path).is_none(),
            "心跳新鲜的锁被抢走了 —— 长操作会因此丢锁"
        );

        // 心跳停了远超阈值：是遗留锁，可以回收
        write_lock(
            &path,
            &LockInfo {
                token: "stale".into(),
                pid: 1,
                at: now_secs().saturating_sub(LOCK_STALE_SECS + 10),
            },
        )
        .unwrap();
        let guard = acquire_config_lock_at(&path);
        assert!(guard.is_some(), "遗留锁没有被回收");
        drop(guard);
    }

    #[test]
    fn concurrent_atomic_writes_use_distinct_temporary_files() {
        let dir = TmpDir::new("atomic");
        let path = dir.file("settings.json", "{}");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let writers = (0..8)
            .map(|id| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    write_json_atomic(&path, &serde_json::json!({"writer":id})).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().unwrap();
        }
        let result: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(result["writer"].as_u64().unwrap() < 8);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_never_downgrades_existing_credential_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TmpDir::new("atomic-perm");
        let path = dir.file("config.json", "{}");
        // 关键场景：目标原本是严格权限（如 keychain/DPAPI 之外的 0600），
        // 一次原子写不该把它变成 umask 默认的 0644 —— MCP env/headers 是明文密钥。
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        write_json_atomic(&path, &serde_json::json!({"write": 1})).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "写入后权限被放宽成 {mode:o}");

        // 反过来：原本宽松的文件（历史遗留 0644）也要被收紧
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_json_atomic(&path, &serde_json::json!({"write": 2})).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "存量 0644 文件未被收紧，仍是 {mode:o}");
    }

    // 每个测试用独立临时目录，避免并行跑测试时互相踩文件；Drop 时自动清理。
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir()
                .join(format!("ccm-sync-test-{tag}-{}-{n}", std::process::id()));
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
}
