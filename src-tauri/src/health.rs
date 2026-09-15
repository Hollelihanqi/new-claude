// 健康检查与诊断:
// 1) model_pin_warnings / fix_model_pin —— 检测环境(和默认 Claude)的 settings.json
//    里 /model 钉死了具体型号(绕过 ANTHROPIC_DEFAULT_*_MODEL 档位映射)。
//    受管理环境支持一键还原；**默认 Claude 只读告警、不提供还原**（决策 7.4）。
// 2) health_check —— 按需跑一组自检(claude CLI、终端集成、共享链接、证书、网关连通),
//    网关探测并行执行,只在用户点开健康面板时才发起。
// 3) export_diagnostics —— 把自检结果 + 脱敏配置 + 同步日志尾部汇总成一个文本报告,
//    落到桌面,同事出问题时直接把文件发给管理员。

use crate::{load, profile_names, Profile, TrustMode, MAIN_PROFILE_KEY, MARK};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------- 模型档位钉死检测 ----------------

// /model 只有选这些档位别名时,模型映射(ANTHROPIC_DEFAULT_*_MODEL)才拦得住;
// 其余值都是具体型号 ID,会原样发给网关、绕过映射。
const MODEL_ALIASES: [&str; 5] = ["default", "opus", "sonnet", "haiku", "opusplan"];

fn is_model_alias(m: &str) -> bool {
    let m = m.trim().to_ascii_lowercase();
    // sonnet[1m] 这类长上下文档位也算别名档位
    let base = m.strip_suffix("[1m]").unwrap_or(&m);
    MODEL_ALIASES.contains(&base)
}

// profile 键 → 该副本 settings.json 的路径(__main__ = 默认 Claude)
fn settings_path_for(profile: &str) -> PathBuf {
    if profile == MAIN_PROFILE_KEY {
        crate::home().join(".claude").join("settings.json")
    } else {
        crate::home()
            .join(".claude-split")
            .join(profile)
            .join(".claude")
            .join("settings.json")
    }
}

