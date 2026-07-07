// ---------------- skill/plugin 市场：检索与安装 ----------------
// 全部通过 `claude plugin ...` 子进程完成，不自行解析市场仓库；
// 安装/卸载/启停一律固定 --scope user 并强制走主账户 ~/.claude
// （去掉继承到的 CLAUDE_CONFIG_DIR，装完由 sync 模块广播给各实例）。

use crate::{load, profile_names};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const NAMES_WINDOWS: &[&str] = &["claude.exe", "claude.cmd", "claude.bat", "claude"];
const NAMES_OTHER: &[&str] = &["claude"];

/// 定位 claude 可执行文件的完整路径；找不到返回 None。
/// 搜索顺序与旧 `claude_found()` 一致：先查继承到的 PATH，再兜底查常见安装目录。
pub fn resolve_claude_exe() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(target_os = "windows") {
        NAMES_WINDOWS
    } else {
        NAMES_OTHER
    };
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            for n in names {
                let p = dir.join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let h = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
        let mut v = vec![h.join(".local").join("bin")];
        if let Ok(appdata) = std::env::var("APPDATA") {
            v.push(PathBuf::from(appdata).join("npm"));
        }
        v
    } else {
        vec![h.join(".local").join("bin"), h.join(".npm-global").join("bin")]
    };
    for dir in candidates {
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn run_claude(args: &[&str]) -> Result<String, String> {
    let exe = resolve_claude_exe().ok_or("未找到 claude 可执行文件，请先安装 Claude Code")?;
    let mut cmd = Command::new(&exe);
    cmd.args(args);
    // 市场/插件管理固定作用于主账户，避免继承到某个实例的 CLAUDE_CONFIG_DIR
    cmd.env_remove("CLAUDE_CONFIG_DIR");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW，避免闪出黑框
    }
    let out = cmd
        .output()
        .map_err(|e| format!("调用 claude 失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if !stderr.is_empty() { stderr } else { stdout.trim().to_string() };
        return Err(if msg.is_empty() {
            format!("claude 命令执行失败（exit code {:?}）", out.status.code())
        } else {
            msg
        });
    }
    Ok(stdout)
}

/// 装/卸/启停会改变 ~/.claude/settings.json 里的 enabledPlugins，
/// 尽力把这次变化合并广播给各实例；失败只记日志，不影响本次操作已经成功的结果。
fn broadcast_after_mutate() {
    if let Err(e) = crate::sync::sync_configs(&profile_names(&load())) {
        crate::sync::log_line(&format!("plugin 变更后同步到各实例失败：{e}"));
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceEntry {
    name: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    install_location: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    install_path: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum PluginSource {
    Text(String),
    Object(PluginSourceObject),
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginSourceObject {
    #[serde(default)]
    source: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default, rename = "ref")]
    ref_: Option<String>,
    #[serde(default)]
    sha: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AvailablePlugin {
    plugin_id: String,
    name: String,
    #[serde(default)]
    description: String,
    marketplace_name: String,
    #[serde(default)]
    install_count: Option<u64>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    source: Option<PluginSource>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginListResult {
    installed: Vec<InstalledPlugin>,
    available: Vec<AvailablePlugin>,
}

#[tauri::command]
pub fn plugin_marketplace_list() -> Result<Vec<MarketplaceEntry>, String> {
    let out = run_claude(&["plugin", "marketplace", "list", "--json"])?;
    serde_json::from_str(&out).map_err(|e| format!("解析市场列表失败：{e}"))
}

#[tauri::command]
pub fn plugin_marketplace_add(source: String) -> Result<String, String> {
    let src = source.trim();
    if src.is_empty() {
        return Err("请填写市场地址（owner/repo 或 git 地址）".into());
    }
    let out = run_claude(&["plugin", "marketplace", "add", src])?;
    Ok(out.trim().to_string())
}

#[tauri::command]
pub fn plugin_marketplace_remove(name: String) -> Result<String, String> {
    let out = run_claude(&["plugin", "marketplace", "remove", name.as_str()])?;
    Ok(out.trim().to_string())
}

#[tauri::command]
pub fn plugin_list() -> Result<PluginListResult, String> {
    let out = run_claude(&["plugin", "list", "--json", "--available"])?;
    serde_json::from_str(&out).map_err(|e| format!("解析插件列表失败：{e}"))
}

#[tauri::command]
pub fn plugin_install(plugin_id: String) -> Result<String, String> {
    let out = run_claude(&["plugin", "install", plugin_id.as_str(), "--scope", "user"])?;
    broadcast_after_mutate();
    Ok(out.trim().to_string())
}

#[tauri::command]
pub fn plugin_uninstall(plugin_id: String) -> Result<String, String> {
    let out = run_claude(&["plugin", "uninstall", plugin_id.as_str(), "--scope", "user", "-y"])?;
    broadcast_after_mutate();
    Ok(out.trim().to_string())
}

#[tauri::command]
pub fn plugin_set_enabled(plugin_id: String, enabled: bool) -> Result<String, String> {
    let sub = if enabled { "enable" } else { "disable" };
    let out = run_claude(&["plugin", sub, plugin_id.as_str(), "--scope", "user"])?;
    broadcast_after_mutate();
    Ok(out.trim().to_string())
}

#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只支持打开 http/https 链接".into());
    }
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("打开链接失败：{e}"))
}
