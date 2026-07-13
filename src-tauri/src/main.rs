#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

mod claude_cli;
mod health;
mod sync;

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
    claude_found: bool,
    integrated: bool,
    cert_imported: bool,
    cert_count: usize,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionGroup {
    kind: String,
    label: String,
    path: String,
    items: Vec<String>,
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

fn directory_items(path: &std::path::Path) -> Vec<String> {
    let mut items = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(String::from))
        .filter(|name| !name.starts_with('.'))
        .collect::<Vec<_>>();
    items.sort_by_key(|name| name.to_ascii_lowercase());
    items
}

// ---------------- 路径 ----------------
fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
fn cfg_dir() -> PathBuf {
    home().join(".cc-manager")
}
fn cfg_path() -> PathBuf {
    cfg_dir().join("config.json")
}
fn cfg_backup_path() -> PathBuf {
    cfg_dir().join("config.backup.json")
}
fn cert_path() -> PathBuf {
    cfg_dir().join("ca-cert.pem")
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

fn valid_base_url(value: &str) -> bool {
    let value = normalize_base_url(value);
    (value.starts_with("https://") || value.starts_with("http://"))
        && value.len() <= 2048
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

// ---------------- shell 引用 ----------------
fn sh_q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn ps_q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

// ---------------- 生成集成脚本 ----------------
fn generate_sh(list: &[Profile]) -> String {
    let mut o = String::from(
        "#!/bin/bash\n# Auto-generated by Claude Center. Do not edit.\n",
    );
    o += "if [ -f \"$HOME/.cc-manager/ca-cert.pem\" ]; then export NODE_EXTRA_CA_CERTS=\"$HOME/.cc-manager/ca-cert.pem\"; fi\n";
    // 每次启动/退出 claude 前后调本程序 --sync:维护共享链接 + 合并 MCP/插件启用状态
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    o += &format!("_ccm_exe={}\n", sh_q(&exe));
    o += "_ccm_sync() { if [ -x \"$_ccm_exe\" ]; then \"$_ccm_exe\" --sync >/dev/null 2>&1; fi; return 0; }\n";
    o += "claude() {\n  case \"$1\" in\n";
    for p in list {
        let n = &p.name;
        if !script_safe_name(n) {
            continue; // 不安全名字绝不写进脚本；install_integration 会对此告警
        }
        o += &format!("    {n})\n");
        o += &format!(
            "      shift; _cch=\"$HOME/.claude-split/{n}/.claude\"; mkdir -p \"$_cch\"\n"
        );
        if p.type_ == "router" {
            let url = sh_q(&p.base_url);
            let svc = sh_q(&format!("{KEYCHAIN_PREFIX}:{n}"));
            let mut envs = format!(
                "CLAUDE_CONFIG_DIR=\"$_cch\" ANTHROPIC_BASE_URL={url} ANTHROPIC_AUTH_TOKEN=\"$(security find-generic-password -s {svc} -w 2>/dev/null)\" CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1"
            );
            if !p.opus_model.is_empty() {
                envs += &format!(" ANTHROPIC_DEFAULT_OPUS_MODEL={}", sh_q(&p.opus_model));
            }
            if !p.sonnet_model.is_empty() {
                envs += &format!(" ANTHROPIC_DEFAULT_SONNET_MODEL={}", sh_q(&p.sonnet_model));
            }
            if !p.haiku_model.is_empty() {
                envs += &format!(" ANTHROPIC_DEFAULT_HAIKU_MODEL={}", sh_q(&p.haiku_model));
            }
            o += "      _ccm_sync\n";
            o += &format!("      {envs} command claude \"$@\"\n");
            o += "      local _rc=$?; _ccm_sync; return $_rc ;;\n";
        } else {
            o += "      _ccm_sync\n";
            o += "      CLAUDE_CONFIG_DIR=\"$_cch\" command claude \"$@\"\n";
            o += "      local _rc=$?; _ccm_sync; return $_rc ;;\n";
        }
    }
    o += "    *) _ccm_sync; command claude \"$@\"; local _rc=$?; _ccm_sync; return $_rc ;;\n  esac\n}\n";
    o
}

fn generate_ps1(list: &[Profile]) -> String {
    let mut o = String::new();
    o += "# Auto-generated by Claude Center. Do not edit.\n";
    o += "if (Test-Path \"$env:USERPROFILE\\.cc-manager\\ca-cert.pem\") { $env:NODE_EXTRA_CA_CERTS = \"$env:USERPROFILE\\.cc-manager\\ca-cert.pem\" }\n";
    o += "function claude {\n";
    o += "  param([Parameter(ValueFromRemainingArguments=$true)][string[]]$A)\n";
    o += "  $exe = $null\n";
    o += "  foreach ($cand in @('claude.cmd','claude.exe','claude.bat')) {\n";
    o += "    $c = Get-Command $cand -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1\n";
    o += "    if ($c) { $exe = $c.Source; break }\n";
    o += "  }\n";
    o += "  if (-not $exe) { Write-Host 'claude not found in PATH'; return }\n";
    // GUI 子系统 exe 用 & 调用不会等待,pre-sync 必须 Start-Process -Wait;post-sync 可不等
    let ccm_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    o += &format!("  $ccm = {}\n", ps_q(&ccm_exe));
    o += "  function _ccmSync([switch]$NoWait) { if ($ccm -and (Test-Path $ccm)) { try { if ($NoWait) { Start-Process -FilePath $ccm -ArgumentList '--sync' -WindowStyle Hidden } else { Start-Process -FilePath $ccm -ArgumentList '--sync' -Wait -WindowStyle Hidden } } catch {} } }\n";
    o += "  $sub = if ($A.Count -ge 1) { $A[0] } else { '' }\n";
    o += "  $rest = if ($A.Count -gt 1) { $A[1..($A.Count-1)] } else { @() }\n";
    o += "  switch ($sub) {\n";
    for p in list {
        let n = &p.name;
        if !script_safe_name(n) {
            continue; // 不安全名字绝不写进脚本；install_integration 会对此告警
        }
        o += &format!("    {} {{\n", ps_q(n));
        o += &format!("      $h = Join-Path $env:USERPROFILE '.claude-split\\{n}\\.claude'\n");
        o += "      New-Item -ItemType Directory -Force -Path $h | Out-Null\n";
        o += "      $bk=@{CLAUDE_CONFIG_DIR=$env:CLAUDE_CONFIG_DIR;ANTHROPIC_BASE_URL=$env:ANTHROPIC_BASE_URL;ANTHROPIC_AUTH_TOKEN=$env:ANTHROPIC_AUTH_TOKEN;ANTHROPIC_DEFAULT_OPUS_MODEL=$env:ANTHROPIC_DEFAULT_OPUS_MODEL;ANTHROPIC_DEFAULT_SONNET_MODEL=$env:ANTHROPIC_DEFAULT_SONNET_MODEL;ANTHROPIC_DEFAULT_HAIKU_MODEL=$env:ANTHROPIC_DEFAULT_HAIKU_MODEL;CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=$env:CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC}\n";
        o += "      try {\n";
        o += "        _ccmSync\n";
        o += "        $env:CLAUDE_CONFIG_DIR=$h\n";
        if p.type_ == "router" {
            o += &format!("        $env:ANTHROPIC_BASE_URL={}\n", ps_q(&p.base_url));
            o += "        $env:CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1'\n";
            if let Some(enc) = &p.token_enc {
                if !enc.is_empty() {
                    o += &format!("        $sec=ConvertTo-SecureString {}\n", ps_q(enc));
                    o += "        $b=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec)\n";
                    o += "        $env:ANTHROPIC_AUTH_TOKEN=[Runtime.InteropServices.Marshal]::PtrToStringBSTR($b)\n";
                    o += "        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($b)\n";
                }
            }
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
        o += "      } finally {\n";
        o += "        foreach ($k in $bk.Keys) { if ($bk[$k]) { Set-Item -Path \"Env:\\$k\" -Value $bk[$k] } else { Remove-Item -Path \"Env:\\$k\" -ErrorAction SilentlyContinue } }\n";
        o += "        _ccmSync -NoWait\n";
        o += "      }\n";
        o += "    }\n";
    }
    o += "    default { _ccmSync; & $exe @A; _ccmSync -NoWait }\n";
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

// 旧方案重定向 HOME/USERPROFILE，实例的 .claude.json 落在实例根目录；
// 新方案用 CLAUDE_CONFIG_DIR 指向 <实例>/.claude，CLI 会到该目录下找 .claude.json，
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

// 实例名会被直接拼进生成的 cc.sh(bash case 分支)/cc.ps1(PowerShell switch 分支)里，
// 必须限制为安全字符集，否则特殊字符(如 ) ; $ ` ' 空格)会破坏脚本语法，导致 claude 函数整体失效。
// 只约束【新建】实例；已存在于 config.json 的旧名字走 script_safe_name 的生成期兜底。
fn valid_name(n: &str) -> bool {
    if n.is_empty()
        || n.len() > 40
        || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return false;
    }
    // __ 前缀保留给内部哨兵值(前端用 __all__ 作筛选哨兵、后端用 __main__ 标记主账户)
    if n.starts_with("__") {
        return false;
    }
    // Windows 保留设备名无法作为目录名创建(.claude-split/<name> 会失败)
    !matches!(
        n.to_ascii_lowercase().as_str(),
        "con" | "prn" | "aux" | "nul"
            | "com1" | "com2" | "com3" | "com4" | "com5" | "com6" | "com7" | "com8" | "com9"
            | "lpt1" | "lpt2" | "lpt3" | "lpt4" | "lpt5" | "lpt6" | "lpt7" | "lpt8" | "lpt9"
    )
}

// 脚本生成期的最后防线：存量配置里可能有旧规则时代保存的名字(中文/点号等)，
// 它们本身无害、继续放行；只拦截会破坏 bash case 分支或引号/路径语法的字符。
// is_alphanumeric 按 Unicode 判定，中文字母数字均通过；空白、) ; $ ` ' " 等一律拦下。
fn script_safe_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn profile_names(list: &[Profile]) -> Vec<String> {
    list.iter()
        .map(|p| p.name.clone())
        .filter(|n| !n.is_empty())
        .collect()
}

fn install_integration(list: &[Profile]) -> Result<String, String> {
    fs::create_dir_all(cfg_dir()).map_err(|e| e.to_string())?;
    migrate_instances(list);
    // 所有实例始终与主账户共享 skills/plugins/agents/commands(幂等)
    let mut link_warns = sync::ensure_links(&profile_names(list)).unwrap_or_else(|e| vec![e]);
    // 名字含不安全字符的实例不会被写进终端脚本(generate_sh/ps1 里跳过)，明确告知而不是静默失效
    let unsafe_names: Vec<&str> = list
        .iter()
        .map(|p| p.name.as_str())
        .filter(|n| !n.is_empty() && !script_safe_name(n))
        .collect();
    if !unsafe_names.is_empty() {
        link_warns.push(format!(
            "实例 {} 的名称含不安全字符，已跳过、不会接入终端，请删除后用合规名称重建",
            unsafe_names.join("、")
        ));
    }
    let warn_suffix = if link_warns.is_empty() {
        String::new()
    } else {
        format!("（注意：{}）", link_warns.join("；"))
    };
    if cfg!(target_os = "windows") {
        fs::write(ps_path(), generate_ps1(list)).map_err(|e| e.to_string())?;
        let out = ps_command()
            .args(["-NoProfile", "-Command", "$PROFILE"])
            .output()
            .map_err(|e| e.to_string())?;
        let profile_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if profile_path.is_empty() {
            return Ok("已生成 cc.ps1，但未能定位 PowerShell $PROFILE，请手动 dot-source 它。".into());
        }
        let line = format!(". \"{}\"  {}", ps_path().display(), MARK);
        ensure_line(&PathBuf::from(&profile_path), &line).map_err(|e| e.to_string())?;
        Ok(format!(
            "已接入 PowerShell $PROFILE。请新开一个 PowerShell 窗口生效。{warn_suffix}"
        ))
    } else {
        fs::write(sh_path(), generate_sh(list)).map_err(|e| e.to_string())?;
        let line = format!(
            "[ -f \"{p}\" ] && source \"{p}\"  {m}",
            p = sh_path().display(),
            m = MARK
        );
        let _ = ensure_line(&home().join(".zshrc"), &line);
        let _ = ensure_line(&home().join(".bashrc"), &line);
        Ok(format!(
            "已接入 shell 配置。请新开一个终端窗口（或 source 一下）生效。{warn_suffix}"
        ))
    }
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
    }
    c
}

fn store_token(name: &str, token: &str) -> Result<Option<String>, String> {
    if cfg!(target_os = "macos") {
        let svc = format!("{KEYCHAIN_PREFIX}:{name}");
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let _ = std::process::Command::new("security")
            .args(["delete-generic-password", "-s", &svc])
            .output();
        let out = std::process::Command::new("security")
            .args([
                "add-generic-password", "-a", &user, "-s", &svc, "-w", token, "-U",
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(None)
    } else if cfg!(target_os = "windows") {
        // PowerShell 5.1 的 -Command 模式下 $args 不可靠，直接把 key 拼进脚本（单引号转义）
        let escaped = token.replace('\'', "''");
        let script = format!(
            "ConvertTo-SecureString -String '{escaped}' -AsPlainText -Force | ConvertFrom-SecureString"
        );
        let out = ps_command()
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map_err(|e| e.to_string())?;
        let enc = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if enc.is_empty() {
            return Err(format!(
                "PowerShell 加密失败：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(Some(enc))
    } else {
        Err("当前平台不支持安全存储 token".into())
    }
}

fn clear_token(name: &str) {
    if cfg!(target_os = "macos") {
        let svc = format!("{KEYCHAIN_PREFIX}:{name}");
        let _ = std::process::Command::new("security")
            .args(["delete-generic-password", "-s", &svc])
            .output();
    }
}

// ---------------- claude 探测 ----------------
fn claude_found() -> bool {
    claude_cli::resolve_claude_exe().is_some()
}

// ---------------- 命令 ----------------
#[tauri::command]
fn list_profiles() -> Vec<Profile> {
    load()
}

#[tauri::command]
fn save_profile(profile: Profile, token: Option<String>) -> Result<String, String> {
    let list = load();
    let mut p = profile;
    // 先归一化再校验：校验和存储必须是同一个字符串
    p.name = p.name.trim().to_string();
    p.base_url = normalize_base_url(&p.base_url);
    // 只校验新建；已存在的名字（旧规则时代创建）放行，否则老用户连换 key 都保存不了
    let is_update = list.iter().any(|x| x.name == p.name);
    if !is_update && !valid_name(&p.name) {
        return Err(
            "实例名称只能包含英文字母、数字、下划线、短横线（1~40 个字符），且不能以 __ 开头或使用 Windows 保留名。".into(),
        );
    }
    if p.type_ == "router" {
        if !valid_base_url(&p.base_url) {
            return Err("网关地址必须是有效的 http:// 或 https:// 地址，且不能包含空格。".into());
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
    install_integration(&list)
}

#[tauri::command]
fn delete_profile(name: String, purge_data: bool) -> Result<String, String> {
    let mut list = load();
    if !list.iter().any(|x| x.name == name) {
        return Err("未找到要删除的实例。".into());
    }
    if purge_data {
        purge_instance_data(&name)?;
    }
    list.retain(|x| x.name != name);
    clear_token(&name);
    save(&list).map_err(|e| e.to_string())?;
    install_integration(&list)?;
    Ok(if purge_data {
        "实例及其登录态、项目记录和历史用量数据已彻底删除。".into()
    } else {
        "实例已从列表移除，历史数据仍保留，可通过同名实例恢复。".into()
    })
}

fn purge_instance_data(name: &str) -> Result<(), String> {
    // 名称来自配置文件，仍要防御被手工篡改后的路径穿越。
    if name == "." || name == ".." || !script_safe_name(name) {
        return Err("实例名称不安全，拒绝删除数据目录。请手动检查配置。".into());
    }
    let root = home().join(".claude-split").join(name);
    if !root.exists() {
        return Ok(());
    }
    // 先显式解除共享目录链接/Junction，确保递归删除永远不会触及主账户目录。
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
    fs::remove_dir_all(&root).map_err(|e| format!("删除实例数据 {} 失败：{e}", root.display()))
}

// GUI 启动时调用:刷新集成脚本(exe 路径可能变化)+ 建齐共享链接 + 跑一轮配置合并
#[tauri::command]
fn sync_all() -> Result<String, String> {
    let list = load();
    let msg = install_integration(&list)?; // 内含 ensure_links
    let sync_msg = sync::sync_configs(&profile_names(&list))?;
    Ok(format!("{msg}（{sync_msg}）"))
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
    EnvInfo {
        platform,
        claude_found: claude_found(),
        integrated: cfg_path().exists(),
        cert_imported: cert_path().exists(),
        cert_count: count_certs(),
    }
}

#[tauri::command]
fn profile_runtime_info() -> Vec<ProfileRuntimeInfo> {
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
            let authenticated = config.join(".claude.json").is_file()
                || config.join(".credentials.json").is_file();
            let shared_dirs_ok = sync::broken_links(&[profile.name.clone()]).is_empty();
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
fn extension_overview() -> Vec<ExtensionGroup> {
    let master = home().join(".claude");
    let mut groups = [
        ("skills", "Skills", master.join("skills")),
        ("plugins", "Plugins", master.join("plugins")),
        ("agents", "Agents", master.join("agents")),
        ("commands", "Commands", master.join("commands")),
    ]
    .into_iter()
    .map(|(kind, label, path)| ExtensionGroup {
        kind: kind.into(),
        label: label.into(),
        items: directory_items(&path),
        path: path.display().to_string(),
    })
    .collect::<Vec<_>>();

    let claude_json = home().join(".claude.json");
    let mcp_items = fs::read_to_string(&claude_json)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("mcpServers").and_then(|v| v.as_object()).cloned())
        .map(|map| {
            let mut names = map.keys().cloned().collect::<Vec<_>>();
            names.sort();
            names
        })
        .unwrap_or_default();
    groups.push(ExtensionGroup {
        kind: "mcp".into(),
        label: "MCP Servers".into(),
        path: claude_json.display().to_string(),
        items: mcp_items,
    });
    groups
}

#[tauri::command]
fn backup_config() -> Result<String, String> {
    let source = cfg_path();
    if !source.is_file() {
        return Err("当前还没有可备份的实例配置。".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let dir = dirs::desktop_dir().unwrap_or_else(cfg_dir);
    let target = dir.join(format!("claude-environment-config-{now}.json"));
    fs::copy(&source, &target).map_err(|e| format!("导出配置备份失败：{e}"))?;
    Ok(target.display().to_string())
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

// 统计信任库里已有几张证书（按 PEM 块计数）
fn count_certs() -> usize {
    fs::read_to_string(cert_path())
        .map(|s| s.matches("-----BEGIN CERTIFICATE-----").count())
        .unwrap_or(0)
}

// 导入 CA 证书：去重追加到固定 bundle 文件，并刷新集成脚本（脚本顶部会设置 NODE_EXTRA_CA_CERTS）
// 一个文件可放多张 PEM 证书，Node 会全部信任，故不同网关的不同证书都能共存、所有实例通用。
#[tauri::command]
fn import_cert(path: String) -> Result<String, String> {
    let src = PathBuf::from(path.trim());
    if !src.exists() {
        return Err("证书文件不存在，请确认路径。".into());
    }
    let new_cert = fs::read_to_string(&src).map_err(|e| format!("读取证书失败：{e}"))?;
    if !new_cert.contains("-----BEGIN CERTIFICATE-----") {
        return Err("文件里没找到 PEM 证书（缺 -----BEGIN CERTIFICATE-----）。".into());
    }
    fs::create_dir_all(cfg_dir()).map_err(|e| e.to_string())?;
    let dest = cert_path();
    let existing = fs::read_to_string(&dest).unwrap_or_default();
    // 去重：按"去掉所有换行"后的整段文本比对，已存在则跳过
    let norm = |s: &str| s.replace(['\r', '\n'], "");
    if norm(&existing).contains(&norm(&new_cert)) {
        return Ok(format!(
            "该证书已在信任库中（当前共 {} 张），无需重复导入。",
            count_certs()
        ));
    }
    let sep = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut to_write = format!("{existing}{sep}{new_cert}");
    if !to_write.ends_with('\n') {
        to_write.push('\n');
    }
    fs::write(&dest, to_write).map_err(|e| format!("写入证书失败：{e}"))?;
    // 重新生成集成脚本，使其包含 NODE_EXTRA_CA_CERTS
    let list = load();
    let _ = install_integration(&list);
    Ok(format!(
        "证书已导入（累计 {} 张）。请重开终端后生效。",
        count_certs()
    ))
}

// 清空所有 CA 证书
#[tauri::command]
fn clear_certs() -> Result<String, String> {
    let p = cert_path();
    if p.exists() {
        fs::remove_file(&p).map_err(|e| format!("删除证书失败：{e}"))?;
    }
    let list = load();
    let _ = install_integration(&list);
    Ok("已清空所有 CA 证书。请重开终端后生效。".into())
}

// 检测网关可用模型：请求 {base_url}/v1/models
#[tauri::command]
fn detect_models(base_url: String, token: String) -> Result<Vec<String>, String> {
    let base = base_url.trim().trim_end_matches('/');
    if !valid_base_url(base) {
        return Err("网关地址必须是有效的 http:// 或 https:// 地址，且不能包含空格。".into());
    }
    if token.trim().is_empty() {
        return Err("请先填写 API Key（检测需要鉴权）。".into());
    }
    let url = format!("{base}/v1/models");
    let curl = if cfg!(target_os = "windows") {
        "curl.exe"
    } else {
        "curl"
    };
    // 用 -K - 从 stdin 读取参数（含 Authorization 头），而不是拼进命令行参数，
    // 避免 key 明文出现在本机进程列表(如 Windows 任务管理器/ps)的命令行里。
    // curl 配置文件按行解析：先剔除控制字符(嵌入换行会跳出引号注入任意 curl 指令)，再转义 \ 和 "。
    let esc = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    // 带超时:健康检查会对每个路由实例并行探测,不能被无响应的网关挂死。
    // silent 会吞掉错误原因,配合 show-error 让"无返回内容"时 stderr 里有真实原因可展示。
    let config = format!(
        "silent\nshow-error\ninsecure\nconnect-timeout = 5\nmax-time = 15\nheader = \"Authorization: Bearer {}\"\nurl = \"{}\"\n",
        esc(token.trim()),
        esc(&url)
    );
    let mut cmd = std::process::Command::new(curl);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.arg("-K").arg("-");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("调用 curl 失败：{e}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(config.as_bytes())
            .map_err(|e| format!("写入 curl 参数失败：{e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("调用 curl 失败：{e}"))?;
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    if body.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("无返回内容。可能是证书或网络问题：{}", err.trim()));
    }
    let v: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| format!("返回不是有效 JSON：{}", body.chars().take(160).collect::<String>()))?;
    let mut ids = vec![];
    if let Some(arr) = v.get("data").and_then(|d| d.as_array()) {
        for m in arr {
            if let Some(id) = m.get("id").and_then(|i| i.as_str()) {
                ids.push(id.to_string());
            }
        }
    }
    if ids.is_empty() {
        return Err(format!(
            "未解析到模型列表：{}",
            body.chars().take(160).collect::<String>()
        ));
    }
    Ok(ids)
}

// ---------------- 解密已存 key / 按实例检测模型 ----------------
fn decrypt_token(p: &Profile) -> Result<String, String> {
    if cfg!(target_os = "macos") {
        let svc = format!("{KEYCHAIN_PREFIX}:{}", p.name);
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", &svc, "-w"])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err("未在钥匙串找到该实例的 Key".into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else if cfg!(target_os = "windows") {
        let enc = match &p.token_enc {
            Some(e) if !e.is_empty() => e.clone(),
            _ => return Err("该实例没有保存 Key".into()),
        };
        let escaped = enc.replace('\'', "''");
        let script = format!(
            "$sec=ConvertTo-SecureString '{escaped}'; $b=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec); [Runtime.InteropServices.Marshal]::PtrToStringBSTR($b)"
        );
        let out = ps_command()
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map_err(|e| e.to_string())?;
        let tok = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if tok.is_empty() {
            return Err("解密 Key 失败".into());
        }
        Ok(tok)
    } else {
        Err("当前平台不支持".into())
    }
}

#[tauri::command]
fn detect_models_for(name: String) -> Result<Vec<String>, String> {
    let list = load();
    let p = list
        .iter()
        .find(|x| x.name == name)
        .ok_or_else(|| "未找到该实例".to_string())?;
    if p.type_ != "router" {
        return Err("该实例不是自定义路由，没有可检测的网关".into());
    }
    if p.base_url.is_empty() {
        return Err("该实例未配置网关地址".into());
    }
    let token = decrypt_token(p)?;
    detect_models(p.base_url.clone(), token)
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

// 扫描单个实例(或主账户)的 projects 目录，把用量/对话记录累加进 map/conv。
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
        cache.insert(f, UsageFileCache { mtime, size, rows, convs });
    }
}

// 解析单个会话 jsonl，返回该文件内按 (datetime, model) 聚合的用量行和对话记录。
fn parse_usage_file(profile: &str, f: &PathBuf) -> (Vec<UsageRow>, Vec<ConvRow>) {
    let mut map: std::collections::HashMap<(String, String), UsageRow> =
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
        let gi = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let input = gi("input_tokens");
        let output = gi("output_tokens");
        let cache_read = gi("cache_read_input_tokens");
        let cache_create = gi("cache_creation_input_tokens");
        if input + output + cache_read + cache_create == 0 {
            continue;
        }
        let ts = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
        if ts.len() < 13 {
            continue;
        }
        let datetime = ts[..13].to_string(); // 例如 2026-06-22T04
        let key = (datetime.clone(), model.to_string());
        let row = map.entry(key).or_insert(UsageRow {
            datetime,
            model: model.to_string(),
            profile: profile.to_string(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_create: 0,
            requests: 0,
        });
        row.input += input;
        row.output += output;
        row.cache_read += cache_read;
        row.cache_create += cache_create;
        row.requests += 1;
    }
    (map.into_values().collect(), conv)
}

// 主账户在用量数据里的稳定键；前端负责把它映射为显示文案（与 __all__ 哨兵同一命名空间，
// valid_name 已保留 __ 前缀，新实例不可能占用；见 UsagePanel 的 profileOpts）。
const MAIN_PROFILE_KEY: &str = "__main__";

// async：首次全量扫描可能较慢（主账户历史可达数百 MB），放到异步线程执行，
// 不阻塞主线程；之后的调用命中文件缓存，只重新解析有变动的活跃会话文件。
fn collect_usage_stats() -> UsageStats {
    use std::collections::HashMap;
    let mut map: HashMap<(String, String, String), UsageRow> = HashMap::new();
    let mut conv: Vec<ConvRow> = Vec::new();

    // 主账户：直接跑 `claude`(不带子命令)的会话落在 ~/.claude/projects
    let main_projects = home().join(".claude").join("projects");
    scan_usage_dir(MAIN_PROFILE_KEY, &main_projects, &mut map, &mut conv);

    let split = home().join(".claude-split");
    if let Ok(insts) = fs::read_dir(&split) {
        for inst in insts.flatten() {
            if !inst.path().is_dir() {
                continue;
            }
            let profile = inst.file_name().to_string_lossy().to_string();
            // 与主账户键同名的历史遗留目录跳过，宁可不显示也不能混进主账户数据
            if profile == MAIN_PROFILE_KEY {
                continue;
            }
            let projects = inst.path().join(".claude").join("projects");
            scan_usage_dir(&profile, &projects, &mut map, &mut conv);
        }
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
        let names = profile_names(&load());
        match sync::ensure_links(&names) {
            Ok(warns) => {
                for w in warns {
                    sync::log_line(&w);
                }
            }
            Err(e) => sync::log_line(&format!("ensure_links 失败:{e}")),
        }
        if let Err(e) = sync::sync_configs(&names) {
            sync::log_line(&format!("sync_configs 失败:{e}"));
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
        .setup(|app| {
            let icon = tauri::image::Image::from_bytes(include_bytes!(
                "../icons/64x64.png"
            ))?;
            if let Some(window) = app.get_webview_window("main") {
                window.set_icon(icon)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            save_profile,
            delete_profile,
            sync_all,
            environment,
            profile_runtime_info,
            extension_overview,
            backup_config,
            recent_sync_log,
            import_cert,
            clear_certs,
            detect_models,
            detect_models_for,
            usage_stats,
            health::model_pin_warnings,
            health::fix_model_pin,
            health::health_check,
            health::export_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn config_save_keeps_previous_valid_version_as_backup() {
        let (primary, backup, dir) = temp_config_paths("backup");
        save_to(&primary, &backup, &[router_profile("first")]).unwrap();
        save_to(&primary, &backup, &[router_profile("second")]).unwrap();
        assert_eq!(parse_profiles(&primary).unwrap()[0].name, "second");
        assert_eq!(parse_profiles(&backup).unwrap()[0].name, "first");
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
        assert_eq!(fs::read_to_string(primary.with_extension("corrupt.json")).unwrap(), "not json");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_save_replaces_corrupt_primary_without_losing_evidence() {
        let (primary, backup, dir) = temp_config_paths("replace-corrupt");
        fs::write(&primary, "truncated {").unwrap();
        save_to(&primary, &backup, &[router_profile("fresh")]).unwrap();
        assert_eq!(parse_profiles(&primary).unwrap()[0].name, "fresh");
        assert_eq!(fs::read_to_string(primary.with_extension("corrupt.json")).unwrap(), "truncated {");
        assert!(parse_profiles(&backup).is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn base_url_requires_http_scheme_and_rejects_whitespace() {
        assert!(valid_base_url("https://gateway.example.com/anthropic"));
        assert!(valid_base_url("  https://gateway.example.com/anthropic/  "));
        assert!(valid_base_url("http://10.0.0.2:8080/anthropic"));
        assert!(!valid_base_url("gateway.example.com"));
        assert!(!valid_base_url("https://example.com/a b"));
        assert!(!valid_base_url("file:///tmp/config"));
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
        assert!(script.contains("*) _ccm_sync;"));
    }

    #[test]
    fn generate_ps1_creates_switch_branch_for_each_profile() {
        let list = vec![router_profile("corp")];
        let script = generate_ps1(&list);
        assert!(script.contains("function claude"));
        assert!(script.contains("'corp' {"));
        assert!(script.contains("ANTHROPIC_BASE_URL"));
    }
}