// settings.json 里钉死的具体型号;别名档位/无 model 字段/文件缺失均返回 None
fn pinned_model_in(path: &Path) -> Option<String> {
    let doc: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let m = doc.get("model")?.as_str()?.trim().to_string();
    if m.is_empty() || is_model_alias(&m) {
        None
    } else {
        Some(m)
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelPinWarning {
    pub profile: String, // 环境名;默认 Claude为 __main__
    pub model: String,   // 被钉死的具体型号
    pub settings_path: String,
}

fn collect_model_pin_warnings(list: &[Profile]) -> Vec<ModelPinWarning> {
    let mut out = vec![];
    let has_router = list.iter().any(|p| p.type_ == "router");
    // 默认 Claude钉死具体型号本身合法,但在 home 目录启动环境时会以"项目级配置"
    // 身份覆盖环境档位,所以只在存在网关环境时才提醒。
    if has_router {
        let p = settings_path_for(MAIN_PROFILE_KEY);
        if let Some(m) = pinned_model_in(&p) {
            out.push(ModelPinWarning {
                profile: MAIN_PROFILE_KEY.into(),
                model: m,
                settings_path: p.display().to_string(),
            });
        }
    }
    // account 类型环境直连官方,钉具体型号是合法用法,只查 router
    for prof in list.iter().filter(|p| p.type_ == "router") {
        let p = settings_path_for(&prof.name);
        if let Some(m) = pinned_model_in(&p) {
            out.push(ModelPinWarning {
                profile: prof.name.clone(),
                model: m,
                settings_path: p.display().to_string(),
            });
        }
    }
    out
}

#[tauri::command]
pub fn model_pin_warnings() -> Vec<ModelPinWarning> {
    collect_model_pin_warnings(&load())
}

// 一键还原:删掉 settings.json 的 model 字段(回到默认档位),其余内容原样保留。
//
// **只对受管理环境开放**（决策 7.4）。默认 Claude 的 `~/.claude` 是用户自己那份配置，
// 应用对它**只读**：可以识别并提醒，但**绝不替用户改写** —— 否则"应用会不会动我的 Claude"
// 就成了真实风险。这个后端拒绝是硬边界，不依赖前端不画按钮。
#[tauri::command]
pub fn fix_model_pin(profile: String) -> Result<String, String> {
    if profile == MAIN_PROFILE_KEY {
        return Err(
            "默认 Claude 的配置由你自己管理，应用不会修改它。如需调整，请在 Claude Code 中自行修改（`/model`）。"
                .into(),
        );
    }
    // 只允许操作已知副本,防止前端传来任意路径片段
    let known = load().iter().any(|p| p.name == profile);
    if !known {
        return Err("未找到该环境".into());
    }
    let path = settings_path_for(&profile);
    let text = fs::read_to_string(&path).map_err(|e| format!("读取 settings.json 失败：{e}"))?;
    let mut doc: Value =
        serde_json::from_str(&text).map_err(|e| format!("settings.json 不是有效 JSON：{e}"))?;
    let obj = doc.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    if obj.remove("model").is_none() {
        return Ok("该副本没有钉死模型，无需还原。".into());
    }
    crate::sync::write_json_atomic(&path, &doc).map_err(|e| format!("写回失败：{e}"))?;
    Ok("已还原为档位别名（默认档）。正在运行的 claude 会话需重启后生效。".into())
}

// ---------------- 健康检查 ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthItem {
    pub id: String,
    pub label: String,
    pub status: String, // "ok" | "warn" | "fail"
    pub detail: String,
}

fn item(id: &str, label: &str, status: &str, detail: String) -> HealthItem {
    HealthItem {
        id: id.into(),
        label: label.into(),
        status: status.into(),
        detail,
    }
}

/// 逐个受支持 shell 报告终端入口状态。
///
/// **必须分开报**：Windows 上 5.1 与 PowerShell 7 的 `$PROFILE` 是两个不同文件，
/// 只报一个总状态会让"只把 5.1 接进去了、但用户实际用 pwsh 7"显示成已生效 ——
/// 这正是 B#11 的界面侧表现。
fn integration_status_items() -> Vec<HealthItem> {
    let targets = crate::available_shell_targets();
    if targets.is_empty() {
        return vec![item(
            "integration",
            "终端集成",
            "fail",
            "没有检测到受支持的终端。当前支持 macOS 的 zsh / bash 与 Windows 的 PowerShell 5.1 / 7；\
             cmd.exe、fish、sh、Git Bash、WSL 未支持，它们不会由本应用接管。"
                .into(),
        )];
    }

    let mut out = vec![];
    for target in targets {
        let label = target.label();
        let id = format!("shell:{label}");
        let title = format!("终端入口（{label}）");
        let paths = match crate::shell_config_paths(target) {
            Ok(paths) => paths,
            Err(e) => {
                out.push(item(
                    &id,
                    &title,
                    "warn",
                    format!("{e}。该终端不会被接入。"),
                ));
                continue;
            }
        };
        let listing = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("、");
        let attached: Vec<&PathBuf> = paths
            .iter()
            .filter(|p| {
                fs::read_to_string(p)
                    .map(|c| c.contains(MARK))
                    .unwrap_or(false)
            })
            .collect();

        if attached.len() == paths.len() {
            out.push(item(&id, &title, "ok", format!("已接入（{listing}）。")));
        } else if attached.is_empty() {
            out.push(item(
                &id,
                &title,
                "fail",
                format!(
                    "终端入口未生效：{listing} 里没有集成行。在该终端里敲 claude <环境名> 不会使用该环境启动。\
                     请在「环境配置」里点一次「保存并接入终端」。"
                ),
            ));
        } else {
            let done = attached
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("、");
            out.push(item(
                &id,
                &title,
                "warn",
                format!("部分生效：已接入 {done}；其余文件未生效（{listing}）。"),
            ));
        }
    }
    out
}

/// 网关探测通过时的说明文案。只有严格校验真的通过才会走到这里——
/// 任何"跳过证书校验"的路径都不允许产出它，否则就是把用户骗进证书错误的坑。
///
/// `plaintext` 对应 http:// 网关：那里根本没有 TLS 这一层，API Key 明文过网。
/// 必须说出来，否则"严格探测通过"会被读成"传输是加密的"。
fn gateway_ok_detail(mode: TrustMode, model_count: usize, plaintext: bool) -> String {
    let base = match mode {
        TrustMode::System => format!("可达，Key 有效，{model_count} 个可用模型。"),
        TrustMode::ImportedCa => format!(
            "可达，Key 有效，{model_count} 个可用模型。网关证书由已导入的 CA 签发，claude 需在已接入集成的终端中运行。"
        ),
    };
    if plaintext {
        // http 现在只可能是 loopback —— 远程 http 在保存时就被拒了（见 PLAINTEXT_TRANSPORT_REJECTED），
        // 所以这里不该再喊"明文经过网络"：回环流量不出本机。
        format!("{base}网关地址是 http://（本机回环），流量不出本机。")
    } else {
        base
    }
}

// ---------------- 凭证文件权限 ----------------
//
// 威胁边界（2026-09-12 用户裁定）：**只防御同机其他普通用户读取**。
// 管理员 / SYSTEM / root / sudo 能绕过文件权限，不在本检查能保证的范围内，
// 所以它们出现在白名单里是预期的，不算违规。
//
// 用**白名单**而不是黑名单：本机实测出现过第三方工具创建的自定义组
// (SID 以 -1006 结尾) 被继承授予了读权限——用"列举 Everyone/Users"的黑名单
// 会正好漏掉这一类。判据是"除了所有者 SYSTEM 和 Administrators，还有谁能读"。

/// 出现这些主体不算违规：SYSTEM、Administrators（SDDL 简写与完整 SID 两种写法）
const ACL_ALLOWED_PRINCIPALS: [&str; 4] = ["SY", "BA", "S-1-5-18", "S-1-5-32-544"];

/// 从 SDDL 里取所有者 SID（`O:` 到 `G:` / `D:` 之间）
fn sddl_owner(sddl: &str) -> Option<String> {
    let rest = &sddl[sddl.find("O:")? + 2..];
    let end = ["G:", "D:"]
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// 这段权限串是否包含"读取文件内容"的能力。
///
/// SDDL 的权限字段有三种写法，都要覆盖：
///   - 文件专有：`FR`(读) / `FA`(全部) / `FW`(写) / `FX`(执行)
///   - 通用映射：`GR`(Generic Read) / `GA`(Generic All) / `GW` / `GX`
///   - 十六进制掩码：`0x1200a9` 这类
///
/// **未知写法一律按"可读"处理（fail-closed）**：这是安全诊断，
/// 宁可误报也不能漏报 —— 漏报会让界面显示"仅限本人读取"而实际并非如此。
fn rights_allow_read(rights: &str) -> bool {
    match rights {
        // 含读取内容能力
        "FA" | "FR" | "RX" | "GR" | "GA" => true,
        // 明确不含读取内容能力（写 / 执行 / 改权限 / 改属主）
        "FW" | "FX" | "GW" | "GX" | "WD" | "WO" | "RC" | "SD" => false,
        _ if rights.starts_with("0x") => u32::from_str_radix(&rights[2..], 16)
            .map(|mask| mask & 0x1 != 0 || mask & 0x8 != 0) // FILE_READ_DATA / FILE_READ_EA
            .unwrap_or(true),
        // 认不出来按有风险处理
        _ => true,
    }
}

/// 找出把读权限给了"非授权主体"的 ACE，返回形如 `SID（权限）` 的描述。
/// 空结果 = 合规。
fn sddl_unauthorized_reads(sddl: &str) -> Vec<String> {
    let owner = sddl_owner(sddl);
    let mut out = vec![];
    for ace in sddl.split('(').skip(1) {
        let Some(ace) = ace.split(')').next() else {
            continue;
        };
        let fields: Vec<&str> = ace.split(';').collect();
        // AceType;AceFlags;Rights;ObjectGuid;InheritObjectGuid;Sid
        if fields.len() < 6 || fields[0] != "A" {
            continue; // 只关心"允许"型 ACE
        }
        let (rights, sid) = (fields[2], fields[5]);
        if !rights_allow_read(rights) {
            continue;
        }
        if ACL_ALLOWED_PRINCIPALS.contains(&sid) || owner.as_deref() == Some(sid) {
            continue;
        }
        out.push(format!("{sid}（{rights}）"));
    }
    out
}

/// 路径是否位于用户主目录之下（Windows 大小写不敏感，故统一小写后比前缀）
fn path_under_home(path: &Path, home: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
    let (target, base) = (norm(path), norm(home));
    target.starts_with(&format!("{base}/"))
}

#[cfg(target_os = "windows")]
fn file_sddl(path: &Path) -> Result<String, String> {
    // 用 ps_q 做 PowerShell 单引号转义，避免路径里的引号打破命令
    let script = format!(
        "(Get-Acl -LiteralPath {}).Sddl",
        crate::ps_q(&path.display().to_string())
    );
    let out = crate::ps_command()
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| format!("调用 PowerShell 读取权限失败：{e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 该文件是否被"非授权主体"读到。返回违规描述，空 = 合规。
#[cfg(target_os = "windows")]
fn unauthorized_readers(path: &Path) -> Result<Vec<String>, String> {
    Ok(sddl_unauthorized_reads(&file_sddl(path)?))
}

/// macOS / Linux：用文件模式判断。`0600` 之外（组或其他用户可读）都算违规。
#[cfg(unix)]
fn unauthorized_readers(path: &Path) -> Result<Vec<String>, String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path)
        .map_err(|e| format!("读取文件权限失败：{e}"))?
        .permissions()
        .mode();
    let mut out = vec![];
    if mode & 0o040 != 0 {
        out.push("同组用户可读".to_string());
    }
    if mode & 0o004 != 0 {
        out.push("其他用户可读".to_string());
    }
    Ok(out)
}

#[cfg(not(any(target_os = "windows", unix)))]
fn unauthorized_readers(_path: &Path) -> Result<Vec<String>, String> {
    Ok(vec![])
}

/// 需要核对权限的凭证文件。
/// 只列**真的含密钥材料或凭据密文**的文件，避免把无关文件报成问题。
fn credential_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = vec![];
    #[cfg(target_os = "windows")]
    {
        // Windows 的网关 Key 是 DPAPI 密文，散落在 config.json **以及它的备份/恢复产物**里
        // （config.backup / .previous / .next / .corrupt*）。按前缀枚举才能整族收进来 ——
        // 只查 config.json 会漏掉备份，而那些文件的权限可能更松。
        // （macOS 的 key 在钥匙串里，这些文件不含密钥材料，故不检查。）
        files.extend(crate::config_artifact_paths_in(&crate::cfg_dir()).unwrap_or_default());
        // 集成脚本里嵌了同一段 DPAPI 密文
        files.push(crate::ps_path());
    }
    // MCP 配置（`.claude.json`）与同步快照：两个平台上都是明文密钥，全平台都查
    files.extend(crate::sync::credential_file_paths());
    // WorkBuddy 的明文密钥文件（含 write_document 派生出的备份/临时产物）
    files.extend(crate::workbuddy::credential_file_paths());
    files.retain(|path| path.is_file());
    files
}

fn credential_permission_item() -> Option<HealthItem> {
    let files = credential_files();
    if files.is_empty() {
        return None;
    }
    let home = crate::home();
    let mut problems: Vec<String> = vec![];
    let mut outside: Vec<String> = vec![];
    for path in &files {
        let name = path.display().to_string();
        if !path_under_home(path, &home) {
            // 例如用户自己设了 WORKBUDDY_CONFIG_DIR 到别处：不算违规，但要说出来
            outside.push(name.clone());
        }
        match unauthorized_readers(path) {
            Ok(readers) if !readers.is_empty() => {
                problems.push(format!("{name}：{}", readers.join("、")));
            }
            Ok(_) => {}
            Err(e) => problems.push(format!("{name}：无法读取权限（{e}）")),
        }
    }

    if problems.is_empty() && outside.is_empty() {
        return Some(item(
            "credperms",
            "凭证文件权限",
            "ok",
            format!(
                "{} 个含密钥的文件均仅限本人读取（管理员/SYSTEM 等高权限主体不在防御范围内）。",
                files.len()
            ),
        ));
    }
    let mut detail = String::new();
    if !problems.is_empty() {
        detail += &format!("以下文件可能被其他普通用户读取：{}。", problems.join("；"));
    }
    if !outside.is_empty() {
        detail += &format!(
            "另有文件不在用户目录下（自定义配置目录？）：{}。",
            outside.join("、")
        );
    }
    Some(item(
        "credperms",
        "凭证文件权限",
        if problems.is_empty() { "warn" } else { "fail" },
        detail,
    ))
}

fn run_health_checks() -> Vec<HealthItem> {
    let list = load();
    let names = profile_names(&list);
    let mut items = vec![];

    // 1. claude CLI
    let detection = crate::claude_cli::detect_claude();
    match detection.path {
        Some(path) => items.push(item(
            "claude",
            "Claude Code CLI",
            "ok",
            format!(
                "已找到：{}（{}）",
                path.display(),
                detection.version.unwrap_or_else(|| "版本未知".to_string())
            ),
        )),
        None => items.push(item("claude", "Claude Code CLI", "fail", detection.detail)),
    }

    // 2. 终端集成
    if names.is_empty() {
        items.push(item(
            "integration",
            "终端集成",
            "warn",
            "还没有创建任何环境。".into(),
        ));
    } else {
        let script = if cfg!(target_os = "windows") {
            crate::ps_path()
        } else {
            crate::sh_path()
        };
        if !script.is_file() {
            items.push(item(
                "integration",
                "终端集成",
                "fail",
                format!(
                    "集成脚本缺失（{}），请点一次「保存并接入终端」重建。",
                    script.display()
                ),
            ));
        } else {
            items.extend(integration_status_items());
        }
    }

    // 3. 环境命令入口是否可用。
    // 与官方子命令同名的存量环境不会被写进集成脚本（见 can_have_terminal_entry），
    // 否则 `claude mcp add ...` 会被截走当成"切环境"、官方子命令根本不被调用。
    // 这是**常驻**项：只要冲突还在就一直显示，绝不渲染成"正常可用"。
    // 名称含不安全字符的环境同理 —— 它们同样拿不到 `claude <名字>` 这个入口。
    if !names.is_empty() {
        // 判定只认 can_have_terminal_entry，下面两行仅用于**解释原因**，
        // 避免生成器将来加了新判据而这里漏跟（判据漂移）。
        let no_entry: Vec<&String> = names
            .iter()
            .filter(|n| !crate::can_have_terminal_entry(n))
            .collect();
        if !no_entry.is_empty() {
            let shadowing: Vec<String> = no_entry
                .iter()
                .filter(|n| crate::collides_with_claude_subcommand(n))
                .map(|n| (*n).clone())
                .collect();
            let unsafe_names: Vec<String> = no_entry
                .iter()
                .filter(|n| !crate::script_safe_name(n))
                .map(|n| (*n).clone())
                .collect();
            let mut why: Vec<String> = vec![];
            if !shadowing.is_empty() {
                why.push(format!(
                    "{} 与 Claude 官方子命令同名（`claude {}` 已交还官方子命令）",
                    shadowing.join("、"),
                    shadowing.first().map(String::as_str).unwrap_or("")
                ));
            }
            if !unsafe_names.is_empty() {
                why.push(format!("{} 名称含不安全字符", unsafe_names.join("、")));
            }
            items.push(item(
                "env_entry",
                "环境命令入口",
                "warn",
                format!(
                    "以下环境的命令入口已禁用（敲 `claude <环境名>` 不会进入该环境）：{}。\
                     其配置、Key 与历史数据均已保留，仍可查看、编辑和删除；改名迁移功能待提供。{}",
                    why.join("；"),
                    crate::RELOAD_HINT
                ),
            ));
        }
    }

    // 4. 扩展资源结构（逐项分发 + 旧版整目录链接迁移）
    if !names.is_empty() {
        let probs = crate::extensions::problems(&names);
        if probs.is_empty() {
            items.push(item(
                "links",
                "扩展资源结构",
                "ok",
                format!(
                    "{} 个环境的 Skills、Agents 与 Plugins 目录结构正常。",
                    names.len()
                ),
            ));
        } else {
            items.push(item(
                "links",
                "扩展资源结构",
                "warn",
                format!(
                    "发现异常：{}。下次启动本程序时会在后台继续迁移或修复。",
                    probs.join("、")
                ),
            ));
        }
    }

    // 5. CA 证书
    let cert_count = crate::count_certs();
    let has_router = list.iter().any(|p| p.type_ == "router");
    if cert_count > 0 {
        items.push(item(
            "cert",
            "CA 证书",
            "ok",
            format!("信任库中共 {cert_count} 张证书。"),
        ));
    } else if has_router {
        items.push(item(
            "cert",
            "CA 证书",
            "warn",
            "未导入任何证书。若公司网关用自签名证书，请到「设置 → CA 证书」导入。".into(),
        ));
    } else {
        items.push(item(
            "cert",
            "CA 证书",
            "ok",
            "未导入（没有网关环境，无需证书）。".into(),
        ));
    }

    // 4b. 凭证文件权限：确认含密钥的文件没被其他普通用户读到
    //     （只防御普通用户；管理员/SYSTEM/root 能绕过文件权限，不在此列）
    if let Some(entry) = credential_permission_item() {
        items.push(entry);
    }

    // 6. 各网关环境的网关连通 + Key 有效性（并行探测）
    //    探测严格校验证书，只有证书真的通过才会报 ok；跳过校验时可达不再算作健康。
    let routers: Vec<Profile> = list
        .iter()
        .filter(|p| p.type_ == "router")
        .cloned()
        .collect();
    let handles: Vec<_> = routers
        .into_iter()
        .map(|p| {
            std::thread::spawn(move || {
                // http:// 没有 TLS：探测照样能过，但 Key 是明文的，文案要讲清楚
                let plaintext = p
                    .base_url
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("http://");
                let res: Result<(TrustMode, usize), String> = if p.base_url.trim().is_empty() {
                    Err("未配置网关地址".to_string())
                } else {
                    crate::decrypt_token(&p).and_then(|t| {
                        crate::detect_models_ladder(&p.name, &p.base_url, &t)
                            .map(|(mode, models)| (mode, models.len()))
                            .map_err(|e| e.message())
                    })
                };
                (p.name, res, plaintext)
            })
        })
        .collect();
    for h in handles {
        if let Ok((name, res, plaintext)) = h.join() {
            match res {
                Ok((mode, count)) => items.push(item(
                    &format!("gateway:{name}"),
                    &format!("网关连通（{name}）"),
                    "ok",
                    gateway_ok_detail(mode, count, plaintext),
                )),
                Err(msg) => items.push(item(
                    &format!("gateway:{name}"),
                    &format!("网关连通（{name}）"),
                    "fail",
                    msg,
                )),
            }
        }
    }

    // 7. 模型档位钉死告警
    for w in collect_model_pin_warnings(&load()) {
        let who = if w.profile == MAIN_PROFILE_KEY {
            "默认 Claude".to_string()
        } else {
            format!("环境 {}", w.profile)
        };
        items.push(item(
            &format!("modelpin:{}", w.profile),
            &format!("模型映射（{who}）"),
            "warn",
            format!(
                "{who} 当前固定使用模型「{}」，可能覆盖模型档位映射。{}",
                w.model,
                if w.profile == MAIN_PROFILE_KEY {
                    "应用只读此配置；如需调整，请在 Claude Code 中自行选择模型。"
                } else {
                    "可在环境管理中恢复档位选择。"
                }
            ),
        ));
    }

    items
}

#[tauri::command]
pub async fn health_check() -> Result<Vec<HealthItem>, String> {
    let items = tauri::async_runtime::spawn_blocking(run_health_checks)
        .await
        .map_err(|e| format!("健康检查任务异常：{e}"))?;
    // 记一次"最近验证"：环境证明卡要能说出"上次完整检测是什么时候"，
    // 否则用户看到的永远是"刚刚"，分不清手里的结论是不是陈的。
    // 记录失败不影响检测结果本身。
    let problems = items.iter().filter(|item| item.status != "ok").count();
    let _ = record_verification(problems);
    Ok(items)
}

// ---------------- 最近验证记录（P1-1 环境证明卡） ----------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRecord {
    /// epoch 秒
    pub at: u64,
    /// 当次检测中 status != ok 的条数
    pub problems: usize,
}

