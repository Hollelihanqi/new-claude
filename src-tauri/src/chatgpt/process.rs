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

fn clear_overrides(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if contaminated(&key.to_string_lossy()) {
            command.env_remove(key);
        }
    }
}

fn private_gateway_no_proxy(base_url: &str) -> Option<String> {
    if !super::api_config::is_private_gateway_url(base_url) {
        return None;
    }
    let url = url::Url::parse(base_url).ok()?;
    let host = match url.host()? {
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
        url::Host::Domain(host) => host.to_string(),
    };
    let mut exceptions = Vec::new();
    for name in ["NO_PROXY", "no_proxy"] {
        if let Ok(current) = std::env::var(name) {
            for entry in current
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
            {
                if !exceptions
                    .iter()
                    .any(|saved: &String| saved.eq_ignore_ascii_case(entry))
                {
                    exceptions.push(entry.to_string());
                }
            }
        }
    }
    if !exceptions
        .iter()
        .any(|saved| saved.eq_ignore_ascii_case(&host))
    {
        exceptions.push(host);
    }
    Some(exceptions.join(","))
}

pub fn configure(command: &mut Command, dir: &Path) {
    clear_overrides(command);
    command
        .env("CODEX_HOME", dir.join("codex"))
        .env("CODEX_ELECTRON_USER_DATA_PATH", dir.join("desktop"))
        .env("CODEX_SQLITE_HOME", dir.join("codex/db"))
        .current_dir(dir);
    if let Some(api) = super::api_config::summary(dir)
        .ok()
        .flatten()
        .filter(|api| api.active)
    {
        let ca_bundle = crate::union_ca_bundle_path();
        if ca_bundle.exists() {
            command.env("CODEX_CA_CERTIFICATE", ca_bundle);
        }
        let no_proxy = if api.compatibility_enabled {
            private_gateway_no_proxy("http://127.0.0.1/v1")
        } else {
            private_gateway_no_proxy(&api.base_url)
        };
        if let Some(no_proxy) = no_proxy {
            // Codex runs in the official desktop process. Give only this API
            // instance the same VPN bypass that PathMux uses for model discovery.
            command.env("NO_PROXY", &no_proxy).env("no_proxy", no_proxy);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Live {
    pub pid: u32,
    pub started: u64,
    pub exe: PathBuf,
    pub args: Vec<OsString>,
    pub parent: Option<u32>,
}

pub fn profile_status(issue: bool, main: bool, window: bool, owned: bool) -> &'static str {
    if issue {
        "error"
    } else if main {
        if window {
            "running"
        } else {
            "background"
        }
    } else if owned {
        "closing"
    } else {
        "stopped"
    }
}

/// Query the desktop once per refresh, separately from process ownership.
pub fn visible_window_pids() -> Result<std::collections::HashSet<u32>, String> {
    #[cfg(windows)]
    {
        use windows_sys::{
            core::BOOL,
            Win32::{
                Foundation::{HWND, LPARAM},
                UI::WindowsAndMessaging::*,
            },
        };
        unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL {
            if IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                (*(data as *mut std::collections::HashSet<u32>)).insert(pid);
            }
            1
        }
        let mut pids = std::collections::HashSet::new();
        if unsafe { EnumWindows(Some(visit), &mut pids as *mut _ as LPARAM) } == 0 {
            return Err("无法读取窗口状态，请刷新重试".into());
        }
        Ok(pids)
    }
    #[cfg(target_os = "macos")]
    {
        // CoreGraphics window metadata needs neither UI scripting nor account access.
        let script = "ObjC.import('CoreGraphics'); const windows = ObjC.deepUnwrap($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, $.kCGNullWindowID)); JSON.stringify(windows.filter(w => w.kCGWindowLayer === 0).map(w => w.kCGWindowOwnerPID));";
        let json =
            output(Command::new("/usr/bin/osascript").args(["-l", "JavaScript", "-e", script]))?;
        serde_json::from_str(&json).map_err(|e| format!("无法读取窗口状态：{e}"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("当前系统不支持桌面窗口检测".into())
    }
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
    owned_at(all, dir, cfg!(windows))
}

fn owned_at<'a>(all: &'a [Live], dir: &Path, windows: bool) -> Vec<&'a Live> {
    let desktop = dir.join("desktop");
    let mut ids: std::collections::HashSet<u32> = all
        .iter()
        .filter(|p| {
            argument_path(&p.args, "--user-data-dir", &desktop, windows)
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
                || argument_path(&p.args, "--database", &desktop.join("Crashpad"), windows)
        })
        .map(|p| p.pid)
        .collect();
    loop {
        let n = ids.len();
        for p in all {
            let explicit_desktop = p.args.iter().any(|arg| {
                let arg = arg.to_string_lossy();
                arg == "--user-data-dir" || arg.starts_with("--user-data-dir=")
            });
            if p.parent.is_some_and(|id| ids.contains(&id))
                && (!explicit_desktop
                    || argument_path(&p.args, "--user-data-dir", &desktop, windows))
            {
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

fn primary_process<'a>(all: &'a [Live], exe: &Path) -> Option<&'a Live> {
    all.iter().find(|p| {
        path_eq(&p.exe, exe, cfg!(windows))
            && !p.args.iter().any(|a| {
                let arg = a.to_string_lossy();
                arg.starts_with("--type=")
                    || arg == "--user-data-dir"
                    || arg.starts_with("--user-data-dir=")
            })
    })
}

#[cfg(any(windows, target_os = "macos"))]
pub fn open_primary(app: &discovery::Installation) -> Result<(), String> {
    let exe = Path::new(&app.executable);
    if let Some(live) = primary_process(&snapshot()?, exe) {
        return window_action(live, false);
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.args(["-n", "-a", &app.path]);
        command
    };
    #[cfg(windows)]
    let mut command = Command::new(exe);
    clear_overrides(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("无法打开主 ChatGPT：{e}"))?;
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if primary_process(&snapshot()?, exe).is_some() {
            return Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("主 ChatGPT 启动请求失败，请检查客户端安装".into());
            }
        }
        if Instant::now() >= deadline {
            return Err("已请求打开主 ChatGPT，但尚未确认窗口，请稍后重试".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open_primary(_app: &discovery::Installation) -> Result<(), String> {
    Err("当前系统不支持打开主 ChatGPT 客户端".into())
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
            if require_stopped(&snapshot()?, dir).is_ok() {
                return Ok(());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Electron may hide its window instead of exiting on WM_CLOSE. Only retire
    // this profile's official client processes after the normal close request.
    let all = snapshot()?;
    for target in shutdown_targets(&all, dir, &live.exe) {
        terminate_verified(target)?;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if owned(&snapshot()?, dir).is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    require_stopped(&snapshot()?, dir)
}

fn shutdown_targets<'a>(all: &'a [Live], dir: &Path, exe: &Path) -> Vec<&'a Live> {
    owned(all, dir)
        .into_iter()
        .filter(|p| {
            path_eq(&p.exe, exe, cfg!(windows))
                || is_reporter(&p.exe)
                || (p.exe.file_stem().is_some_and(|name| name == "codex")
                    && exe
                        .parent()
                        .is_some_and(|installation| p.exe.starts_with(installation)))
        })
        .collect()
}

fn terminate_verified(live: &Live) -> Result<(), String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(live.pid)]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let Some(p) = system.process(sysinfo::Pid::from_u32(live.pid)) else {
        return Ok(());
    };
    if p.start_time() != live.started
        || !p
            .exe()
            .is_some_and(|exe| path_eq(exe, &live.exe, cfg!(windows)))
    {
        return Err("进程身份已变化，已停止关闭操作，请刷新重试".into());
    }
    #[cfg(unix)]
    let stopped = p.kill_with(sysinfo::Signal::Term) == Some(true);
    #[cfg(windows)]
    let stopped = p.kill();
    if stopped
        || !snapshot()?
            .iter()
            .any(|p| p.pid == live.pid && p.started == live.started)
    {
        Ok(())
    } else {
        Err("无法结束该实例的后台进程，请在 ChatGPT 中退出后重试".into())
    }
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
                && GetWindowTextLengthW(hwnd) > 0
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
    launch_at(app, dir, None)
}

pub fn launch_thread(
    app: &discovery::Installation,
    dir: &Path,
    thread_id: &str,
) -> Result<(), String> {
    thread_route(thread_id)?;
    launch_at(app, dir, Some(thread_id))
}

fn thread_route(thread_id: &str) -> Result<String, String> {
    storage::validate_id(thread_id)?;
    Ok(format!("codex://threads/{thread_id}"))
}

fn launch_at(
    app: &discovery::Installation,
    dir: &Path,
    thread_id: Option<&str>,
) -> Result<(), String> {
    if !app.compatible {
        return Err(app.detail.clone());
    }
    storage::restore_login_choices(dir)?;
    cleanup_reporters(dir)?;
    let all = snapshot()?;
    if let Some(live) = main_process(&all, dir, Path::new(&app.executable)) {
        if let Some(id) = thread_id {
            let mut command = Command::new(&app.executable);
            configure(&mut command, dir);
            command.arg(format!("--user-data-dir={}", dir.join("desktop").display()));
            command.arg(thread_route(id)?);
            quiet(&mut command)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("无法打开目标会话：{e}"))?;
        }
        return window_action(live, false);
    }
    require_stopped(&all, dir)?;
    if super::api_config::compatibility(dir)?.is_some_and(|(_, _, enabled)| enabled) {
        let port = super::bridge::start(dir, Path::new(&app.executable))?;
        super::api_config::set_runtime_bridge_url(dir, port)?;
    }
    let mut command = Command::new(&app.executable);
    configure(&mut command, dir);
    command.arg(format!("--user-data-dir={}", dir.join("desktop").display()));
    if let Some(id) = thread_id {
        command.arg(thread_route(id)?);
    }
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
    fn hidden_instance_is_background_not_running() {
        assert_eq!(profile_status(false, true, false, true), "background");
        assert_eq!(profile_status(false, true, true, true), "running");
        assert_eq!(profile_status(false, false, false, true), "closing");
        assert_eq!(profile_status(false, false, false, false), "stopped");
        assert_eq!(profile_status(true, true, true, true), "error");
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_macos_window_metadata_is_readable() {
        visible_window_pids()
            .expect("CoreGraphics window metadata must be readable without UI scripting");
    }
    #[test]
    fn closing_one_instance_never_includes_another_or_unrelated_tools() {
        for (dir, other, exe, tool) in [
            (
                "C:/profiles/a",
                "C:/profiles/b",
                "C:/Apps/ChatGPT.exe",
                "C:/Tools/editor.exe",
            ),
            (
                "/tmp/profiles/a",
                "/tmp/profiles/b",
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
                "/usr/bin/editor",
            ),
        ] {
            let make = |pid, parent, executable: &str, args: Vec<OsString>| Live {
                pid,
                parent,
                started: 1,
                exe: executable.into(),
                args,
            };
            let all = vec![
                make(
                    1,
                    None,
                    exe,
                    vec![format!("--user-data-dir={dir}/desktop").into()],
                ),
                make(
                    2,
                    Some(1),
                    exe,
                    vec![format!("--user-data-dir={other}/desktop").into()],
                ),
                make(3, Some(1), exe, vec!["--type=renderer".into()]),
                make(4, Some(1), tool, vec![]),
                make(5, Some(2), exe, vec!["--type=renderer".into()]),
            ];
            let owned: Vec<_> = owned_at(&all, Path::new(dir), dir.starts_with("C:"))
                .into_iter()
                .map(|p| p.pid)
                .collect();
            assert_eq!(owned, vec![1, 3, 4]);
            let targets: Vec<_> = shutdown_targets(&all, Path::new(dir), Path::new(exe))
                .into_iter()
                .map(|p| p.pid)
                .collect();
            assert_eq!(targets, vec![1, 3]);
            assert!(require_stopped(&all, Path::new(dir)).is_err());
            assert!(require_stopped(&[], Path::new(dir)).is_ok());
        }
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "helper used only by close_exits_a_client_that_hides_on_window_close"]
    fn close_test_worker() {
        use windows_sys::Win32::{
            Foundation::{HWND, LPARAM, LRESULT, WPARAM},
            UI::WindowsAndMessaging::*,
        };
        let Some(dir) = std::env::var_os("PATHMUX_CLOSE_TEST_DIR") else {
            return;
        };
        unsafe extern "system" fn hide_on_close(
            hwnd: HWND,
            message: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if message == WM_CLOSE {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            }
            DefWindowProcW(hwnd, message, w, l)
        }
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let title: Vec<u16> = "PathMux close regression helper\0".encode_utf16().collect();
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                160,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, hide_on_close as *const () as isize);
            fs::write(Path::new(&dir).join("ready"), "ready").unwrap();
            let mut message = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    #[cfg(windows)]
    #[test]
    fn close_exits_a_client_that_hides_on_window_close() {
        struct Cleanup(std::process::Child);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fs::create_dir(dir.join("desktop")).unwrap();
        let exe = std::env::current_exe().unwrap();
        let child = Command::new(&exe)
            .args([
                "--exact",
                "chatgpt::process::tests::close_test_worker",
                "--ignored",
                "--skip",
            ])
            .arg(format!("--user-data-dir={}", dir.join("desktop").display()))
            .env("PATHMUX_CLOSE_TEST_DIR", dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut cleanup = Cleanup(child);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.join("ready").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(dir.join("ready").exists(), "close helper failed to start");
        let all = snapshot().unwrap();
        let live = main_process(&all, dir, &exe).unwrap();
        assert_eq!(live.pid, cleanup.0.id());
        close_profile(live, dir).unwrap();
        assert!(
            cleanup.0.try_wait().unwrap().is_some(),
            "close reported success while client remained alive"
        );
        assert!(owned(&snapshot().unwrap(), dir).is_empty());
    }
    #[cfg(windows)]
    #[test]
    fn native_window_visibility_tracks_hide_restore_and_minimize() {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let title: Vec<u16> = "PathMux window-state test\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                160,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        struct Cleanup(windows_sys::Win32::Foundation::HWND);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let _cleanup = Cleanup(hwnd);
        let pid = std::process::id();
        assert!(!visible_window_pids().unwrap().contains(&pid));
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        assert!(visible_window_pids().unwrap().contains(&pid));
        unsafe {
            ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
        }
        assert!(visible_window_pids().unwrap().contains(&pid));
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        assert!(!visible_window_pids().unwrap().contains(&pid));
    }
    #[test]
    fn vpn_api_instance_bypasses_proxy_without_disabling_public_proxy() {
        let exceptions = private_gateway_no_proxy("https://vpn-test.localhost/v1").unwrap();
        assert!(exceptions
            .split(',')
            .any(|entry| entry == "vpn-test.localhost"));
        assert!(private_gateway_no_proxy("https://api.openai.com/v1").is_none());
    }
    #[test]
    fn thread_deep_link_requires_a_managed_uuid() {
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(thread_route(&id).unwrap(), format!("codex://threads/{id}"));
        assert!(thread_route("../../other").is_err());
    }
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
    fn primary_detection_excludes_managed_instances_and_helpers() {
        let exe = PathBuf::from("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT");
        let make = |pid, args: Vec<OsString>| Live {
            pid,
            started: 1,
            exe: exe.clone(),
            args,
            parent: None,
        };
        let managed = make(
            1,
            vec![
                "ChatGPT".into(),
                "--user-data-dir=/tmp/profile/desktop".into(),
            ],
        );
        let helper = make(2, vec!["ChatGPT".into(), "--type=renderer".into()]);
        let primary = make(3, vec!["ChatGPT".into()]);
        assert!(primary_process(&[managed.clone(), helper.clone()], &exe).is_none());
        assert_eq!(
            primary_process(&[managed, helper, primary], &exe).map(|p| p.pid),
            Some(3)
        );
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
