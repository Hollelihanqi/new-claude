//! Keep Store app identity without sharing a profile's credentials or directories.
use super::*;

#[cfg(windows)]
pub fn package_root(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .take(4)
        .find(|dir| dir.join("AppxManifest.xml").is_file())
        .map(Path::to_path_buf)
}

pub fn command(original: Command, primary: bool) -> Result<Command, String> {
    #[cfg(windows)]
    if let Some(root) = package_root(Path::new(original.get_program())) {
        let target = PathBuf::from(original.get_program());
        return packaged_command(original, &root, primary, &target);
    }
    let _ = primary;
    Ok(original)
}

pub fn prepare_runtime(exe: &Path, dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if package_root(exe).is_some() {
        return sync_runtime_preferences(
            &dir.join("codex/.codex-global-state.json"),
            &crate::home().join(".codex/.codex-global-state.json"),
        );
    }
    let _ = (exe, dir);
    Ok(())
}

#[cfg(any(windows, test))]
fn sync_runtime_preferences(profile: &Path, primary: &Path) -> Result<(), String> {
    if !primary.exists() {
        return Ok(());
    }
    let read_state = |path: &Path| -> Result<serde_json::Value, String> {
        if fs::metadata(path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
            return Err("客户端状态文件过大，无法安全同步运行框架设置".into());
        }
        storage::read_json(path)
            .map_err(|_| "客户端状态文件无法解析，请在官方客户端中检查设置后重试".into())
    };
    let source = read_state(primary)?;
    let flags: Vec<_> = [
        "electron-windows-core-runtime-frameworks-enabled",
        "electron-windows-primary-runtime-frameworks-enabled",
    ]
    .into_iter()
    .filter_map(|key| {
        source
            .get(key)
            .and_then(|value| value.as_bool())
            .map(|value| (key, value))
    })
    .collect();
    if flags.is_empty() {
        return Ok(());
    }
    storage::plain(profile)?;
    let mut target = if profile.exists() {
        read_state(profile)?
    } else {
        serde_json::json!({})
    };
    let state = target
        .as_object_mut()
        .ok_or("实例客户端状态格式无效，已停止同步")?;
    let mut changed = false;
    for (key, value) in flags {
        if state.get(key).and_then(|value| value.as_bool()) != Some(value) {
            state.insert(key.to_owned(), value.into());
            changed = true;
        }
    }
    if changed {
        storage::write_json(profile, &target)?;
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn executable_path(value: &str) -> String {
    if let Some(path) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{path}")
    } else {
        value.strip_prefix("\\\\?\\").unwrap_or(value).to_owned()
    }
}

#[cfg(any(windows, test))]
fn quote_argument(value: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            '\\',
            if character == '"' {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        out.push(character);
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

// Appx activation deliberately does not inherit our process environment. A
// short-lived package worker rebuilds the explicit profile environment before
// starting the client. Payload data is base64, never interpolated as shell code.
#[cfg(any(windows, test))]
const WORKER: &str = r#"
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$request=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__PAYLOAD__')) | ConvertFrom-Json
$info=New-Object Diagnostics.ProcessStartInfo
$info.FileName=$request.program
$info.Arguments=$request.arguments
$info.UseShellExecute=$false
$info.CreateNoWindow=$true
if ($request.cwd) { $info.WorkingDirectory=$request.cwd }
foreach ($key in @($info.EnvironmentVariables.Keys)) {
 if ($key -match '^(CODEX_|OPENAI_|CHATGPT_|ELECTRON_|NODE_)') { $info.EnvironmentVariables.Remove($key) }
}
foreach ($entry in $request.env.PSObject.Properties) { $info.EnvironmentVariables[$entry.Name]=[string]$entry.Value }
$child=[Diagnostics.Process]::Start($info)
if (-not $child) { throw 'Client launch failed' }
# Keep the activation owner alive until the client exits, rather than relying
# on the lifetime of a detached debugging activation context.
$child.WaitForExit()
exit $child.ExitCode
"#;

#[cfg(any(windows, test))]
const ACTIVATE: &str = r#"
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
function Normalize-Path([string]$path) { [IO.Path]::GetFullPath($path).Replace('\\?\','').TrimEnd('\') }
$root=Normalize-Path $env:PATHMUX_PACKAGE_ROOT
$packages=@(Get-AppxPackage | Where-Object { (Normalize-Path $_.InstallLocation) -eq $root })
if ($packages.Count -ne 1) { throw 'Selected ChatGPT package is not registered for this user; select the client again' }
$package=$packages[0]
$manifest=Get-AppxPackageManifest -Package $package.PackageFullName
$target=Normalize-Path $env:PATHMUX_PACKAGE_TARGET
$entries=@($manifest.Package.Applications.Application | Where-Object { (Normalize-Path (Join-Path $root $_.Executable)) -eq $target })
if ($entries.Count -ne 1) { throw 'Selected executable does not match a registered application' }
if ($env:PATHMUX_PACKAGE_PRIMARY -eq '1') {
 Start-Process -FilePath ('shell:AppsFolder\'+$package.PackageFamilyName+'!'+$entries[0].Id)
} else {
 Invoke-CommandInDesktopPackage -PackageFamilyName $package.PackageFamilyName -AppId $entries[0].Id -Command $env:PATHMUX_PACKAGE_LAUNCHER -Args ('--chatgpt-package-worker '+$env:PATHMUX_PACKAGE_WORKER) -PreventBreakaway -ErrorAction Stop
}
exit 0
"#;

#[cfg(windows)]
fn encoded_script(script: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    )
}

#[cfg(windows)]
fn packaged_command(
    original: Command,
    root: &Path,
    primary: bool,
    target: &Path,
) -> Result<Command, String> {
    packaged_command_with_scripts(original, root, primary, target, ACTIVATE, WORKER)
}

#[cfg(windows)]
fn package_worker_command(script: &str) -> Result<Command, String> {
    use base64::Engine;
    // Enter before Tauri initialization. The GUI executable receives package
    // identity without allocating a console and starts the isolated script
    // with CREATE_NO_WINDOW, rather than hiding an already created terminal.
    if script.len() > 30_000 {
        return Err("实例启动参数过长".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(script)
        .map_err(|_| "实例启动参数无效")?;
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return Err("实例启动参数无效".into());
    }
    let shell = PathBuf::from(std::env::var_os("SystemRoot").ok_or("无法定位 Windows 系统目录")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut command = Command::new(shell);
    command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", script]);
    process::quiet(&mut command);
    Ok(command)
}

#[cfg(windows)]
pub fn run_package_worker(script: &str) -> Result<i32, String> {
    let status = package_worker_command(script)?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("无法启动实例后台助手：{e}"))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(windows)]
fn packaged_command_with_scripts(
    original: Command,
    root: &Path,
    primary: bool,
    target: &Path,
    activation: &str,
    worker: &str,
) -> Result<Command, String> {
    use base64::Engine;
    let shell = PathBuf::from(std::env::var_os("SystemRoot").ok_or("无法定位 Windows 系统目录")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if !shell.is_file() {
        return Err("无法定位 Windows 应用启动组件，请从系统程序列表打开 ChatGPT".into());
    }
    let launcher = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(test)]
    let launcher = PathBuf::from(
        std::env::var_os("PATHMUX_PACKAGE_TEST_LAUNCHER")
            .unwrap_or_else(|| launcher.into_os_string()),
    );
    let env: std::collections::BTreeMap<_, _> = original
        .get_envs()
        .filter_map(|(key, value)| {
            value.map(|value| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
        })
        .collect();
    let payload = serde_json::json!({
        "program": executable_path(&original.get_program().to_string_lossy()),
        "arguments": original.get_args().map(|arg| quote_argument(&arg.to_string_lossy())).collect::<Vec<_>>().join(" "),
        "cwd": original.get_current_dir().map(|dir| dir.to_string_lossy()),
        "env": env,
    });
    let payload = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&payload).map_err(|e| e.to_string())?);
    let mut command = Command::new(&shell);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
            &encoded_script(activation),
        ])
        .env("PATHMUX_PACKAGE_ROOT", root)
        .env("PATHMUX_PACKAGE_TARGET", target)
        .env("PATHMUX_PACKAGE_PRIMARY", if primary { "1" } else { "0" })
        .env("PATHMUX_PACKAGE_LAUNCHER", launcher)
        .env(
            "PATHMUX_PACKAGE_WORKER",
            encoded_script(&worker.replace("__PAYLOAD__", &payload)),
        );
    for (key, value) in original.get_envs() {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    if let Some(dir) = original.get_current_dir() {
        command.current_dir(dir);
    }
    process::quiet(&mut command);
    Ok(command)
}

pub fn verify_identity(live: &process::Live) -> Result<(), String> {
    #[cfg(windows)]
    if let Some(root) = package_root(&live.exe) {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER},
            Storage::Packaging::Appx::GetPackageFullName,
            System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, live.pid) };
        if handle.is_null() {
            return Err("无法确认客户端的 Windows 应用身份，请刷新后重试".into());
        }
        let mut length = 0;
        let mut name = Vec::<u16>::new();
        let mut result = unsafe { GetPackageFullName(handle, &mut length, std::ptr::null_mut()) };
        if result == ERROR_INSUFFICIENT_BUFFER && length <= 32768 {
            name.resize(length as usize, 0);
            result = unsafe { GetPackageFullName(handle, &mut length, name.as_mut_ptr()) };
        }
        unsafe {
            CloseHandle(handle);
        }
        let name = String::from_utf16_lossy(name.split(|unit| *unit == 0).next().unwrap_or(&[]));
        if result != 0
            || !root
                .file_name()
                .is_some_and(|expected| expected.to_string_lossy().eq_ignore_ascii_case(&name))
        {
            return Err("该窗口仍使用旧启动方式，缺少 Windows 应用身份。请先关闭实例，再重新启动以恢复完整运行环境。".into());
        }
    }
    let _ = live;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_arguments_preserve_unicode_quotes_and_trailing_slashes() {
        assert_eq!(quote_argument(""), "\"\"");
        assert_eq!(quote_argument("C:\\工作 space\\"), "\"C:\\工作 space\\\\\"");
        assert_eq!(quote_argument("a\\\"b"), "\"a\\\\\\\"b\"");
        assert_eq!(
            quote_argument("O'Brien;$(command)"),
            "\"O'Brien;$(command)\""
        );
    }
    #[test]
    fn client_executable_path_removes_device_prefix_without_changing_unc_paths() {
        assert_eq!(
            executable_path(r"\\?\C:\Program Files\ChatGPT.exe"),
            r"C:\Program Files\ChatGPT.exe"
        );
        assert_eq!(
            executable_path(r"\\?\UNC\server\apps\ChatGPT.exe"),
            r"\\server\apps\ChatGPT.exe"
        );
        assert_eq!(
            executable_path(r"C:\apps\ChatGPT.exe"),
            r"C:\apps\ChatGPT.exe"
        );
    }
    #[test]
    fn package_worker_applies_isolation_after_activation_without_changing_approvals() {
        assert!(WORKER.contains("$info.EnvironmentVariables[$entry.Name]"));
        assert!(
            WORKER
                .find("$info.EnvironmentVariables[$entry.Name]")
                .unwrap()
                < WORKER.find("[Diagnostics.Process]::Start").unwrap()
        );
        for policy in [
            "approvals_reviewer",
            "approval_policy",
            "sandbox_mode",
            "full-access",
        ] {
            assert!(!WORKER.contains(policy));
        }
        assert!(ACTIVATE.contains("shell:AppsFolder"));
        assert!(ACTIVATE.contains("-PreventBreakaway"));
        assert!(ACTIVATE.contains("-Command $env:PATHMUX_PACKAGE_LAUNCHER"));
        assert!(!ACTIVATE.contains("-Command $env:PATHMUX_PACKAGE_POWERSHELL"));
    }
    #[test]
    fn non_store_and_macos_launches_keep_original_arguments_and_environment() {
        for exe in [
            "C:/Programs/ChatGPT.exe",
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
        ] {
            let mut original = Command::new(exe);
            original
                .arg("--user-data-dir=独立 space")
                .env("CODEX_HOME", "separate");
            let command = command(original, false).unwrap();
            assert_eq!(command.get_program(), exe);
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                ["--user-data-dir=独立 space"]
            );
            assert!(command.get_envs().any(|(key, value)| key == "CODEX_HOME"
                && value == Some(std::ffi::OsStr::new("separate"))));
        }
    }
    #[test]
    fn runtime_preferences_sync_does_not_copy_permissions_accounts_or_other_state() {
        let temp = tempfile::tempdir().unwrap();
        // macOS exposes its temporary directory through the /var system alias.
        // Keep the fixture on the physical path so it obeys managed-path checks.
        let root = temp.path().canonicalize().unwrap();
        let primary = root.join("primary.json");
        let profile = root.join("profile.json");
        storage::write_json(&primary, &serde_json::json!({
            "electron-windows-primary-runtime-frameworks-enabled": true,
            "electron-windows-core-runtime-frameworks-enabled": false,
            "electron-persisted-atom-state": {"composer-permission-mode-visibility":{"guardian-approvals":true}},
            "private-account-state": "primary-only"
        })).unwrap();
        let original = serde_json::json!({
            "electron-windows-primary-runtime-frameworks-enabled": false,
            "electron-persisted-atom-state": {"composer-permission-mode-visibility":false},
            "approvals_reviewer": "user",
            "private-account-state": "profile-only"
        });
        storage::write_json(&profile, &original).unwrap();
        sync_runtime_preferences(&profile, &primary).unwrap();
        let state: serde_json::Value = storage::read_json(&profile).unwrap();
        assert_eq!(
            state["electron-windows-primary-runtime-frameworks-enabled"],
            true
        );
        assert_eq!(
            state["electron-windows-core-runtime-frameworks-enabled"],
            false
        );
        assert_eq!(
            state["electron-persisted-atom-state"],
            original["electron-persisted-atom-state"]
        );
        assert_eq!(state["approvals_reviewer"], "user");
        assert_eq!(state["private-account-state"], "profile-only");
        sync_runtime_preferences(&profile, &root.join("missing.json")).unwrap();
        assert_eq!(
            storage::read_json::<serde_json::Value>(&profile).unwrap(),
            state
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a registered official Store app; probes helpers without opening ChatGPT"]
    fn native_package_helpers_do_not_allocate_terminal_windows() {
        use base64::Engine;
        use std::time::{Duration, Instant};
        let exe = PathBuf::from(
            std::env::var_os("PATHMUX_PACKAGE_TEST_EXE").expect("set installed app path"),
        );
        let root = package_root(&exe).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let launcher = PathBuf::from(
            std::env::var_os("PATHMUX_PACKAGE_TEST_LAUNCHER").expect("set built GUI helper path"),
        );
        let image = fs::read(&launcher).unwrap();
        let pe_offset = u32::from_le_bytes(image[0x3c..0x40].try_into().unwrap()) as usize;
        let subsystem_offset = pe_offset + 4 + 20 + 68;
        assert_eq!(
            u16::from_le_bytes(
                image[subsystem_offset..subsystem_offset + 2]
                    .try_into()
                    .unwrap()
            ),
            2,
            "package activation must target a GUI executable"
        );
        let outer = temp.path().join("outer.json");
        let inner = temp.path().join("inner.json");
        let probe = |path: &Path| {
            let path =
                base64::engine::general_purpose::STANDARD.encode(path.to_string_lossy().as_bytes());
            format!(
                r#"
Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class ConsoleProbe{{[DllImport("kernel32.dll")]public static extern IntPtr GetConsoleWindow();}}'
@{{console=[ConsoleProbe]::GetConsoleWindow().ToInt64()}} | ConvertTo-Json -Compress | Set-Content -LiteralPath ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{path}'))) -Encoding UTF8
"#
            )
        };
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut original = Command::new(shell);
        original.args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"]);
        process::configure(&mut original, temp.path());
        let mut command = packaged_command_with_scripts(
            original,
            &root,
            false,
            &exe,
            &format!("{}{}", probe(&outer), ACTIVATE),
            &format!("{}{}", probe(&inner), WORKER),
        )
        .unwrap();
        // Use the launch call site's plain spawn: quiet() here would conceal
        // a missing console-creation flag on the returned activation command.
        let mut child = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        let deadline = Instant::now() + Duration::from_secs(15);
        while !inner.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let read = |path: &Path| -> serde_json::Value {
            let text = fs::read_to_string(path).expect("console probe did not finish");
            serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap()
        };
        let outer_console = read(&outer)["console"].as_i64().unwrap();
        let inner_console = read(&inner)["console"].as_i64().unwrap();
        assert_eq!(
            (outer_console, inner_console),
            (0, 0),
            "launch helpers allocated terminal windows: outer={outer_console}, inner={inner_console}"
        );
    }
    #[cfg(windows)]
    #[test]
    fn package_worker_rejects_invalid_or_oversized_encoded_scripts() {
        for invalid in ["", "not-base64", "YQ=="] {
            assert!(package_worker_command(invalid).is_err());
        }
        assert!(package_worker_command(&"A".repeat(30_001)).is_err());
        let valid = encoded_script("exit 0");
        let command = package_worker_command(&valid).unwrap();
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-NoProfile", "-NonInteractive", "-EncodedCommand", &valid]
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a registered official Store app; set PATHMUX_PACKAGE_TEST_EXE"]
    fn native_package_launch_preserves_identity_and_profile_environment() {
        use std::time::{Duration, Instant};
        let exe = PathBuf::from(
            std::env::var_os("PATHMUX_PACKAGE_TEST_EXE").expect("set installed app path"),
        );
        let root = package_root(&exe).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("native.json");
        let fixture = r#"
Add-Type -TypeDefinition 'using System;using System.Text;using System.Runtime.InteropServices;public static class IdentityFixture{[DllImport("kernel32.dll",CharSet=CharSet.Unicode)]static extern int GetCurrentPackageFullName(ref uint length,StringBuilder name);public static string Package(){uint n=0;int r=GetCurrentPackageFullName(ref n,null);if(r!=122)return "error:"+r;var b=new StringBuilder((int)n);r=GetCurrentPackageFullName(ref n,b);return r==0?b.ToString():"error:"+r;}}'
@{package=[IdentityFixture]::Package(); home=$env:CODEX_HOME; desktop=$env:CODEX_ELECTRON_USER_DATA_PATH; database=$env:CODEX_SQLITE_HOME; marker=$env:PATHMUX_TEST_MARKER} | ConvertTo-Json -Compress | Set-Content -LiteralPath $env:PATHMUX_TEST_OUTPUT -Encoding UTF8
"#;
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut original = Command::new(shell);
        original.args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded_script(fixture),
        ]);
        process::configure(&mut original, temp.path());
        original
            .env("PATHMUX_TEST_OUTPUT", &output)
            .env("PATHMUX_TEST_MARKER", "工作 space ' ; $()");
        let mut command = packaged_command(original, &root, false, &exe).unwrap();
        // This deliberately fails if the package worker loses the independent
        // environment, as direct Invoke-CommandInDesktopPackage did in the repro.
        let mut child = process::quiet(&mut command).spawn().unwrap();
        assert!(child.wait().unwrap().success());
        let deadline = Instant::now() + Duration::from_secs(15);
        while !output.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let text = fs::read_to_string(&output).expect("package worker did not finish");
        let state: serde_json::Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(
            state["package"].as_str(),
            root.file_name().and_then(|name| name.to_str())
        );
        assert_eq!(state["home"].as_str(), temp.path().join("codex").to_str());
        assert_eq!(
            state["desktop"].as_str(),
            temp.path().join("desktop").to_str()
        );
        assert_eq!(
            state["database"].as_str(),
            temp.path().join("codex/db").to_str()
        );
        assert_eq!(state["marker"], "工作 space ' ; $()");
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "opens a temporary official client; set PATHMUX_PACKAGE_TEST_EXE"]
    fn native_chatgpt_launch_keeps_package_identity_without_using_primary_data() {
        let exe = PathBuf::from(
            std::env::var_os("PATHMUX_PACKAGE_TEST_EXE").expect("set installed app path"),
        );
        let app = discovery::inspect(&exe).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let mut registry = storage::registry(temp.path()).unwrap();
        let profile =
            storage::create(temp.path(), &mut registry, "Package launch regression").unwrap();
        let dir = storage::profile_dir(temp.path(), &profile.id).unwrap();
        struct Cleanup(PathBuf, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(all) = process::snapshot() {
                    if let Some(live) = process::main_process(&all, &self.0, &self.1) {
                        let _ = process::close_profile(live, &self.0);
                    }
                }
            }
        }
        let _cleanup = Cleanup(dir.clone(), exe.clone());
        process::launch(&app, &dir).unwrap();
        let all = process::snapshot().unwrap();
        let live = process::main_process(&all, &dir, &exe).unwrap();
        verify_identity(live).unwrap();
        assert!(process::argument_path(
            &live.args,
            "--user-data-dir",
            &dir.join("desktop"),
            true
        ));
        assert!(
            !dir.join("codex/auth.json").exists(),
            "fixture must not inherit primary credentials"
        );
        process::close_profile(live, &dir).unwrap();
        assert!(process::owned(&process::snapshot().unwrap(), &dir).is_empty());
    }
}
