//! Managed desktop profiles. Every mutation is serialized and restricted to a
//! registered UUID directory. Authentication remains in the official client.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

mod api_config;
mod batch;
mod bridge;
mod discovery;
mod history;
mod launcher;
#[cfg(target_os = "macos")]
mod macos_windows;
mod process;
mod rpc;
mod snapshot;
mod storage;

static OPERATIONS: Mutex<()> = Mutex::new(());

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    id: String,
    name: String,
    created_at: u64,
    #[serde(default)]
    last_executable: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    version: u32,
    installation: Option<String>,
    profiles: Vec<Profile>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            version: 1,
            installation: None,
            profiles: vec![],
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    #[serde(flatten)]
    profile: Profile,
    directory: String,
    status: String,
    pid: Option<u32>,
    issue: Option<String>,
    api: Option<api_config::Summary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    installation: Option<discovery::Installation>,
    installation_issue: Option<String>,
    profiles: Vec<ProfileView>,
    picker_extensions: Vec<String>,
    picker_title: String,
    data_root: String,
}

fn selected<'a>(r: &'a Registry, id: &str) -> Result<&'a Profile, String> {
    r.profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "实例不存在，请刷新".into())
}

fn delete_stopped_profile(
    root: &Path,
    registry: &mut Registry,
    profile: &Profile,
    dir: &Path,
) -> Result<(), String> {
    storage::verify_config(dir)?;
    // Keep the data recoverable until the registry update has succeeded. A failed
    // physical removal returns the exact remaining path instead of reporting success.
    let trash = root.join(format!("deleted-{}", profile.id));
    storage::plain(&trash)?;
    if trash.exists() {
        return Err("上次删除有残留，请先处理实例数据目录中的 deleted 文件夹".into());
    }
    fs::rename(dir, &trash).map_err(|e| e.to_string())?;
    let previous_profiles = registry.profiles.clone();
    registry.profiles.retain(|x| x.id != profile.id);
    if let Err(e) = storage::save(root, registry) {
        registry.profiles = previous_profiles;
        return match fs::rename(&trash, dir) {
            Ok(()) => Err(e),
            Err(restore_error) => Err(format!(
                "保存实例登记失败：{e}；恢复原目录失败：{restore_error}。数据仍在：{}",
                trash.display()
            )),
        };
    }
    fs::remove_dir_all(&trash).map_err(|_| {
        format!(
            "实例已移除，但部分文件未删除，请手动清理：{}",
            trash.display()
        )
    })
}

fn state_at(root: &Path) -> Result<State, String> {
    let registry = storage::registry(root)?;
    let (installation, installation_issue) =
        match discovery::discover(registry.installation.as_deref()) {
            Ok(app) => (Some(app), None),
            Err(e) => (None, Some(e)),
        };
    let all = process::snapshot()?;
    let visible = process::visible_window_pids()?;
    let profiles = registry
        .profiles
        .iter()
        .map(|p| {
            let directory = root.join("profiles").join(&p.id);
            let issue = storage::profile_dir(root, &p.id)
                .and_then(|d| storage::restore_login_choices(&d))
                .err();
            let api = if issue.is_none() {
                api_config::summary(&directory).ok().flatten()
            } else {
                None
            };
            let exe = p
                .last_executable
                .as_deref()
                .or_else(|| installation.as_ref().map(|a| a.executable.as_str()));
            let live = exe.and_then(|e| process::main_process(&all, &directory, Path::new(e)));
            let owned = process::owned(&all, &directory);
            ProfileView {
                profile: p.clone(),
                directory: directory.to_string_lossy().into(),
                status: process::profile_status(
                    issue.is_some(),
                    live.is_some(),
                    live.is_some_and(|p| visible.contains(&p.pid)),
                    !owned.is_empty(),
                )
                .into(),
                pid: live.map(|p| p.pid),
                issue,
                api,
            }
        })
        .collect();
    Ok(State {
        installation,
        installation_issue,
        profiles,
        picker_extensions: if cfg!(windows) {
            vec!["exe".into()]
        } else {
            vec!["app".into()]
        },
        picker_title: if cfg!(windows) {
            "选择 ChatGPT 应用程序"
        } else {
            "选择 ChatGPT.app"
        }
        .into(),
        data_root: root.to_string_lossy().into(),
    })
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| "实例管理状态异常，请重启 PathMux")?;
        f()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn chatgpt_state() -> Result<State, String> {
    blocking(|| state_at(&storage::root()?)).await
}

