//! Managed desktop profiles. Every mutation is serialized and restricted to a
//! registered UUID directory. Authentication remains in the official client.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

mod batch;
mod discovery;
mod history;
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

fn state_at(root: &Path) -> Result<State, String> {
    let registry = storage::registry(root)?;
    let (installation, installation_issue) =
        match discovery::discover(registry.installation.as_deref()) {
            Ok(app) => (Some(app), None),
            Err(e) => (None, Some(e)),
        };
    let all = process::snapshot()?;
    let profiles = registry
        .profiles
        .iter()
        .map(|p| {
            let directory = root.join("profiles").join(&p.id);
            let issue = storage::profile_dir(root, &p.id)
                .and_then(|d| storage::verify_config(&d))
                .err();
            let exe = p
                .last_executable
                .as_deref()
                .or_else(|| installation.as_ref().map(|a| a.executable.as_str()));
            let live = exe.and_then(|e| process::main_process(&all, &directory, Path::new(e)));
            let owned = process::owned(&all, &directory);
            ProfileView {
                profile: p.clone(),
                directory: directory.to_string_lossy().into(),
                status: if issue.is_some() {
                    "error"
                } else if live.is_some() {
                    "running"
                } else if !owned.is_empty() {
                    "closing"
                } else {
                    "stopped"
                }
                .into(),
                pid: live.map(|p| p.pid),
                issue,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Launch,
    Focus,
    Stop,
    Cleanup,
    Rename,
    Delete,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    id: String,
    action: Action,
    name: Option<String>,
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
                    process::window_action(live, false)?;
                }
            }
            Action::Cleanup => {
                process::cleanup_reporters(&dir)?;
                process::require_stopped(&process::snapshot()?, &dir)?;
            }
            Action::Rename => {
                let name = storage::name(request.name.as_deref().unwrap_or(""))?;
                if r.profiles
                    .iter()
                    .any(|x| x.id != p.id && x.name.to_lowercase() == name.to_lowercase())
                {
                    return Err("该实例名称已存在".into());
                }
                r.profiles.iter_mut().find(|x| x.id == p.id).unwrap().name = name;
                storage::save(&root, &r)?;
            }
            Action::Delete => {
                process::require_stopped(&process::snapshot()?, &dir)?;
                storage::verify_config(&dir)?;
                // Rename first. If registration fails, restore the directory. A failed
                // physical removal is reported with its exact recovery location.
                let trash = root.join(format!("deleted-{}", p.id));
                storage::plain(&trash)?;
                if trash.exists() {
                    return Err("上次删除有残留，请先处理实例数据目录中的 deleted 文件夹".into());
                }
                fs::rename(&dir, &trash).map_err(|e| e.to_string())?;
                r.profiles.retain(|x| x.id != p.id);
                if let Err(e) = storage::save(&root, &r) {
                    let _ = fs::rename(&trash, &dir);
                    return Err(e);
                }
                fs::remove_dir_all(&trash).map_err(|_| {
                    format!(
                        "实例已移除，但部分文件未删除，请手动清理：{}",
                        trash.display()
                    )
                })?;
            }
        }
        state_at(&root)
    })
    .await
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
        storage::verify_config(&dir)?;
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
pub async fn chatgpt_diagnose(id: String) -> Result<Vec<String>, String> {
    blocking(move || {
        let root = storage::root()?;
        let r = storage::registry(&root)?;
        selected(&r, &id)?;
        let dir = storage::profile_dir(&root, &id)?;
        storage::verify_config(&dir)?;
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
        let identity = match account.get("account").filter(|a| !a.is_null()) {
            Some(a) if a["type"] == "chatgpt" => format!(
                "客户端保存的 ChatGPT 账号：{}（未刷新网络授权）",
                a["email"].as_str().unwrap_or("未返回邮箱")
            ),
            Some(_) => "客户端返回了非 ChatGPT 登录方式，请在官方窗口重新登录。".into(),
            None => "客户端未返回已登录账号，请在官方窗口完成登录。".into(),
        };
        Ok(vec![
            "独立目录、配置与所有权检查通过。".into(),
            "官方会话服务已确认使用此实例的数据目录。".into(),
            identity,
        ])
    })
    .await
}
