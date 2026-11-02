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
    // 客户端更换打包器后，bootstrap-* 已改为 early-bootstrap-*，且能力实现
    // 可能位于任意启动分块。按实际构建目录有界检查内容，不依赖文件名前缀。
    let scripts: Vec<_> = build.keys().filter(|name| name.ends_with(".js")).collect();
    if scripts.len() > 128 {
        return Err("客户端启动分块过多，无法确认独立目录能力，请更新 PathMux 后重新检测".into());
    }
    let mut inspected_bytes = 0u64;
    for name in scripts {
        let size = build[name]["size"].as_u64().ok_or("应用包文件大小无效")?;
        inspected_bytes = inspected_bytes
            .checked_add(size)
            .filter(|total| *total <= 64 * 1024 * 1024)
            .ok_or("客户端启动脚本过大，无法确认独立目录能力，请更新 PathMux 后重新检测")?;
        let bytes = asar_entry(
            &mut file,
            &header,
            base,
            &format!(".vite/build/{name}"),
            16 * 1024 * 1024,
        )?;
        let code = String::from_utf8_lossy(&bytes);
        if code.contains("CODEX_ELECTRON_USER_DATA_PATH") && code.contains("setPath") {
            return Ok((version, true));
        }
    }
    Ok((version, false))
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
            "未能确认当前客户端的独立目录能力，已暂停多开。请刷新或重新选择客户端；若客户端刚更新，请检查 PathMux 更新。"
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

    fn archive_fixture(scripts: &[(&str, &str)]) -> tempfile::NamedTempFile {
        let mut header = serde_json::json!({ "files": {} });
        let mut data = Vec::new();
        let package = serde_json::json!({
            "name": "openai-codex-electron", "version": "26.1027.30824",
            "main": ".vite/build/early-bootstrap.js"
        })
        .to_string();
        for (name, content) in
            std::iter::once(("package.json", package.as_str())).chain(scripts.iter().copied())
        {
            let mut node = &mut header;
            let parts: Vec<_> = name.split('/').collect();
            for part in &parts[..parts.len() - 1] {
                let files = node["files"].as_object_mut().unwrap();
                node = files
                    .entry(*part)
                    .or_insert_with(|| serde_json::json!({ "files": {} }));
            }
            node["files"][parts[parts.len() - 1]] = serde_json::json!({
                "size": content.len(), "offset": data.len().to_string()
            });
            data.extend_from_slice(content.as_bytes());
        }
        let json = header.to_string();
        let header_payload = (4 + json.len()).next_multiple_of(4);
        let mut bytes = Vec::new();
        for size in [
            4,
            (4 + header_payload) as u32,
            header_payload as u32,
            json.len() as u32,
        ] {
            bytes.extend_from_slice(&size.to_le_bytes());
        }
        bytes.extend_from_slice(json.as_bytes());
        bytes.resize(12 + header_payload, 0);
        bytes.extend_from_slice(&data);
        let archive = tempfile::NamedTempFile::new().unwrap();
        fs::write(archive.path(), bytes).unwrap();
        archive
    }

    const ISOLATED_DATA_PATH: &str =
        "app.setPath('userData', process.env.CODEX_ELECTRON_USER_DATA_PATH);";

    #[test]
    fn detects_isolated_data_path_in_current_client_bundles() {
        // 同一解析器供 Windows resources 与 macOS Contents/Resources 使用。
        for name in [
            ".vite/build/early-bootstrap-339-096ec1020e8592e0.js",
            ".vite/build/early-bootstrap.js",
        ] {
            let archive = archive_fixture(&[(name, ISOLATED_DATA_PATH)]);
            let (version, compatible) = inspect_asar(archive.path()).unwrap();
            assert_eq!(version, "26.1027.30824");
            assert!(compatible, "不能因启动分块名称变化禁用多开：{name}");
        }
    }

    #[test]
    fn checks_bootstrap_chunks_after_the_first_eight() {
        let mut scripts: Vec<_> = (0..12)
            .map(|i| {
                (
                    format!(".vite/build/early-bootstrap-{i:02}.js"),
                    "other startup code",
                )
            })
            .collect();
        scripts[11].1 = ISOLATED_DATA_PATH;
        let entries: Vec<_> = scripts
            .iter()
            .map(|(name, code)| (name.as_str(), *code))
            .collect();
        let archive = archive_fixture(&entries);
        assert!(inspect_asar(archive.path()).unwrap().1);
    }

    #[test]
    fn unsupported_archives_do_not_gain_isolation_from_unrelated_content() {
        for entries in [
            vec![(
                ".vite/build/start.js",
                "app.setPath('userData', defaultPath);",
            )],
            vec![(
                ".vite/build/start.js",
                "process.env.CODEX_ELECTRON_USER_DATA_PATH",
            )],
            vec![
                ("docs/example.js", ISOLATED_DATA_PATH),
                (".vite/build/start.js", "other code"),
            ],
            vec![
                (".vite/build/one.js", "setPath"),
                (".vite/build/two.js", "CODEX_ELECTRON_USER_DATA_PATH"),
            ],
        ] {
            let archive = archive_fixture(&entries);
            assert!(!inspect_asar(archive.path()).unwrap().1);
        }
    }

    #[test]
    fn archive_probe_rejects_excessive_script_counts() {
        let scripts: Vec<_> = (0..129)
            .map(|i| (format!(".vite/build/chunk-{i}.js"), "other code"))
            .collect();
        let entries: Vec<_> = scripts
            .iter()
            .map(|(name, code)| (name.as_str(), *code))
            .collect();
        let archive = archive_fixture(&entries);
        assert!(inspect_asar(archive.path())
            .unwrap_err()
            .contains("分块过多"));
    }

    #[test]
    #[ignore = "read-only installed client check; set PATHMUX_TEST_CHATGPT_EXE"]
    fn installed_client_still_supports_isolated_data_paths() {
        let path = std::env::var_os("PATHMUX_TEST_CHATGPT_EXE").expect("set installed client path");
        let app = inspect(Path::new(&path)).unwrap();
        assert!(app.compatible, "{}: {}", app.version, app.detail);
        println!(
            "Installed client {} supports isolated data directories",
            app.version
        );
    }

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
