#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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
    share_skills: bool,
    #[serde(default)]
    share_plugins: bool,
}

#[derive(Serialize)]
struct EnvInfo {
    platform: String,
    claude_found: bool,
    integrated: bool,
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
fn sh_path() -> PathBuf {
    cfg_dir().join("cc.sh")
}
fn ps_path() -> PathBuf {
    cfg_dir().join("cc.ps1")
}

// ---------------- 配置读写 ----------------
fn load() -> Vec<Profile> {
    if let Ok(s) = fs::read_to_string(cfg_path()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Some(arr) = v.get("profiles") {
                if let Ok(list) = serde_json::from_value::<Vec<Profile>>(arr.clone()) {
                    return list;
                }
            }
        }
    }
    vec![]
}

fn save(list: &[Profile]) -> std::io::Result<()> {
    fs::create_dir_all(cfg_dir())?;
    let obj = serde_json::json!({ "profiles": list });
    fs::write(cfg_path(), serde_json::to_string_pretty(&obj)?)
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
        "#!/bin/bash\n# 由 Claude Code 配置工具自动生成，请勿手动编辑。\nclaude() {\n  case \"$1\" in\n",
    );
    for p in list {
        let n = &p.name;
        if n.is_empty() {
            continue;
        }
        o += &format!("    {n})\n");
        o += &format!(
            "      shift; _cch=\"$HOME/.claude-split/{n}\"; mkdir -p \"$_cch/.local/bin\"\n"
        );
        if p.type_ == "router" {
            let url = sh_q(&p.base_url);
            let svc = sh_q(&format!("{KEYCHAIN_PREFIX}:{n}"));
            o += &format!(
                "      HOME=\"$_cch\" ANTHROPIC_BASE_URL={url} ANTHROPIC_API_KEY=\"$(security find-generic-password -s {svc} -w 2>/dev/null)\" command claude \"$@\" ;;\n"
            );
        } else {
            o += "      HOME=\"$_cch\" command claude \"$@\" ;;\n";
        }
    }
    o += "    *) command claude \"$@\" ;;\n  esac\n}\n";
    o
}

fn generate_ps1(list: &[Profile]) -> String {
    let mut o = String::new();
    o += "# 由 Claude Code 配置工具自动生成，请勿手动编辑。\n";
    o += "function claude {\n";
    o += "  param([Parameter(ValueFromRemainingArguments=$true)][string[]]$A)\n";
    o += "  $exe = (Get-Command claude.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source\n";
    o += "  if (-not $exe) { $exe = (Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source }\n";
    o += "  if (-not $exe) { Write-Host 'claude 未安装或不在 PATH'; return }\n";
    o += "  $sub = if ($A.Count -ge 1) { $A[0] } else { '' }\n";
    o += "  $rest = if ($A.Count -gt 1) { $A[1..($A.Count-1)] } else { @() }\n";
    o += "  switch ($sub) {\n";
    for p in list {
        let n = &p.name;
        if n.is_empty() {
            continue;
        }
        o += &format!("    {} {{\n", ps_q(n));
        o += &format!("      $h = Join-Path $env:USERPROFILE '.claude-split\\{n}'\n");
        o += "      New-Item -ItemType Directory -Force -Path (Join-Path $h '.local\\bin') | Out-Null\n";
        o += "      $o1=$env:USERPROFILE; $o2=$env:ANTHROPIC_BASE_URL; $o3=$env:ANTHROPIC_API_KEY\n";
        o += "      try {\n";
        o += "        $env:USERPROFILE=$h\n";
        if p.type_ == "router" {
            o += &format!("        $env:ANTHROPIC_BASE_URL={}\n", ps_q(&p.base_url));
            if let Some(enc) = &p.token_enc {
                if !enc.is_empty() {
                    o += &format!("        $sec=ConvertTo-SecureString {}\n", ps_q(enc));
                    o += "        $b=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec)\n";
                    o += "        $env:ANTHROPIC_API_KEY=[Runtime.InteropServices.Marshal]::PtrToStringBSTR($b)\n";
                    o += "        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($b)\n";
                }
            }
        }
        o += "        & $exe @rest\n";
        o += "      } finally {\n";
        o += "        $env:USERPROFILE=$o1\n";
        o += "        if($o2){$env:ANTHROPIC_BASE_URL=$o2}else{Remove-Item Env:\\ANTHROPIC_BASE_URL -ErrorAction SilentlyContinue}\n";
        o += "        if($o3){$env:ANTHROPIC_API_KEY=$o3}else{Remove-Item Env:\\ANTHROPIC_API_KEY -ErrorAction SilentlyContinue}\n";
        o += "      }\n";
        o += "    }\n";
    }
    o += "    default { & $exe @A }\n";
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

fn install_integration(list: &[Profile]) -> Result<String, String> {
    fs::create_dir_all(cfg_dir()).map_err(|e| e.to_string())?;
    if cfg!(target_os = "windows") {
        fs::write(ps_path(), generate_ps1(list)).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "$PROFILE"])
            .output()
            .map_err(|e| e.to_string())?;
        let profile_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if profile_path.is_empty() {
            return Ok("已生成 cc.ps1，但未能定位 PowerShell $PROFILE，请手动 dot-source 它。".into());
        }
        let line = format!(". \"{}\"  {}", ps_path().display(), MARK);
        ensure_line(&PathBuf::from(&profile_path), &line).map_err(|e| e.to_string())?;
        Ok("已接入 PowerShell $PROFILE。请新开一个 PowerShell 窗口生效。".into())
    } else {
        fs::write(sh_path(), generate_sh(list)).map_err(|e| e.to_string())?;
        let line = format!(
            "[ -f \"{p}\" ] && source \"{p}\"  {m}",
            p = sh_path().display(),
            m = MARK
        );
        let _ = ensure_line(&home().join(".zshrc"), &line);
        let _ = ensure_line(&home().join(".bashrc"), &line);
        Ok("已接入 shell 配置。请新开一个终端窗口（或 source 一下）生效。".into())
    }
}

