use super::*;
use std::ffi::OsString;
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

pub fn output(command: &mut Command) -> Result<String, String> {
    let mut child = quiet(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().ok_or("无法读取程序输出")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = out
            .by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes);
        let _ = tx.send(result.map(|_| bytes));
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("系统操作超时或无法检查进程状态，请重试".into());
            }
        }
    };
    if !status.success() {
        return Err("系统操作未成功，请检查应用路径与当前用户权限".into());
    }
    let bytes = rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "程序未完成输出")?
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("程序输出超出限制".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into())
}

pub fn contaminated(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    k.starts_with("CODEX_")
        || k.starts_with("OPENAI_")
        || k.starts_with("CHATGPT_")
        || k.starts_with("ELECTRON_")
        || k.starts_with("NODE_")
}

pub fn configure(command: &mut Command, dir: &Path) {
    for (key, _) in std::env::vars_os() {
        if contaminated(&key.to_string_lossy()) {
            command.env_remove(key);
        }
    }
    command
        .env("CODEX_HOME", dir.join("codex"))
        .env("CODEX_ELECTRON_USER_DATA_PATH", dir.join("desktop"))
        .env("CODEX_SQLITE_HOME", dir.join("codex/db"))
        .current_dir(dir);
}

#[derive(Clone, Debug)]
pub struct Live {
    pub pid: u32,
    pub started: u64,
    pub exe: PathBuf,
    pub args: Vec<OsString>,
    pub parent: Option<u32>,
}

pub fn snapshot() -> Result<Vec<Live>, String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    if !system
        .processes()
        .contains_key(&sysinfo::Pid::from_u32(std::process::id()))
    {
        return Err("无法可靠读取进程状态，已暂停实例操作".into());
    }
    Ok(system
        .processes()
        .iter()
        .filter_map(|(pid, p)| {
            Some(Live {
                pid: pid.as_u32(),
                started: p.start_time(),
                exe: p.exe().map(Path::to_path_buf).unwrap_or_default(),
                args: p.cmd().to_vec(),
                parent: p.parent().map(|p| p.as_u32()),
            })
        })
        .collect())
}

pub fn path_eq(a: &Path, b: &Path, windows: bool) -> bool {
    let a = a.canonicalize().unwrap_or_else(|_| a.into());
    let b = b.canonicalize().unwrap_or_else(|_| b.into());
    if windows {
        a.to_string_lossy()
            .replace('/', "\\")
            .eq_ignore_ascii_case(&b.to_string_lossy().replace('/', "\\"))
    } else {
        a == b
    }
}

pub fn argument_path(args: &[OsString], key: &str, path: &Path, windows: bool) -> bool {
    args.iter().enumerate().any(|(i, arg)| {
        let s = arg.to_string_lossy();
        let value = s
            .strip_prefix(&format!("{key}="))
            .map(OsString::from)
            .or_else(|| {
                if s == key {
                    args.get(i + 1).cloned()
                } else {
                    None
                }
            });
        value.is_some_and(|v| path_eq(Path::new(&v), path, windows))
    })
}

pub fn owned<'a>(all: &'a [Live], dir: &Path) -> Vec<&'a Live> {
    let desktop = dir.join("desktop");
    let mut ids: std::collections::HashSet<u32> = all
        .iter()
        .filter(|p| {
            argument_path(&p.args, "--user-data-dir", &desktop, cfg!(windows))
                || p.args.windows(2).any(|pair| {
                    pair[0] == "--config"
                        && pair[1]
                            == format!(
                                "sqlite_home={}",
                                serde_json::to_string(&dir.join("codex/db").to_string_lossy())
                                    .unwrap()
                            )
                            .as_str()
                })
                || argument_path(
                    &p.args,
                    "--database",
                    &desktop.join("Crashpad"),
                    cfg!(windows),
                )
        })
        .map(|p| p.pid)
        .collect();
    loop {
        let n = ids.len();
        for p in all {
            if p.parent.is_some_and(|id| ids.contains(&id)) {
                ids.insert(p.pid);
            }
        }
        if n == ids.len() {
            break;
        }
    }
    all.iter().filter(|p| ids.contains(&p.pid)).collect()
}

pub fn main_process<'a>(all: &'a [Live], dir: &Path, exe: &Path) -> Option<&'a Live> {
    owned(all, dir).into_iter().find(|p| {
        path_eq(&p.exe, exe, cfg!(windows))
            && !p
                .args
                .iter()
                .any(|a| a.to_string_lossy().starts_with("--type="))
    })
}

