use super::*;
use std::io::{Read, Seek, SeekFrom};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub path: String,
    pub executable: String,
    pub version: String,
    pub cli: Option<String>,
    pub compatible: bool,
    pub detail: String,
}

// ASAR is inspected as data, never executed. Bounds prevent malformed selected files
// from allocating arbitrary amounts of memory or reading outside the archive.
fn asar_entry(
    file: &mut fs::File,
    header: &serde_json::Value,
    base: u64,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let mut node = header;
    for part in name.split('/') {
        node = node
            .get("files")
            .and_then(|v| v.get(part))
            .ok_or("应用包中缺少必要文件")?;
    }
    if node.get("unpacked").and_then(|v| v.as_bool()) == Some(true) {
        return Err("不支持外置的应用包元数据".into());
    }
    let offset = node["offset"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or("无效的应用包位置")?;
    let size = node["size"]
        .as_u64()
        .filter(|s| *s <= limit)
        .ok_or("应用包文件过大")?;
    let begin = base.checked_add(offset).ok_or("应用包位置溢出")?;
    if begin.checked_add(size).ok_or("应用包大小溢出")?
        > file.metadata().map_err(|e| e.to_string())?.len()
    {
        return Err("应用包已损坏".into());
    }
    file.seek(SeekFrom::Start(begin))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn inspect_asar(path: &Path) -> Result<(String, bool), String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut sizes = [0u8; 16];
    file.read_exact(&mut sizes).map_err(|e| e.to_string())?;
    let header_size = u32::from_le_bytes(sizes[12..16].try_into().unwrap()) as u64;
    let base = 8 + u32::from_le_bytes(sizes[4..8].try_into().unwrap()) as u64;
    if header_size > 16 * 1024 * 1024 || base < 16 + header_size {
        return Err("应用包头无效".into());
    }
    let mut bytes = vec![0; header_size as usize];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "无法识别应用包")?;
    let package: serde_json::Value = serde_json::from_slice(&asar_entry(
        &mut file,
        &header,
        base,
        "package.json",
        1024 * 1024,
    )?)
    .map_err(|_| "无法读取客户端版本")?;
    if package["name"] != "openai-codex-electron" {
        return Err("请选择基于 Codex 的新版官方 ChatGPT 客户端".into());
    }
    let version = package["version"]
        .as_str()
        .ok_or("缺少客户端版本")?
        .to_owned();
    let build = header
        .pointer("/files/.vite/files/build/files")
        .and_then(|v| v.as_object())
        .ok_or("无法识别客户端启动布局")?;
    let mut found = false;
    for name in build
        .keys()
        .filter(|n| n.starts_with("bootstrap-") && n.ends_with(".js"))
        .take(8)
    {
        let bytes = asar_entry(
            &mut file,
            &header,
            base,
            &format!(".vite/build/{name}"),
            16 * 1024 * 1024,
        )?;
        let code = String::from_utf8_lossy(&bytes);
        found |= code.contains("CODEX_ELECTRON_USER_DATA_PATH") && code.contains("setPath");
    }
    Ok((version, found))
}