fn verification_path() -> PathBuf {
    crate::cfg_dir().join("last-verification.json")
}

fn record_verification(problems: usize) -> Result<(), String> {
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let record = VerificationRecord { at, problems };
    fs::create_dir_all(crate::cfg_dir()).map_err(|e| e.to_string())?;
    crate::sync::write_json_atomic(
        &verification_path(),
        &serde_json::to_value(&record).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// 最近一次完整检测。从未检测过、文件损坏、时间戳来自未来（系统时钟被改过）
/// 都返回 `None` —— 环境证明卡会显示"尚未验证"，而不是给一个不可信的"刚刚"。
#[tauri::command]
pub fn last_verification() -> Option<VerificationRecord> {
    let text = fs::read_to_string(verification_path()).ok()?;
    let record: VerificationRecord = serde_json::from_str(&text).ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if record.at > now {
        return None;
    }
    Some(record)
}

// ---------------- 诊断报告导出 ----------------

// 不引入 chrono:用 Howard Hinnant 的 civil_from_days 算法把 epoch 秒转成 UTC 日期
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn fmt_utc(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let rem = secs % 86400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn status_tag(s: &str) -> &'static str {
    match s {
        "ok" => "[正常]",
        "warn" => "[警告]",
        _ => "[异常]",
    }
}

// 环境配置的脱敏摘要:绝不输出 token_enc / 任何密钥内容
fn profile_summary(p: &Profile) -> String {
    let mut s = format!(
        "- {}（{}）",
        p.name,
        if p.type_ == "router" {
            "网关环境"
        } else {
            "独立登录环境"
        }
    );
    if p.type_ == "router" {
        fn or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
            if s.is_empty() {
                fallback
            } else {
                s
            }
        }
        s += &format!(
            "\n    网关: {}\n    Key: {}\n    映射: opus={} sonnet={} haiku={}",
            or(&p.base_url, "<未配置>"),
            if p.has_token {
                "已保存（内容不导出）"
            } else {
                "未保存"
            },
            or(&p.opus_model, "<未设>"),
            or(&p.sonnet_model, "<未设>"),
            or(&p.haiku_model, "<未设>"),
        );
    }
    s
}

fn build_report(app_version: &str, items: &[HealthItem]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "其他"
    };
    let mut r = String::new();
    r += "=== cc-manager 诊断报告 ===\n";
    r += &format!(
        "生成时间: {}\nApp 版本: v{app_version}\n平台: {platform}\n",
        fmt_utc(now)
    );
    // curl 的 TLS 后端随平台/安装来源而异(Schannel / LibreSSL / OpenSSL)，它决定了
    // --cacert 的语义，是远程排查证书问题时最先要看的一行。
    r += &format!("curl: {}\n", crate::curl_version_line());

    r += "\n--- 健康检查 ---\n";
    for it in items {
        r += &format!("{} {} — {}\n", status_tag(&it.status), it.label, it.detail);
    }

    r += "\n--- 环境配置（已脱敏） ---\n";
    let list = load();
    if list.is_empty() {
        r += "（还没有环境）\n";
    }
    for p in &list {
        r += &profile_summary(p);
        r.push('\n');
    }

    r += "\n--- 各副本 settings.json 的 model 字段 ---\n";
    let mut keys = vec![MAIN_PROFILE_KEY.to_string()];
    keys.extend(profile_names(&list));
    for k in keys {
        let path = settings_path_for(&k);
        let who = if k == MAIN_PROFILE_KEY {
            "默认 Claude"
        } else {
            k.as_str()
        };
        let val = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|d| d.get("model").and_then(|m| m.as_str()).map(String::from));
        r += &format!("- {who}: {}\n", val.unwrap_or_else(|| "<未设置>".into()));
    }

    r += "\n--- 共享配置分发台账概要 ---\n";
    match fs::read_to_string(crate::shared_config::ledger_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        Some(v) => {
            if let Some(domains) = v.get("domains").and_then(|d| d.as_object()) {
                for (field, snap) in domains {
                    let n = snap
                        .get("envs")
                        .and_then(|s| s.as_object())
                        .map(|o| {
                            o.values()
                                .filter_map(|v| v.as_object())
                                .map(|o| o.len())
                                .sum::<usize>()
                        })
                        .unwrap_or(0);
                    let reps = snap
                        .get("envs")
                        .and_then(|s| s.as_object())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    r += &format!("- {field}: {n} 条分发记录，{reps} 个环境\n");
                }
            }
        }
        None => r += "（分发台账不存在或无法读取，请检查同步状态）\n",
    }

    r += "\n--- 同步日志（最近 200 行） ---\n";
    let log = fs::read_to_string(crate::cfg_dir().join("sync.log")).unwrap_or_default();
    if log.is_empty() {
        r += "（日志为空）\n";
    } else {
        let lines: Vec<&str> = log.lines().collect();
        let start = lines.len().saturating_sub(200);
        for l in &lines[start..] {
            r += l;
            r.push('\n');
        }
    }
    r
}