#[tauri::command]
pub async fn chatgpt_open_primary() -> Result<(), String> {
    blocking(|| {
        let root = storage::root()?;
        let registry = storage::registry(&root)?;
        let app = discovery::discover(registry.installation.as_deref())?;
        process::open_primary(&app)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_set_installation(path: String) -> Result<State, String> {
    blocking(move || {
        let app = discovery::inspect(Path::new(&path))?;
        let root = storage::root()?;
        let mut r = storage::registry(&root)?;
        r.installation = Some(app.path);
        storage::save(&root, &r)?;
        state_at(&root)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_create_profile(name: String) -> Result<State, String> {
    blocking(move || {
        let root = storage::root()?;
        let mut r = storage::registry(&root)?;
        storage::create(&root, &mut r, &name)?;
        state_at(&root)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_discover_api_models(
    request: api_config::DiscoverRequest,
) -> Result<Vec<api_config::Model>, String> {
    blocking(move || {
        let root = storage::root()?;
        let registry = storage::registry(&root)?;
        selected(&registry, &request.id)?;
        let dir = storage::profile_dir(&root, &request.id)?;
        api_config::discover(&dir, &request)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_save_api_config(request: api_config::Request) -> Result<State, String> {
    blocking(move || {
        let root = storage::root()?;
        let registry = storage::registry(&root)?;
        selected(&registry, &request.id)?;
        let dir = storage::profile_dir(&root, &request.id)?;
        process::require_stopped(&process::snapshot()?, &dir)?;
        api_config::save(&dir, &request)?;
        state_at(&root)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_use_account_login(id: String) -> Result<State, String> {
    blocking(move || {
        let root = storage::root()?;
        let registry = storage::registry(&root)?;
        selected(&registry, &id)?;
        let dir = storage::profile_dir(&root, &id)?;
        process::require_stopped(&process::snapshot()?, &dir)?;
        api_config::use_account(&dir)?;
        state_at(&root)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_set_api_compatibility(id: String, enabled: bool) -> Result<State, String> {
    blocking(move || {
        let root = storage::root()?;
        let registry = storage::registry(&root)?;
        selected(&registry, &id)?;
        let dir = storage::profile_dir(&root, &id)?;
        process::require_stopped(&process::snapshot()?, &dir)?;
        api_config::set_compatibility(&dir, enabled)?;
        state_at(&root)
    })
    .await
}

pub fn run_bridge_helper(dir: &Path, executable: &Path) -> Result<(), String> {
    bridge::serve(dir, executable)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Launch,
    Focus,
    Stop,
    Cleanup,
    Delete,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    id: String,
    action: Action,
}

#[tauri::command]
pub async fn chatgpt_profile_action(request: ActionRequest) -> Result<State, String> {
    blocking(move || {
        let root = storage::root()?;
        let mut r = storage::registry(&root)?;
        let p = selected(&r, &request.id)?.clone();
        let dir = storage::profile_dir(&root, &p.id)?;
        match request.action {
            Action::Launch => {
                let app = discovery::discover(r.installation.as_deref())?;
                // Persist identity before spawning so recovery works if PathMux exits.
                r.profiles
                    .iter_mut()
                    .find(|x| x.id == p.id)
                    .unwrap()
                    .last_executable = Some(app.executable.clone());
                storage::save(&root, &r)?;
                process::launch(&app, &dir)?;
            }
            Action::Focus | Action::Stop => {
                let all = process::snapshot()?;
                let exe = p
                    .last_executable
                    .as_deref()
                    .ok_or("该实例尚未由 PathMux 启动")?;
                let live = process::main_process(&all, &dir, Path::new(exe))
                    .ok_or("实例没有运行中的主窗口，请刷新")?;
                if matches!(request.action, Action::Stop) {
                    process::close_profile(live, &dir)?;
                } else {
                    process::reopen(live, &dir, None)?;
                }
            }
            Action::Cleanup => {
                process::cleanup_reporters(&dir)?;
                process::require_stopped(&process::snapshot()?, &dir)?;
            }
            Action::Delete => {
                process::require_stopped(&process::snapshot()?, &dir)?;
                delete_stopped_profile(&root, &mut r, &p, &dir)?;
            }
        }
        state_at(&root)
    })
    .await
}

#[cfg(test)]
mod delete_tests {
    use super::*;

    #[test]
    fn deletion_removes_only_the_selected_profile_and_its_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        let a = storage::create(&root, &mut registry, "A").unwrap();
        let b = storage::create(&root, &mut registry, "B").unwrap();
        let a_dir = storage::profile_dir(&root, &a.id).unwrap();
        let b_dir = storage::profile_dir(&root, &b.id).unwrap();
        fs::write(a_dir.join("desktop/session"), "login").unwrap();
        fs::write(a_dir.join("codex/history"), "history").unwrap();
        fs::write(a_dir.join("logs/last.log"), "log").unwrap();
        fs::write(b_dir.join("desktop/session"), "keep").unwrap();

        delete_stopped_profile(&root, &mut registry, &a, &a_dir).unwrap();

        assert!(!a_dir.exists());
        assert!(!root.join(format!("deleted-{}", a.id)).exists());
        assert_eq!(
            fs::read_to_string(b_dir.join("desktop/session")).unwrap(),
            "keep"
        );
        assert!(storage::registry(&root)
            .unwrap()
            .profiles
            .iter()
            .all(|p| p.id != a.id));
        assert!(storage::registry(&root)
            .unwrap()
            .profiles
            .iter()
            .any(|p| p.id == b.id));
    }
}

#[tauri::command]
pub async fn chatgpt_history(source_id: String) -> Result<history::HistoryList, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        history::list(&root, &r, &source_id)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_transfer(
    request: history::TransferRequest,
) -> Result<history::TransferResult, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        history::transfer(&root, &r, request)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_open_thread(target_id: String, thread_id: String) -> Result<State, String> {
    blocking(move || {
        storage::validate_id(&thread_id)?;
        let root = storage::root()?;
        let mut r = storage::registry(&root)?;
        selected(&r, &target_id)?;
        let dir = storage::profile_dir(&root, &target_id)?;
        storage::restore_login_choices(&dir)?;
        let app = discovery::discover(r.installation.as_deref())?;
        if !app.compatible {
            return Err(app.detail);
        }
        let cli = app.cli.as_deref().ok_or("客户端缺少会话服务")?;
        let mut client = rpc::Client::start(cli, &dir)?;
        let read = client.call(
            "thread/read",
            serde_json::json!({"threadId":thread_id,"includeTurns":false}),
        )?;
        let path = read
            .pointer("/thread/path")
            .and_then(serde_json::Value::as_str)
            .ok_or("无法确认会话位置")?;
        if !std::fs::canonicalize(path)
            .map_err(|e| e.to_string())?
            .starts_with(
                dir.join("codex")
                    .canonicalize()
                    .map_err(|e| e.to_string())?,
            )
        {
            return Err("会话不属于目标账号实例".into());
        }
        drop(client);
        r.profiles
            .iter_mut()
            .find(|p| p.id == target_id)
            .unwrap()
            .last_executable = Some(app.executable.clone());
        storage::save(&root, &r)?;
        process::launch_thread(&app, &dir, &thread_id)?;
        state_at(&root)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_preview(
    request: history::TransferRequest,
) -> Result<history::Preview, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        history::preview(&root, &r, &request)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_pending(target_id: String) -> Result<Vec<history::PendingTransfer>, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        history::pending(&root, &r, &target_id)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_recover(
    target_id: String,
    key: String,
    discard: bool,
) -> Result<history::TransferResult, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        history::recover(&root, &r, &target_id, &key, discard)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_batch_create(
    requests: Vec<history::TransferRequest>,
) -> Result<batch::Job, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        batch::create(&root, &r, requests)
    })
    .await
}
#[tauri::command]
pub async fn chatgpt_batch_list(target_id: String) -> Result<Vec<batch::Job>, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        batch::list(&root, &r, &target_id)
    })
    .await
}
#[tauri::command]
pub async fn chatgpt_batch_step(
    target_id: String,
    id: String,
    cancel: bool,
) -> Result<batch::Job, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        batch::step(&root, &r, &target_id, &id, cancel)
    })
    .await
}

#[tauri::command]
pub async fn chatgpt_diagnose(id: String) -> Result<DiagnosticResult, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        selected(&r, &id)?;
        let dir = storage::profile_dir(&root, &id)?;
        storage::restore_login_choices(&dir)?;
        process::require_stopped(&process::snapshot()?, &dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for p in [&dir, &dir.join("codex"), &dir.join("desktop")] {
                if fs::metadata(p)
                    .map_err(|e| e.to_string())?
                    .permissions()
                    .mode()
                    & 0o077
                    != 0
                {
                    return Err("实例目录允许其他用户访问，请恢复私有目录权限后重试".into());
                }
            }
        }
        let app = discovery::discover(r.installation.as_deref())?;
        if !app.compatible {
            return Err(app.detail);
        }
        let mut client = rpc::Client::start(app.cli.as_deref().ok_or("客户端没有会话服务")?, &dir)?;
        let account = client.call("account/read", serde_json::json!({"refreshToken":false}))?;
        let (identity, healthy) = account_diagnostic(&account);
        Ok(DiagnosticResult {
            healthy,
            details: vec![
                "独立目录、配置与所有权检查通过。".into(),
                "官方会话服务已确认使用此实例的数据目录。".into(),
                identity,
            ],
        })
    })
    .await
}

fn account_diagnostic(result: &serde_json::Value) -> (String, bool) {
    match result.get("account").filter(|a| !a.is_null()) {
        Some(a) if a["type"] == "chatgpt" => (
            format!(
                "客户端保存的 ChatGPT 账号：{}（未刷新网络授权）",
                a["email"].as_str().unwrap_or("未返回邮箱")
            ),
            true,
        ),
        Some(a) if a["type"] == "apiKey" => (
            "客户端已启用 API Key 登录（凭证由官方客户端管理）。".into(),
            true,
        ),
        Some(a) if a["type"] == "amazonBedrock" => (
            "客户端已启用 Amazon Bedrock 登录（凭证由官方客户端管理）。".into(),
            true,
        ),
        Some(_) => (
            "官方客户端已完成登录（登录方式由客户端管理）。".into(),
            true,
        ),
        None if result["requiresOpenaiAuth"] == false => {
            ("客户端当前配置无需 OpenAI 登录。".into(), true)
        }
        None => (
            "客户端未返回已登录账号，请在官方窗口选择账号或 API Key 登录。".into(),
            false,
        ),
    }
}

#[derive(Serialize)]
pub struct DiagnosticResult {
    healthy: bool,
    details: Vec<String>,
}

#[cfg(test)]
mod login_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_key_and_chatgpt_logins_are_both_healthy() {
        for account in [
            json!({"type":"chatgpt","email":"user@example.com"}),
            json!({"type":"apiKey","apiKey":"sk-test-secret"}),
        ] {
            let (detail, healthy) = account_diagnostic(&json!({"account":account}));
            assert!(healthy);
            assert!(!detail.contains("sk-"));
        }
        assert!(account_diagnostic(&json!({"account":null,"requiresOpenaiAuth":false})).1);
        assert!(!account_diagnostic(&json!({"account":null})).1);
    }
}