/// Electron crash reporters can outlive a closed app. Only retire reporters when
/// every remaining owned process is a reporter; never terminate a live workload.
fn is_reporter(exe: &Path) -> bool {
    exe.file_name().is_some_and(|n| {
        matches!(
            n.to_str(),
            Some(
                "chrome_crashpad_handler"
                    | "chrome_crashpad_handler.exe"
                    | "browser_crashpad_handler"
                    | "browser_crashpad_handler.exe"
            )
        )
    })
}

pub fn cleanup_reporters(dir: &Path) -> Result<(), String> {
    let all = snapshot()?;
    let remaining = owned(&all, dir);
    if remaining.iter().any(|p| !is_reporter(&p.exe)) {
        return Ok(());
    }
    let had_reporters = !remaining.is_empty();
    for live in remaining {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(live.pid)]),
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
        );
        if let Some(p) = system.process(sysinfo::Pid::from_u32(live.pid)) {
            if p.start_time() == live.started
                && p.exe()
                    .is_some_and(|exe| path_eq(exe, &live.exe, cfg!(windows)))
            {
                #[cfg(unix)]
                if p.kill_with(sysinfo::Signal::Term) != Some(true) {
                    let _ = p.kill();
                }
                #[cfg(windows)]
                let _ = p.kill();
            }
        }
    }
    if had_reporters {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let all = snapshot()?;
            if owned(&all, dir).is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}