// 在资源管理器 / Finder 里定位到导出的文件(尽力而为,失败不影响导出结果)
fn reveal_file(path: &Path) {
    #[cfg(debug_assertions)]
    if std::env::var_os("CC_MANAGER_TEST_HOME").is_some() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
}

#[tauri::command]
pub fn export_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
    let version = app.package_info().version.to_string();
    let items = run_health_checks();
    let report = build_report(&version, &items);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((now / 86400) as i64);
    let rem = now % 86400;
    let name = format!(
        "cc-manager-诊断-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.txt",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    );
    let dir = crate::export_directory();
    let path = dir.join(name);
    fs::write(&path, report).map_err(|e| format!("写入诊断文件失败：{e}"))?;
    reveal_file(&path);
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_check_command_remains_async() {
        fn assert_future<T: std::future::Future>(_: T) {}
        assert_future(health_check());
    }

    #[test]
    fn model_alias_recognized_case_insensitive_with_1m_suffix() {
        assert!(is_model_alias("sonnet"));
        assert!(is_model_alias("Opus"));
        assert!(is_model_alias("DEFAULT"));
        assert!(is_model_alias("opusplan"));
        assert!(is_model_alias("sonnet[1m]"));
        assert!(is_model_alias(" haiku "));
        // 具体型号 ID 都不是别名
        assert!(!is_model_alias("claude-sonnet-4-6"));
        assert!(!is_model_alias("glm-5.2"));
        assert!(!is_model_alias("claude-opus-4-7-20260101"));
        assert!(!is_model_alias(""));
    }

    fn tmp_settings(content: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "ccm-health-test-{}-{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn pinned_model_detects_concrete_id_only() {
        let pinned = tmp_settings(r#"{"model":"glm-5.2","enabledPlugins":{}}"#);
        assert_eq!(pinned_model_in(&pinned), Some("glm-5.2".to_string()));
        let _ = fs::remove_file(&pinned);

        let alias = tmp_settings(r#"{"model":"sonnet"}"#);
        assert_eq!(pinned_model_in(&alias), None);
        let _ = fs::remove_file(&alias);

        let absent = tmp_settings(r#"{"enabledPlugins":{}}"#);
        assert_eq!(pinned_model_in(&absent), None);
        let _ = fs::remove_file(&absent);

        // 文件缺失 / 非法 JSON 都不告警
        assert_eq!(pinned_model_in(Path::new("Z:/no/such/file.json")), None);
        let broken = tmp_settings("not json");
        assert_eq!(pinned_model_in(&broken), None);
        let _ = fs::remove_file(&broken);
    }

    #[test]
    fn fix_model_pin_refuses_the_default_claude_copy() {
        // 决策 7.4：应用对默认 Claude 只读 —— 可以发现风险并说明原因，但不替用户改写。
        // 这个后端拒绝是**硬边界**：不依赖前端不画按钮，也不依赖调用方守规矩
        // （界面改错、或有人直接 invoke 命令，都必须被挡下）。
        // 拒绝发生在前面的分支，所以不需要任何真实配置文件存在。
        let err = fix_model_pin(MAIN_PROFILE_KEY.to_string()).unwrap_err();
        assert!(err.contains("不会修改"), "{err}");
        assert!(err.contains("/model"), "要告诉用户该去哪儿改：{err}");
    }

    #[test]
    fn fmt_utc_converts_known_dates() {
        assert_eq!(fmt_utc(0), "1970-01-01 00:00:00 UTC");
        // 2024-01-01 00:00:00 UTC
        assert_eq!(fmt_utc(1_704_067_200), "2024-01-01 00:00:00 UTC");
        // 2026-07-09 12:34:56 UTC（20643 天 * 86400 + 45296 秒）
        assert_eq!(fmt_utc(1_783_600_496), "2026-07-09 12:34:56 UTC");
    }

    #[test]
    fn gateway_ok_detail_states_imported_ca_dependency() {
        // 走通"系统信任"时不需要额外说明；走通"已导入 CA"时必须写清依赖，
        // 否则用户不知道它依赖 NODE_EXTRA_CA_CERTS(即必须从已接入集成的终端启动)。
        let sys = gateway_ok_detail(TrustMode::System, 12, false);
        assert!(sys.contains("可达"), "{sys}");
        assert!(sys.contains("12 个可用模型"), "{sys}");
        assert!(!sys.contains("已导入的 CA"), "{sys}");
        assert!(!sys.contains("回环"), "https 网关不该提回环：{sys}");

        let imported = gateway_ok_detail(TrustMode::ImportedCa, 12, false);
        assert!(imported.contains("可达"), "{imported}");
        assert!(imported.contains("已导入的 CA"), "{imported}");
        assert!(imported.contains("终端"), "{imported}");
    }

    #[test]
    fn gateway_ok_detail_discloses_plaintext_http() {
        // http 现在只可能是 loopback（远程 http 在保存时就被拒），
        // 但仍要说清"这一层没有 TLS"，别让"严格探测通过"被读成"传输是加密的"。
        let http = gateway_ok_detail(TrustMode::System, 3, true);
        assert!(http.contains("可达"), "{http}");
        assert!(http.contains("http://"), "{http}");
        assert!(http.contains("回环"), "{http}");
    }

    // ---------------- 凭证文件权限 ----------------

    /// SDDL 样本。**形状**取自真机的 `Get-Acl` 输出（含继承标记 `ID`、权限掩码 `0x1200a9`），
    /// 但**所有 SID 都是合成值** —— 真实机器 SID 与计算机名属机器标识，不得写进仓库。
    /// - 违规形态：除所有者/SYSTEM/Administrators 外，还有一个自定义组(RID -1006)被继承授予了 0x1200a9
    const SDDL_CUSTOM_GROUP_HAS_READ: &str = "O:S-1-5-21-1-2-3-1005G:S-1-5-21-1-2-3-513D:AI(A;ID;0x1200a9;;;S-1-5-21-1-2-3-1006)(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;FA;;;S-1-5-21-1-2-3-1005)";
    /// 合规形态：只有所有者 + SYSTEM + Administrators
    const SDDL_OWNER_SYSTEM_ADMIN_ONLY: &str = "O:S-1-5-21-1-2-3-1005G:S-1-5-21-1-2-3-513D:AI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;FA;;;S-1-5-21-1-2-3-1005)";

    #[test]
    fn sddl_owner_is_extracted() {
        assert_eq!(
            sddl_owner(SDDL_OWNER_SYSTEM_ADMIN_ONLY).as_deref(),
            Some("S-1-5-21-1-2-3-1005")
        );
    }

    #[test]
    fn custom_group_with_read_is_flagged() {
        // 真机上出现的形态。用"列举 Everyone/Users"的黑名单会漏掉自定义组，
        // 所以判据必须是白名单：除所有者/SYSTEM/Administrators 外谁都算违规。
        let found = sddl_unauthorized_reads(SDDL_CUSTOM_GROUP_HAS_READ);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("-1006"), "{found:?}");
        assert!(found[0].contains("0x1200a9"), "{found:?}");
    }

    #[test]
    fn owner_system_and_administrators_are_not_flagged() {
        // 管理员/SYSTEM 不在"文件权限能防御"的范围内，出现它们不算违规
        assert!(sddl_unauthorized_reads(SDDL_OWNER_SYSTEM_ADMIN_ONLY).is_empty());
    }

    #[test]
    fn everyone_and_builtin_users_are_flagged() {
        let sddl =
            "O:S-1-5-21-1-2-3-1001D:(A;;FA;;;S-1-1-0)(A;;FA;;;S-1-5-32-545)(A;;FA;;;S-1-5-11)";
        let found = sddl_unauthorized_reads(sddl);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found[0].contains("S-1-1-0"));
        assert!(found[1].contains("S-1-5-32-545"));
        assert!(found[2].contains("S-1-5-11"));
    }

    #[test]
    fn deny_aces_and_write_only_grants_are_not_flagged() {
        // 拒绝型 ACE 不是"给了权限"
        assert!(sddl_unauthorized_reads("O:S-1-5-21-1-2-3-1001D:(D;;FA;;;S-1-1-0)").is_empty());
        // 只有写权限不算可读
        assert!(sddl_unauthorized_reads("O:S-1-5-21-1-2-3-1001D:(A;;FW;;;S-1-1-0)").is_empty());
    }

    #[test]
    fn read_right_detection_covers_codes_and_masks() {
        // 文件专有 + 通用映射 + 十六进制掩码，三种写法都要认
        for readable in ["FA", "FR", "RX", "GR", "GA", "0x1200a9", "0x120089"] {
            assert!(rights_allow_read(readable), "{readable} 应判为可读");
        }
        for not_readable in ["FW", "FX", "GW", "GX", "WD", "WO", "RC", "SD", "0x100000"] {
            assert!(
                !rights_allow_read(not_readable),
                "{not_readable} 不该判为可读"
            );
        }
        // 认不出来宁可误报，不能漏报（fail-closed）——漏报会让界面谎报"仅限本人读取"
        assert!(rights_allow_read("0xzz"));
        assert!(rights_allow_read("ZZ"));
    }

    #[test]
    fn generic_read_ace_is_flagged() {
        // 回归：曾只认 FA/FR/RX，`(A;;GR;;;SID)` 这种合法 SDDL 会被判成不可读而放过。
        let found = sddl_unauthorized_reads("O:S-1-5-21-1-2-3-1001D:(A;;GR;;;S-1-1-0)");
        assert_eq!(found.len(), 1, "GR 被漏掉了：{found:?}");
        assert!(found[0].contains("S-1-1-0"), "{found:?}");

        // GA（Generic All）同样含读取
        let ga = sddl_unauthorized_reads("O:S-1-5-21-1-2-3-1001D:(A;;GA;;;S-1-5-32-545)");
        assert_eq!(ga.len(), 1, "GA 被漏掉了：{ga:?}");

        // 未知写法也不能放过
        let unknown = sddl_unauthorized_reads("O:S-1-5-21-1-2-3-1001D:(A;;QQ;;;S-1-1-0)");
        assert_eq!(unknown.len(), 1, "未知权限码被放过了：{unknown:?}");
    }

    #[test]
    fn path_under_home_handles_both_platform_shapes() {
        let win_home = Path::new("C:\\Users\\hq");
        assert!(path_under_home(
            Path::new("C:\\Users\\hq\\.cc-manager\\config.json"),
            win_home
        ));
        // Windows 路径大小写不敏感
        assert!(path_under_home(
            Path::new("c:\\users\\HQ\\.workbuddy\\models.json"),
            win_home
        ));
        assert!(path_under_home(
            Path::new("/Users/hq/.cc-manager/config.json"),
            Path::new("/Users/hq")
        ));
        assert!(!path_under_home(
            Path::new("D:\\shared\\config.json"),
            win_home
        ));
        assert!(!path_under_home(
            Path::new("/tmp/config.json"),
            Path::new("/Users/hq")
        ));
        // 前缀相似但不是子目录，不能误判成"在主目录下"
        assert!(!path_under_home(
            Path::new("C:\\Users\\hqother\\x.json"),
            win_home
        ));
    }

    #[test]
    fn profile_summary_never_leaks_token() {
        let p = Profile {
            name: "corp".into(),
            type_: "router".into(),
            base_url: "https://gw.example.com/anthropic".into(),
            token_enc: Some("SECRET-ENCRYPTED-BLOB".into()),
            has_token: true,
            opus_model: "glm-5.2".into(),
            sonnet_model: String::new(),
            haiku_model: String::new(),
        };
        let s = profile_summary(&p);
        assert!(!s.contains("SECRET-ENCRYPTED-BLOB"));
        assert!(s.contains("已保存（内容不导出）"));
        assert!(s.contains("glm-5.2"));
        assert!(s.contains("<未设>"));
    }
}