pub fn inspect(path: &Path) -> Result<Installation, String> {
    let path = fs::canonicalize(path).map_err(|_| "应用路径不存在，请重新选择")?;
    #[cfg(target_os = "macos")]
    let (executable, resources) = {
        let value = plist::Value::from_file(path.join("Contents/Info.plist"))
            .map_err(|_| "请选择 ChatGPT.app 应用包")?;
        let dict = value.as_dictionary().ok_or("应用信息无效")?;
        if dict.get("CFBundleIdentifier").and_then(|v| v.as_string()) != Some("com.openai.codex") {
            return Err("请选择基于 Codex 的新版官方 ChatGPT 客户端".into());
        }
        let name = dict
            .get("CFBundleExecutable")
            .and_then(|v| v.as_string())
            .ok_or("应用中没有主程序")?;
        if name.contains(['/', '\\']) {
            return Err("应用主程序名称无效".into());
        }
        (
            path.join("Contents/MacOS").join(name),
            path.join("Contents/Resources"),
        )
    };
    #[cfg(windows)]
    let (executable, resources) = {
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some("exe")
        {
            return Err("请选择 ChatGPT 的应用程序文件".into());
        }
        let resources = path.parent().ok_or("无效的程序路径")?.join("resources");
        (path.clone(), resources)
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let (executable, resources) = (
        path.clone(),
        path.parent().unwrap_or(&path).join("resources"),
    );
    if !executable.is_file() {
        return Err("客户端主程序不存在".into());
    }
    let (version, compatible) = inspect_asar(&resources.join("app.asar"))?;
    let cli = cli_candidates(&resources, cfg!(windows))
        .into_iter()
        .find(|p| p.is_file());
    Ok(Installation {
        path: path.to_string_lossy().into(),
        executable: executable.to_string_lossy().into(),
        version,
        cli: cli.map(|p| p.to_string_lossy().into()),
        compatible,
        detail: if compatible {
            "已识别独立目录启动能力；双账号与当前版本的完整功能仍需实机验收。"
        } else {
            "未识别独立目录启动能力，已阻止多开。请更新客户端后重新检测。"
        }
        .into(),
    })
}

pub fn cli_candidates(resources: &Path, windows: bool) -> Vec<PathBuf> {
    if windows {
        [
            "codex.exe",
            "codex-cli/bin/codex.exe",
            "codex-cli/codex.exe",
        ]
        .map(|p| resources.join(p))
        .into()
    } else {
        [
            "codex-cli/CodexCLI.app/Contents/MacOS/codex",
            "codex",
            "codex-cli/bin/codex",
        ]
        .map(|p| resources.join(p))
        .into()
    }
}

pub fn discover(saved: Option<&str>) -> Result<Installation, String> {
    if let Some(p) = saved {
        if let Ok(app) = inspect(Path::new(p)) {
            return Ok(app);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        candidates
            .extend(["/Applications/ChatGPT.app", "/Applications/Codex.app"].map(PathBuf::from));
        if let Some(home) = dirs::home_dir() {
            candidates.extend([
                home.join("Applications/ChatGPT.app"),
                home.join("Applications/Codex.app"),
            ]);
        }
    }
    #[cfg(windows)]
    {
        for key in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(key) {
                for rel in [
                    "Programs/ChatGPT/ChatGPT.exe",
                    "Programs/Codex/Codex.exe",
                    "ChatGPT/ChatGPT.exe",
                    "Codex/Codex.exe",
                ] {
                    candidates.push(PathBuf::from(&base).join(rel));
                }
            }
        }
        // Package paths change after every Store update. Discover them again instead
        // of relying on a saved WindowsApps version directory.
        let script = "$ErrorActionPreference='Stop'; @(Get-AppxPackage | Where-Object { $_.Name -match 'OpenAI|ChatGPT|Codex' } | ForEach-Object { $root=$_.InstallLocation; $m=Get-AppxPackageManifest -Package $_; foreach($a in $m.Package.Applications.Application) { if($a.Executable) { Join-Path $root $a.Executable } } }) | ConvertTo-Json -Compress";
        if let Ok(raw) = process::output(Command::new("powershell.exe").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(s) = value.as_str() {
                    candidates.push(s.into());
                }
                if let Some(list) = value.as_array() {
                    candidates.extend(list.iter().filter_map(|v| v.as_str()).map(PathBuf::from));
                }
            }
        }
    }
    for candidate in candidates {
        if let Ok(app) = inspect(&candidate) {
            return Ok(app);
        }
    }
    Err("未找到支持多开的 ChatGPT 客户端，请手动选择安装位置。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_layouts_cover_both_platforms() {
        let base = Path::new("resources");
        assert!(cli_candidates(base, true)
            .iter()
            .all(|p| p.extension().unwrap() == "exe"));
        assert!(cli_candidates(base, false)
            .iter()
            .any(|p| p.to_string_lossy().contains("CodexCLI.app")));
    }
    #[test]
    fn rejects_corrupt_archive_without_allocating() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), [255u8; 16]).unwrap();
        assert!(inspect_asar(tmp.path()).is_err());
    }
}
