use serde::Serialize;
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const NAMES_WINDOWS: &[&str] = &["claude.exe", "claude.cmd", "claude.bat", "claude"];
const NAMES_OTHER: &[&str] = &["claude"];
const PATH_MARKER_BEGIN: &str = "__CCM_PATH_BEGIN__";
const PATH_MARKER_END: &str = "__CCM_PATH_END__";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_CHECKED_PATHS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    Macos,
    Other,
}

impl Platform {
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectionStatus {
    Ready,
    NotFound,
    Unusable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectionSource {
    Cache,
    ProcessPath,
    LoginShell,
    PackageManager,
    Fallback,
    Manual,
}

impl DetectionSource {
    fn label(self) -> &'static str {
        match self {
            Self::Cache => "上次成功记录",
            Self::ProcessPath => "应用环境 PATH",
            Self::LoginShell => "登录终端 PATH",
            Self::PackageManager => "安装器",
            Self::Fallback => "常见安装位置",
            Self::Manual => "手动选择",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDetection {
    pub found: bool,
    pub status: DetectionStatus,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub source: Option<DetectionSource>,
    pub detail: String,
    pub checked_paths: Vec<PathBuf>,
    pub shell_warning: Option<String>,
}

#[derive(Clone, Debug)]
struct Candidate {
    path: PathBuf,
    source: DetectionSource,
}

impl Candidate {
    fn new(path: PathBuf, source: DetectionSource) -> Self {
        Self { path, source }
    }
}

#[derive(Clone, Debug)]
struct SelectionResult {
    found: bool,
    status: DetectionStatus,
    path: Option<PathBuf>,
    version: Option<String>,
    source: Option<DetectionSource>,
    detail: String,
    checked_paths: Vec<PathBuf>,
}

fn executable_names(platform: Platform) -> &'static [&'static str] {
    if platform == Platform::Windows {
        NAMES_WINDOWS
    } else {
        NAMES_OTHER
    }
}

fn split_path_value(value: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(value)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

fn find_executable_in_path(
    value: Option<&OsStr>,
    names: &[&str],
    checked: &mut Vec<PathBuf>,
) -> Option<PathBuf> {
    for dir in value.map(split_path_value).unwrap_or_default() {
        for name in names {
            let candidate = dir.join(name);
            push_checked(checked, candidate.clone());
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn push_checked(checked: &mut Vec<PathBuf>, path: PathBuf) {
    if checked.len() < MAX_CHECKED_PATHS && !checked.contains(&path) {
        checked.push(path);
    }
}

fn push_directory_candidates(
    target: &mut Vec<Candidate>,
    directories: impl IntoIterator<Item = PathBuf>,
    names: &[&str],
    source: DetectionSource,
) {
    for dir in directories {
        for name in names {
            target.push(Candidate::new(dir.join(name), source));
        }
    }
}

fn fallback_directories(platform: Platform, home: &Path, appdata: Option<&OsStr>) -> Vec<PathBuf> {
    match platform {
        Platform::Windows => {
            let mut paths = vec![home.join(".local").join("bin")];
            if let Some(appdata) = appdata {
                paths.push(PathBuf::from(appdata).join("npm"));
            }
            paths
        }
        Platform::Macos => vec![
            home.join(".local").join("bin"),
            home.join(".npm-global").join("bin"),
            home.join(".claude").join("local").join("bin"),
            home.join(".claude")
                .join("local")
                .join("node_modules")
                .join(".bin"),
            home.join(".volta").join("bin"),
            home.join(".asdf").join("shims"),
            home.join(".local").join("share").join("mise").join("shims"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ],
        Platform::Other => vec![
            home.join(".local").join("bin"),
            home.join(".npm-global").join("bin"),
            home.join(".claude").join("local").join("bin"),
            home.join(".volta").join("bin"),
            home.join(".asdf").join("shims"),
            home.join(".local").join("share").join("mise").join("shims"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ],
    }
}

fn cache_path(home: &Path) -> PathBuf {
    home.join(".cc-manager").join("claude-cli-path")
}

fn read_cached_path(home: &Path) -> Option<PathBuf> {
    let value = fs::read_to_string(cache_path(home)).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn write_cached_path(home: &Path, executable: &Path) -> Result<(), String> {
    let file = cache_path(home);
    let parent = file.parent().ok_or("Claude 路径缓存目录无效")?;
    fs::create_dir_all(parent).map_err(|error| format!("创建 Claude 路径缓存失败：{error}"))?;
    fs::write(&file, executable.to_string_lossy().as_bytes())
        .map_err(|error| format!("保存 Claude 路径缓存失败：{error}"))
}

fn clear_stale_cache(home: &Path) {
    let file = cache_path(home);
    if file.is_file() {
        let _ = fs::remove_file(file);
    }
}

fn command_for_executable(executable: &Path, args: &[&str]) -> Command {
    if cfg!(target_os = "windows")
        && executable
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.arg("/D").arg("/C").arg(executable);
        command.args(args);
        command
    } else {
        let mut command = Command::new(executable);
        command.args(args);
        command
    }
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动：{error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("读取执行结果失败：{error}"));
            }
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("执行超过 {} 秒", timeout.as_secs()));
            }
            Err(error) => return Err(format!("等待执行结果失败：{error}")),
        }
    }
}

fn output_text(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        stdout
    } else {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    }
}

fn first_line(value: &str) -> String {
    value
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect()
}

fn verify_claude(executable: &Path) -> Result<String, String> {
    if !executable.is_file() {
        return Err("文件不存在".to_string());
    }
    let file_name = executable
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or("文件名不是有效文本")?;
    if !executable_names(Platform::current())
        .iter()
        .any(|name| name.eq_ignore_ascii_case(file_name))
    {
        return Err("所选文件不是 Claude 可执行文件".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = executable
            .metadata()
            .map_err(|error| format!("无法读取文件信息：{error}"))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err("文件没有执行权限".to_string());
        }
    }
    let output = run_with_timeout(
        command_for_executable(executable, &["--version"]),
        COMMAND_TIMEOUT,
    )?;
    let text = first_line(&output_text(&output));
    if !output.status.success() {
        return Err(if text.is_empty() {
            format!("--version 返回状态 {}", output.status)
        } else {
            format!("--version 失败：{text}")
        });
    }
    if text.is_empty() {
        return Err("--version 没有返回版本信息".to_string());
    }
    Ok(text)
}

fn select_working_candidate<F>(candidates: Vec<Candidate>, mut verify: F) -> SelectionResult
where
    F: FnMut(&Path) -> Result<String, String>,
{
    let mut seen = HashSet::new();
    let mut checked_paths = Vec::new();
    let mut failures = Vec::new();
    let mut saw_existing_file = false;

    for candidate in candidates {
        if !seen.insert(candidate.path.clone()) {
            continue;
        }
        push_checked(&mut checked_paths, candidate.path.clone());
        saw_existing_file |= candidate.path.is_file();
        match verify(&candidate.path) {
            Ok(version) => {
                return SelectionResult {
                    found: true,
                    status: DetectionStatus::Ready,
                    path: Some(candidate.path),
                    version: Some(version.clone()),
                    source: Some(candidate.source),
                    detail: format!(
                        "已通过{}检测到 Claude Code（{}）。",
                        candidate.source.label(),
                        version
                    ),
                    checked_paths,
                };
            }
            Err(error) if candidate.path.is_file() => {
                failures.push(format!("{}：{}", candidate.path.display(), error));
            }
            Err(_) => {}
        }
    }

    let (status, detail) = if saw_existing_file {
        let suffix = failures.into_iter().take(3).collect::<Vec<_>>().join("；");
        (
            DetectionStatus::Unusable,
            format!("找到了 Claude 程序，但无法执行版本检查。{suffix}"),
        )
    } else {
        (
            DetectionStatus::NotFound,
            "已检查应用环境、登录终端和常见安装位置，但没有找到可执行的 Claude Code。".to_string(),
        )
    };
    SelectionResult {
        found: false,
        status,
        path: None,
        version: None,
        source: None,
        detail,
        checked_paths,
    }
}

fn login_shell_path(platform: Platform) -> Result<OsString, String> {
    if platform == Platform::Windows {
        return Err("Windows 不使用登录 Shell 探测".to_string());
    }
    let default_shell = if platform == Platform::Macos {
        PathBuf::from("/bin/zsh")
    } else {
        PathBuf::from("/bin/sh")
    };
    let configured = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or(default_shell);
    if !configured.is_file() {
        return Err(format!("登录 Shell 不存在：{}", configured.display()));
    }
    let shell_name = configured
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let script = if shell_name == "fish" {
        format!(
            "printf '\\n{}%s{}\\n' (string join : $PATH)",
            PATH_MARKER_BEGIN, PATH_MARKER_END
        )
    } else {
        format!(
            "printf '\\n{}%s{}\\n' \"$PATH\"",
            PATH_MARKER_BEGIN, PATH_MARKER_END
        )
    };
    let mut command = Command::new(&configured);
    command.arg("-lic").arg(script);
    let output = run_with_timeout(command, COMMAND_TIMEOUT)?;
    let text = String::from_utf8_lossy(&output.stdout);
    let start = text
        .rfind(PATH_MARKER_BEGIN)
        .map(|index| index + PATH_MARKER_BEGIN.len())
        .ok_or_else(|| {
            let error = first_line(&String::from_utf8_lossy(&output.stderr));
            if error.is_empty() {
                format!("{} 没有返回 PATH", configured.display())
            } else {
                format!("{} 没有返回 PATH：{error}", configured.display())
            }
        })?;
    let remaining = &text[start..];
    let end = remaining
        .find(PATH_MARKER_END)
        .ok_or_else(|| format!("{} 返回的 PATH 格式不完整", configured.display()))?;
    let value = remaining[..end].trim();
    if value.is_empty() {
        Err(format!("{} 返回了空 PATH", configured.display()))
    } else {
        Ok(OsString::from(value))
    }
}

fn find_named_executable(
    path_values: &[OsString],
    names: &[&str],
    fixed: &[PathBuf],
) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for value in path_values {
        let mut checked = Vec::new();
        if let Some(path) = find_executable_in_path(Some(value), names, &mut checked) {
            found.push(path);
        }
    }
    found.extend(fixed.iter().filter(|path| path.is_file()).cloned());
    let mut seen = HashSet::new();
    found.retain(|path| seen.insert(path.clone()));
    found
}

fn package_manager_directories(platform: Platform, path_values: &[OsString]) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if platform == Platform::Macos {
        let brew_candidates = find_named_executable(
            path_values,
            &["brew"],
            &[
                PathBuf::from("/opt/homebrew/bin/brew"),
                PathBuf::from("/usr/local/bin/brew"),
            ],
        );
        for brew in brew_candidates {
            if let Ok(output) = run_with_timeout(
                command_for_executable(&brew, &["--prefix"]),
                COMMAND_TIMEOUT,
            ) {
                if output.status.success() {
                    let prefix = first_line(&output_text(&output));
                    if !prefix.is_empty() {
                        directories.push(PathBuf::from(prefix).join("bin"));
                    }
                }
            }
        }
    }

    let npm_names: &[&str] = if platform == Platform::Windows {
        &["npm.cmd", "npm.exe", "npm"]
    } else {
        &["npm"]
    };
    for npm in find_named_executable(path_values, npm_names, &[]) {
        if let Ok(output) = run_with_timeout(
            command_for_executable(&npm, &["prefix", "-g"]),
            COMMAND_TIMEOUT,
        ) {
            if output.status.success() {
                let prefix = first_line(&output_text(&output));
                if !prefix.is_empty() {
                    let path = PathBuf::from(prefix);
                    directories.push(if platform == Platform::Windows {
                        path
                    } else {
                        path.join("bin")
                    });
                }
            }
        }
    }

    let mut seen = HashSet::new();
    directories.retain(|path| seen.insert(path.clone()));
    directories
}

fn candidates_from_directories(
    directories: impl IntoIterator<Item = PathBuf>,
    names: &[&str],
    source: DetectionSource,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    push_directory_candidates(&mut candidates, directories, names, source);
    candidates
}

fn finish_detection(
    home: &Path,
    result: SelectionResult,
    shell_warning: Option<String>,
) -> ClaudeDetection {
    if let Some(path) = result.path.as_deref() {
        let _ = write_cached_path(home, path);
    } else {
        clear_stale_cache(home);
    }
    ClaudeDetection {
        found: result.found,
        status: result.status,
        path: result.path,
        version: result.version,
        source: result.source,
        detail: result.detail,
        checked_paths: result.checked_paths,
        shell_warning,
    }
}

fn merge_missed_stages(stages: Vec<SelectionResult>) -> SelectionResult {
    let mut checked_paths = Vec::new();
    let mut unusable_details = Vec::new();
    for stage in stages {
        for path in stage.checked_paths {
            push_checked(&mut checked_paths, path);
        }
        if stage.status == DetectionStatus::Unusable {
            unusable_details.push(stage.detail);
        }
    }
    let (status, detail) = if unusable_details.is_empty() {
        (
            DetectionStatus::NotFound,
            "已检查应用环境、登录终端、安装器和常见安装位置，但没有找到可执行的 Claude Code。"
                .to_string(),
        )
    } else {
        (DetectionStatus::Unusable, unusable_details.join("；"))
    };
    SelectionResult {
        found: false,
        status,
        path: None,
        version: None,
        source: None,
        detail,
        checked_paths,
    }
}

pub fn detect_claude() -> ClaudeDetection {
    let platform = Platform::current();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let names = executable_names(platform);
    let mut missed_stages = Vec::new();
    let mut shell_warning = None;
    let mut path_values = Vec::new();

    if let Some(cached) = read_cached_path(&home) {
        let result = select_working_candidate(
            vec![Candidate::new(cached, DetectionSource::Cache)],
            verify_claude,
        );
        if result.found {
            return finish_detection(&home, result, None);
        }
        missed_stages.push(result);
    }

    if let Some(process_path) = std::env::var_os("PATH") {
        let result = select_working_candidate(
            candidates_from_directories(
                split_path_value(&process_path),
                names,
                DetectionSource::ProcessPath,
            ),
            verify_claude,
        );
        path_values.push(process_path);
        if result.found {
            return finish_detection(&home, result, None);
        }
        missed_stages.push(result);
    }

    if platform != Platform::Windows {
        match login_shell_path(platform) {
            Ok(shell_path) => {
                let result = select_working_candidate(
                    candidates_from_directories(
                        split_path_value(&shell_path),
                        names,
                        DetectionSource::LoginShell,
                    ),
                    verify_claude,
                );
                path_values.push(shell_path);
                if result.found {
                    return finish_detection(&home, result, None);
                }
                missed_stages.push(result);
            }
            Err(error) => shell_warning = Some(error),
        }
    }

    let result = select_working_candidate(
        candidates_from_directories(
            fallback_directories(platform, &home, std::env::var_os("APPDATA").as_deref()),
            names,
            DetectionSource::Fallback,
        ),
        verify_claude,
    );
    if result.found {
        return finish_detection(&home, result, shell_warning);
    }
    missed_stages.push(result);

    let result = select_working_candidate(
        candidates_from_directories(
            package_manager_directories(platform, &path_values),
            names,
            DetectionSource::PackageManager,
        ),
        verify_claude,
    );
    if result.found {
        return finish_detection(&home, result, shell_warning);
    }
    missed_stages.push(result);

    finish_detection(&home, merge_missed_stages(missed_stages), shell_warning)
}

pub fn remember_manual_path(path: PathBuf) -> Result<ClaudeDetection, String> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let result = select_working_candidate(
        vec![Candidate::new(path, DetectionSource::Manual)],
        verify_claude,
    );
    let executable = result
        .path
        .as_deref()
        .ok_or_else(|| result.detail.clone())?;
    write_cached_path(&home, executable)?;
    Ok(ClaudeDetection {
        found: true,
        status: result.status,
        path: result.path,
        version: result.version,
        source: result.source,
        detail: result.detail,
        checked_paths: result.checked_paths,
        shell_warning: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn discovers_claude_from_login_shell_path_without_resolving_shell_functions() {
        let root = std::env::temp_dir().join(format!(
            "cc-manager-claude-shell-path-{}",
            std::process::id()
        ));
        let bin = root.join("future-package-manager").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let executable = bin.join("claude");
        std::fs::write(&executable, b"fixture").unwrap();

        let shell_path = std::env::join_paths([bin]).unwrap();
        let mut checked = Vec::new();
        let found =
            find_executable_in_path(Some(shell_path.as_os_str()), &["claude"], &mut checked);

        assert_eq!(found, Some(executable));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn macos_fallbacks_cover_homebrew_and_native_installers() {
        let home = PathBuf::from("/Users/example");
        let paths = fallback_directories(Platform::Macos, &home, None);

        assert!(paths.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(paths.contains(&PathBuf::from("/usr/local/bin")));
        assert!(paths.contains(&home.join(".local").join("bin")));
        assert!(paths.contains(&home.join(".claude").join("local").join("bin")));
    }

    #[test]
    fn stale_cached_path_falls_through_to_a_new_dynamic_location() {
        let stale = PathBuf::from("/old/location/claude");
        let current = PathBuf::from("/new/location/claude");
        let candidates = vec![
            Candidate::new(stale.clone(), DetectionSource::Cache),
            Candidate::new(current.clone(), DetectionSource::LoginShell),
        ];

        let result = select_working_candidate(candidates, |path| {
            if path == current {
                Ok("2.1.176 (Claude Code)".to_string())
            } else {
                Err("文件不存在".to_string())
            }
        });

        assert_eq!(result.path, Some(current));
        assert_eq!(result.source, Some(DetectionSource::LoginShell));
        assert_eq!(result.version.as_deref(), Some("2.1.176 (Claude Code)"));
        assert!(result.checked_paths.iter().any(|path| path == &stale));
    }

    #[test]
    fn empty_path_is_reported_as_not_found_with_checked_locations() {
        let candidates = vec![Candidate::new(
            PathBuf::from("/opt/homebrew/bin/claude"),
            DetectionSource::Fallback,
        )];

        let result = select_working_candidate(candidates, |_| Err("文件不存在".to_string()));

        assert_eq!(result.status, DetectionStatus::NotFound);
        assert!(!result.found);
        assert_eq!(result.checked_paths.len(), 1);
        assert!(!result.detail.is_empty());
    }

    #[test]
    fn path_list_parsing_does_not_depend_on_command_v_output() {
        let value = std::env::join_paths([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/bin"),
        ])
        .unwrap_or_else(|_| OsString::from("/opt/homebrew/bin:/usr/bin"));
        let paths = split_path_value(value.as_os_str());

        assert!(!paths.is_empty());
        assert!(paths.iter().any(|path| path.ends_with("bin")));
    }

    #[test]
    fn verifies_version_through_the_resolved_absolute_path() {
        let root =
            std::env::temp_dir().join(format!("cc manager claude version {}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();

        #[cfg(target_os = "windows")]
        let executable = {
            let path = root.join("claude.cmd");
            std::fs::write(
                &path,
                b"@echo off\r\nif \"%1\"==\"--version\" echo 9.9.9 (Claude Code)\r\n",
            )
            .unwrap();
            path
        };
        #[cfg(not(target_os = "windows"))]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("claude");
            std::fs::write(
                &path,
                b"#!/bin/sh\n[ \"$1\" = \"--version\" ] && echo '9.9.9 (Claude Code)'\n",
            )
            .unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
            path
        };

        assert_eq!(verify_claude(&executable).unwrap(), "9.9.9 (Claude Code)");
        std::fs::remove_dir_all(root).unwrap();
    }
}