pub fn close_profile(live: &Live, dir: &Path) -> Result<(), String> {
    window_action(live, true)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let all = snapshot()?;
        if !all
            .iter()
            .any(|p| p.pid == live.pid && p.started == live.started)
        {
            cleanup_reporters(dir)?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub fn require_stopped(all: &[Live], dir: &Path) -> Result<(), String> {
    if !owned(all, dir).is_empty() {
        return Err("该实例仍有运行中的进程，请关闭后重试".into());
    }
    Ok(())
}

pub fn same_process(live: &Live) -> Result<(), String> {
    if snapshot()?.iter().any(|p| {
        p.pid == live.pid && p.started == live.started && path_eq(&p.exe, &live.exe, cfg!(windows))
    }) {
        Ok(())
    } else {
        Err("进程状态已变化，请刷新后重试".into())
    }
}

pub fn window_action(live: &Live, close: bool) -> Result<(), String> {
    same_process(live)?;
    #[cfg(target_os = "macos")]
    {
        let action = if close {
            "terminate"
        } else {
            "activateWithOptions(3)"
        };
        let script = format!("ObjC.import('AppKit'); const app = $.NSRunningApplication.runningApplicationWithProcessIdentifier({}); if (!app || !app.{}) throw new Error('Window operation failed');", live.pid, action);
        output(Command::new("/usr/bin/osascript").args(["-l", "JavaScript", "-e", &script]))?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows_sys::{
            core::BOOL,
            Win32::{
                Foundation::{HWND, LPARAM},
                UI::WindowsAndMessaging::*,
            },
        };
        struct Search {
            pid: u32,
            window: HWND,
        }
        unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
            let search = &mut *(data as *mut Search);
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == search.pid
                && IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
            {
                search.window = hwnd;
                return 0;
            }
            1
        }
        let mut search = Search {
            pid: live.pid,
            window: std::ptr::null_mut(),
        };
        unsafe {
            EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
            if search.window.is_null() {
                return Err("尚未找到实例窗口，请稍后重试或从任务栏打开".into());
            }
            if close {
                if PostMessageW(search.window, WM_CLOSE, 0, 0) == 0 {
                    return Err("无法请求关闭窗口，请在 ChatGPT 中退出".into());
                }
            } else {
                ShowWindow(search.window, SW_RESTORE);
                if SetForegroundWindow(search.window) == 0 {
                    return Err("Windows 阻止切换前台，请从任务栏打开实例".into());
                }
            }
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = close;
        Err("当前系统不支持桌面实例管理".into())
    }
}

pub fn launch(app: &discovery::Installation, dir: &Path) -> Result<(), String> {
    if !app.compatible {
        return Err(app.detail.clone());
    }
    storage::verify_config(dir)?;
    cleanup_reporters(dir)?;
    let all = snapshot()?;
    if let Some(live) = main_process(&all, dir, Path::new(&app.executable)) {
        return window_action(live, false);
    }
    require_stopped(&all, dir)?;
    let mut command = Command::new(&app.executable);
    configure(&mut command, dir);
    command.arg(format!("--user-data-dir={}", dir.join("desktop").display()));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("启动失败：{e}"))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let all = snapshot()?;
        if main_process(&all, dir, Path::new(&app.executable)).is_some() {
            return Ok(());
        }
        if !all.iter().any(|p| p.pid == pid) {
            return Err("客户端启动后退出；可能是版本或 Windows 安装类型不兼容。请重新选择客户端并查看兼容说明。".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err("已发出启动请求，但尚未确认独立实例，请刷新查看；不会自动重复启动。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reporter_names_cover_both_desktop_variants() {
        for name in [
            "browser_crashpad_handler",
            "browser_crashpad_handler.exe",
            "chrome_crashpad_handler",
            "chrome_crashpad_handler.exe",
        ] {
            assert!(is_reporter(Path::new(name)));
        }
        assert!(!is_reporter(Path::new("ChatGPT.exe")));
    }
    #[test]
    fn exact_arguments_and_both_platform_paths() {
        for (win, path) in [
            (false, "/tmp/工作 space/desktop"),
            (true, "C:\\Users\\工作 space\\desktop"),
        ] {
            let args = vec![OsString::from(format!("--user-data-dir={path}"))];
            assert!(argument_path(
                &args,
                "--user-data-dir",
                Path::new(path),
                win
            ));
            assert!(!argument_path(
                &args,
                "--user-data-dir",
                Path::new(&format!("{path}-other")),
                win
            ));
            assert!(argument_path(
                &["--user-data-dir".into(), path.into()],
                "--user-data-dir",
                Path::new(path),
                win
            ));
        }
        assert!(path_eq(
            Path::new("C:/Profiles/A"),
            Path::new("c:\\profiles\\a"),
            true
        ));
        assert!(!path_eq(
            Path::new("/profiles/A"),
            Path::new("/profiles/a"),
            false
        ));
    }
    #[test]
    fn child_env_overrides_are_local_and_complete() {
        let dir = Path::new("/test/profile");
        let mut cmd = Command::new("test");
        configure(&mut cmd, dir);
        let entries: Vec<_> = cmd.get_envs().collect();
        for key in [
            "CODEX_HOME",
            "CODEX_ELECTRON_USER_DATA_PATH",
            "CODEX_SQLITE_HOME",
        ] {
            assert!(entries.iter().any(|(k, v)| *k == key && v.is_some()));
        }
        assert!(contaminated("OpenAI_API_KEY"));
        assert!(contaminated("CODEX_APP_SERVER_URL"));
        assert!(!contaminated("HOME"));
        assert!(!contaminated("SYSTEMROOT"));
    }
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "opens two temporary official desktop windows; no login or inference"]
    fn official_desktop_instances_have_independent_lifecycles() {
        struct Cleanup(Vec<PathBuf>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(all) = snapshot() {
                    for dir in &self.0 {
                        for live in owned(&all, dir) {
                            if same_process(live).is_ok() {
                                let _ = Command::new("/bin/kill")
                                    .args(["-TERM", &live.pid.to_string()])
                                    .status();
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
                if let Ok(all) = snapshot() {
                    for dir in &self.0 {
                        for live in owned(&all, dir) {
                            if same_process(live).is_ok() {
                                let _ = Command::new("/bin/kill")
                                    .args(["-KILL", &live.pid.to_string()])
                                    .status();
                            }
                        }
                    }
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        let app = discovery::discover(None).unwrap();
        let a = storage::create(&root, &mut registry, "Lifecycle A").unwrap();
        let b = storage::create(&root, &mut registry, "Lifecycle B").unwrap();
        let a = storage::profile_dir(&root, &a.id).unwrap();
        let b = storage::profile_dir(&root, &b.id).unwrap();
        let cleanup = Cleanup(vec![a.clone(), b.clone()]);
        launch(&app, &a).unwrap();
        launch(&app, &b).unwrap();
        std::thread::sleep(Duration::from_secs(6));
        let all = snapshot().unwrap();
        let first = main_process(&all, &a, Path::new(&app.executable)).unwrap();
        let second = main_process(&all, &b, Path::new(&app.executable)).unwrap();
        assert_ne!(first.pid, second.pid);
        for dir in [&a, &b] {
            assert!(dir.join("desktop/Local State").exists());
            assert!(dir.join("codex/db").is_dir());
            assert!(!dir.join("codex/auth.json").exists());
        }
        window_action(first, false).unwrap();
        close_profile(first, &a).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let all = snapshot().unwrap();
            if main_process(&all, &a, Path::new(&app.executable)).is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let all = snapshot().unwrap();
        assert!(main_process(&all, &a, Path::new(&app.executable)).is_none());
        assert!(main_process(&all, &b, Path::new(&app.executable)).is_some());
        drop(cleanup);
        let all = snapshot().unwrap();
        assert!(owned(&all, &a).is_empty());
        assert!(owned(&all, &b).is_empty());
    }
}
