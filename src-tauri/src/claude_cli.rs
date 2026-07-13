use std::path::PathBuf;

const NAMES_WINDOWS: &[&str] = &["claude.exe", "claude.cmd", "claude.bat", "claude"];
const NAMES_OTHER: &[&str] = &["claude"];

/// 定位 Claude Code CLI。优先使用 PATH，再检查常见的用户级安装目录。
pub fn resolve_claude_exe() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(target_os = "windows") {
        NAMES_WINDOWS
    } else {
        NAMES_OTHER
    };
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
        let mut dirs = vec![home.join(".local").join("bin")];
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        dirs
    } else {
        vec![
            home.join(".local").join("bin"),
            home.join(".npm-global").join("bin"),
        ]
    };
    for dir in candidates {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