// ---------------- token（平台原生） ----------------
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
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "ConvertTo-SecureString -String $args[0] -AsPlainText -Force | ConvertFrom-SecureString",
                token,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        let enc = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if enc.is_empty() {
            return Err("PowerShell 加密失败".into());
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
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["claude.exe", "claude.cmd", "claude.bat", "claude"]
    } else {
        &["claude"]
    };
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            for n in names {
                if dir.join(n).is_file() {
                    return true;
                }
            }
        }
    }
    false
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
    if p.type_ == "router" {
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
fn delete_profile(name: String) -> Result<(), String> {
    let mut list = load();
    list.retain(|x| x.name != name);
    clear_token(&name);
    save(&list).map_err(|e| e.to_string())?;
    install_integration(&list)?;
    Ok(())
}

#[tauri::command]
fn sync_links(name: String, skills: bool, plugins: bool) -> Result<String, String> {
    let real_claude = home().join(".claude");
    let dest_claude = home().join(".claude-split").join(&name).join(".claude");
    let mut subs: Vec<&str> = vec![];
    if skills {
        subs.push("skills");
    }
    if plugins {
        subs.push("plugins");
    }
    if subs.is_empty() {
        return Ok("请先打开要共享的 skills 或 plugins 开关。".into());
    }

    if cfg!(target_os = "windows") {
        let dest = dest_claude.display().to_string();
        let src = real_claude.display().to_string();
        let mut lines = vec![format!(
            "New-Item -ItemType Directory -Force -Path '{dest}' | Out-Null"
        )];
        for sub in &subs {
            lines.push(format!(
                "if (Test-Path '{src}\\{sub}') {{ New-Item -ItemType SymbolicLink -Force -Path '{dest}\\{sub}' -Target '{src}\\{sub}' | Out-Null }}"
            ));
        }
        lines.push("Write-Host '链接已创建。' -ForegroundColor Green; Start-Sleep 2".to_string());
        let inner = lines.join("; ");
        let elevate = format!(
            "Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-Command',{}",
            ps_q(&inner)
        );
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &elevate])
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok("已请求管理员权限创建符号链接（请在 UAC 中确认）。".into())
    } else {
        fs::create_dir_all(&dest_claude).map_err(|e| e.to_string())?;
        let mut out: Vec<String> = vec![];
        for sub in &subs {
            let src = real_claude.join(sub);
            let dst = dest_claude.join(sub);
            if !src.exists() {
                out.push(format!("{sub}（主账户无此目录，跳过）"));
                continue;
            }
            if let Ok(meta) = fs::symlink_metadata(&dst) {
                if meta.file_type().is_symlink() {
                    let _ = fs::remove_file(&dst);
                } else {
                    out.push(format!("{sub}（目标已存在且非链接，跳过）"));
                    continue;
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;
                match symlink(&src, &dst) {
                    Ok(_) => out.push(format!("{sub} ✓")),
                    Err(e) => out.push(format!("{sub} 失败：{e}")),
                }
            }
            #[cfg(not(unix))]
            {
                out.push(format!("{sub}（当前平台不支持）"));
            }
        }
        Ok(out.join(" | "))
    }
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
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            save_profile,
            delete_profile,
            sync_links,
            environment
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
