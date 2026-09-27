//! Managed desktop profiles. Every mutation is serialized and restricted to a
//! registered UUID directory. Authentication remains in the official client.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

mod discovery;
mod history;
mod process;
mod rpc;
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
                process::window_action(live, matches!(request.action, Action::Stop))?;
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
