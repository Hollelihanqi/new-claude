use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::error::Error as StdError;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// 注意：这里**不再**有任何默认网关地址。
// 原先是一个公司内网地址，后果有两层：
//   1) 泄漏组织内网地址（本仓要公开）；
//   2) 它同时是前端表单的**预填值** —— 公开用户打开页面时地址栏已被填好，
//      不动它直接保存就会指向一个不可达的内网地址。
// 因此所有回退值改为空串，由界面用 placeholder 提示用户填写自己的网关。
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const ORGANIZATION_OWNER_FIELD: &str = "_ccManagerOrganizationId";
static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn write_guard() -> MutexGuard<'static, ()> {
    WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyEnvironment {
    found: bool,
    platform: String,
    executable_path: Option<String>,
    version: Option<String>,
    config_path: String,
    config_exists: bool,
    config_valid: bool,
    detail: String,
    platform_ui: WorkBuddyPlatformUi,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkBuddyPlatformUi {
    executable_picker_title: String,
    executable_filter_name: String,
    executable_extensions: Vec<String>,
    ca_import_consequences: Vec<String>,
}

fn platform_ui(platform: &str) -> WorkBuddyPlatformUi {
    let shared = "该 CA 会同时加入应用信任库 —— 此后全部托管 Claude 环境都会信任它签发的任意证书。";
    let final_note = "请只导入公司网关管理员提供的证书。";
    match platform {
        "macos" => WorkBuddyPlatformUi {
            executable_picker_title: "选择 WorkBuddy.app".into(),
            executable_filter_name: "WorkBuddy 应用程序".into(),
            executable_extensions: vec!["app".into()],
            ca_import_consequences: vec![
                shared.into(),
                "同时同步到 WorkBuddy.app 内置 CLI 的证书文件。".into(),
                "不会修改 macOS 系统钥匙串，也不影响系统层面的信任设置。".into(),
                "WorkBuddy 更新后，管理中心会在下次启动时自动补写。".into(),
                final_note.into(),
            ],
        },
        "windows" => WorkBuddyPlatformUi {
            executable_picker_title: "选择 WorkBuddy.exe".into(),
            executable_filter_name: "WorkBuddy 应用程序".into(),
            executable_extensions: vec!["exe".into()],
            ca_import_consequences: vec![
                shared.into(),
                "还会加入当前 Windows 用户的受信任根证书库；不会改动其他用户账户。".into(),
                "并写入 WorkBuddy 安装目录下的共享 ca.pem。".into(),
                "WorkBuddy 更新后，管理中心会在下次启动时自动补写。".into(),
                final_note.into(),
            ],
        },
        _ => WorkBuddyPlatformUi {
            executable_picker_title: "选择 WorkBuddy 可执行文件".into(),
            executable_filter_name: "WorkBuddy 应用程序".into(),
            executable_extensions: Vec::new(),
            ca_import_consequences: vec![shared.into(), final_note.into()],
        },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyModel {
    id: String,
    name: String,
    vendor: String,
    url: String,
    has_api_key: bool,
    max_input_tokens: u64,
    max_output_tokens: u64,
    supports_tool_call: bool,
    supports_images: bool,
    supports_reasoning: bool,
    use_custom_protocol: bool,
    visible: bool,
    uses_global_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyGatewayConfig {
    url: String,
    has_api_key: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyOrganization {
    id: String,
    name: String,
    #[serde(default)]
    model_prefix: String,
    url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    selected_models: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyOrganizationState {
    id: String,
    name: String,
    model_prefix: String,
    url: String,
    selected_models: Vec<String>,
    has_api_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyState {
    environment: WorkBuddyEnvironment,
    gateway: WorkBuddyGatewayConfig,
    organizations: Vec<WorkBuddyOrganizationState>,
    models: Vec<WorkBuddyModel>,
    /// `models.json` 的修订号
    revision: String,
    /// `cc-manager-gateway.json` 的修订号
    gateway_revision: String,
    /// `cc-manager-organizations.json` 的修订号
    organizations_revision: String,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkBuddyOrganizationRequest {
    id: Option<String>,
    /// 前端拿到的 `organizationsRevision`。与 model 写入同级保护：
    /// 不一致说明文件被别的程序改过，拒绝覆盖而不是静默 last-writer-wins。
    expected_revision: String,
    expected_models_revision: String,
    name: String,
    #[serde(default)]
    model_prefix: String,
    url: String,
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyWorkBuddyOrganizationModelsRequest {
    organization_id: String,
    models: Vec<String>,
    expected_organizations_revision: String,
    expected_models_revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkBuddyOrganizationRequest {
    id: String,
    expected_organizations_revision: String,
    expected_models_revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyModelInput {
    id: String,
    name: String,
    vendor: String,
    url: String,
    api_key: Option<String>,
    max_input_tokens: u64,
    max_output_tokens: u64,
    supports_tool_call: bool,
    supports_images: bool,
    supports_reasoning: bool,
    use_custom_protocol: bool,
    visible: bool,
    use_global_key: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkBuddyGatewayRequest {
    url: String,
    api_key: Option<String>,
    /// 同 organization：防静默覆盖别的程序写的内容
    expected_revision: String,
    expected_models_revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkBuddyModelRequest {
    previous_id: Option<String>,
    expected_revision: String,
    model: WorkBuddyModelInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteWorkBuddyModelRequest {
    id: String,
    expected_revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestWorkBuddyModelRequest {
    id: String,
    url: String,
    api_key: Option<String>,
    use_custom_protocol: bool,
    use_global_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyTestResult {
    ok: bool,
    status_code: u16,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyCertificateStatus {
    state: String,
    detail: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkBuddyModelsRequest {
    id: Option<String>,
    url: String,
    api_key: Option<String>,
}

/// 安装目录写入失败时给出说明，成功时给出正常文案。
fn bundle_note_or(note: &str, target: &Path) -> String {
    if note.is_empty() {
        format!("CA 已同步到 WorkBuddy CLI（{}）。", target.display())
    } else {
        note.to_string()
    }
}

fn config_dir() -> PathBuf {
    std::env::var_os("WORKBUDDY_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("CODEBUDDY_CONFIG_DIR").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::home().join(".workbuddy"))
}

fn models_path() -> PathBuf {
    config_dir().join("models.json")
}

fn gateway_path() -> PathBuf {
    config_dir().join("cc-manager-gateway.json")
}

fn organizations_path() -> PathBuf {
    config_dir().join("cc-manager-organizations.json")
}

fn installation_path() -> PathBuf {
    config_dir().join("cc-manager-installation.json")
}

/// 原子替换文本文件：同目录临时文件 → rename。
///
/// WorkBuddy 的 `ca.pem` 是**多方共用**的（本应用合并进去、用户可能也手工放过证书、
/// 其它工具也可能维护它）。`fs::write` 会先截断再写，断电 / 磁盘满 / 写入失败
/// 会把别人的证书一起毁掉。rename 在同一文件系统内是原子的，要么旧内容要么新内容。
fn write_text_atomic(path: &Path, text: &str) -> Result<(), String> {
    // 临时名必须唯一：固定的 .ccm-ca.tmp 会让并发的导入/清理互相覆盖
    let tmp = path.with_extension(format!("ccm-ca-{}.tmp", crate::sync::unique_token()));
    fs::write(&tmp, text).map_err(|error| format!("写入临时证书失败：{error}"))?;
    fs::rename(&tmp, path).map_err(|error| {
        // rename 失败时清理临时文件；**不要**去动目标文件
        let _ = fs::remove_file(&tmp);
        format!("替换证书文件失败：{error}")
    })
}

/// 把 PEM 文本切成证书块，去掉空白后规范化 —— 便于跨来源比对同一张证书。
fn pem_blocks(text: &str) -> Vec<String> {
    let normalized = text.replace(['\r', '\n'], "");
    let mut blocks = vec![];
    let mut rest = normalized.as_str();
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    while let Some(start) = rest.find(BEGIN) {
        let Some(end) = rest[start..].find(END) else {
            break;
        };
        let stop = start + end + END.len();
        blocks.push(rest[start..stop].to_string());
        rest = &rest[stop..];
    }
    blocks
}

/// 从 WorkBuddy CLI 的 `ca.pem` 里**只移除本应用添加的证书**，保留其它来源的条目。
///
/// 清理 CA 时必须走这里：`sync_workbuddy_ca_bundle` 是**合并**写入的，
/// 直接删文件会把用户/其它工具放的证书一起删掉。
fn remove_managed_pem_blocks(existing: &str, managed: &str) -> String {
    let managed_blocks = pem_blocks(managed);
    if managed_blocks.is_empty() {
        return existing.to_string();
    }
    let mut out = existing.to_string();
    for certificate in pem_certificates(existing) {
        if managed_blocks.contains(&normalize_pem(&certificate)) {
            out = out.replacen(&certificate, "", 1);
        }
    }
    if out.trim().is_empty() {
        String::new()
    } else {
        out
    }
}

// ---------------- CA 所有权清单（审查 F3） ----------------
//
// 撤销必须只撤销**本应用建立的**信任。原先只按 PEM 内容比对：
// 如果同一张证书在导入前就已经存在于 WorkBuddy 的 ca.pem（用户手工放的）
// 或 Windows 当前用户 Root 库（IT 推送的），清理时会把它一并撤掉 ——
// 那是**撤销了不属于我们的信任**，可能直接打断机器原有的信任链。
//
// 所以导入时留下凭据：指纹 + **每个目标在导入前是否已存在**。撤销只处理确由本应用新增的。

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedCert {
    /// 规范化 PEM 块的 sha256（用 content_hash，带算法前缀）
    pub fingerprint: String,
    /// 该证书的 PEM 块，撤销时按它匹配
    pub pem: String,
    /// 导入前 WorkBuddy 的 ca.pem 里**没有**它 ⇒ 是本应用加进去的
    pub added_to_workbuddy: bool,
    /// 导入前 Windows 当前用户 Root 库里**没有**它 ⇒ 是本应用加进去的。
    /// `None` = **无法判定**（查询失败）。无法判定一律按"不是我们加的"处理：
    /// 漏撤销只是留下信任，误撤销会破坏别人的信任链 —— 两者不对称。
    pub added_to_root_store: Option<bool>,
    /// epoch 秒
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct ManagedCerts {
    certificates: Vec<ManagedCert>,
}

/// "导入前是否已存在" → "是否由本应用新增"。
///
/// 抽出来是因为这两个语义**方向相反**，极易写反：上一版直接把 `root_store_contains`
/// 的结果（"已存在"）赋给了 `added_to_root_store`（"新增"）——
/// 于是别人装的证书被记成我们装的（清理时误删），我们装的被记成 false（清理时反而留下）。
fn ownership_from_presence(exists: Option<bool>) -> Option<bool> {
    // None = 查询失败 ⇒ 保持"无法判定"，不能猜
    exists.map(|already_there| !already_there)
}

/// 那张证书是否**已在**当前 Windows 用户的根证书库里。
/// `None` = 查询失败/无法判定 —— 调用方按"不是我们加的"处理（保守）。
#[cfg(target_os = "windows")]
fn root_store_contains(pem: &str) -> Option<bool> {
    access_root_certificate(pem, false).ok()
}

/// 按证书指纹精确查询/删除。certutil 的 CertId 参数不接受 PEM 文件路径。
/// 证书经标准输入传递；查询故障与“证书不存在”严格区分。
#[cfg(target_os = "windows")]
fn access_root_certificate(pem: &str, remove: bool) -> Result<bool, String> {
    use std::io::Write;
    use std::process::Stdio;
    let operation = if remove {
        "$store.RemoveRange($matches)"
    } else {
        ""
    };
    let mode = if remove { "ReadWrite" } else { "ReadOnly" };
    let script = format!(
        r#"
$ErrorActionPreference='Stop'
$pem=[Console]::In.ReadToEnd()
$body=$pem -replace '-----BEGIN CERTIFICATE-----|-----END CERTIFICATE-----|\s',''
$cert=[System.Security.Cryptography.X509Certificates.X509Certificate2]::new([Convert]::FromBase64String($body))
$store=[System.Security.Cryptography.X509Certificates.X509Store]::new('Root','CurrentUser')
try {{
  $store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::{mode})
  $matches=$store.Certificates.Find([System.Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint,$cert.Thumbprint,$false)
  if ($matches.Count -gt 0) {{ {operation}; [Console]::Out.Write('present') }} else {{ [Console]::Out.Write('absent') }}
}} finally {{ $store.Close(); $cert.Dispose() }}
"#
    );
    let mut child = crate::ps_command()
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("无法打开证书输入")?
        .write_all(pem.as_bytes())
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Windows 用户证书库操作失败，未确认结果".into());
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "present" => Ok(true),
        "absent" => Ok(false),
        _ => Err("证书库返回不可识别的结果".into()),
    }
}

fn managed_certs_path() -> PathBuf {
    crate::cfg_dir().join("managed-certs.json")
}

fn read_managed_certs() -> Result<ManagedCerts, String> {
    let text = crate::read_optional_text(&managed_certs_path())?;
    if text.is_empty() && !managed_certs_path().exists() {
        return Ok(ManagedCerts::default());
    }
    serde_json::from_str(&text).map_err(|e| format!("CA 所有权记录损坏，已停止修改：{e}"))
}

/// 清理成功后**移除已处理条目**。
/// 不缩减的话，清完仍留着旧记录；下次再导入同一张证书时，
/// `record_managed_certs` 会保留「最早那次」的所有权判定 —— 而那个判定可能已经过时。
pub(crate) fn prune_managed_certs(cleared_pem: &str) -> Result<(), String> {
    let cleared = pem_blocks(cleared_pem);
    if cleared.is_empty() {
        return Ok(());
    }
    let mut manifest = read_managed_certs()?;
    let before = manifest.certificates.len();
    manifest
        .certificates
        .retain(|cert| !cleared.contains(&cert.pem));
    if manifest.certificates.len() == before {
        return Ok(());
    }
    fs::create_dir_all(crate::cfg_dir()).map_err(|e| e.to_string())?;
    crate::sync::write_json_atomic(
        &managed_certs_path(),
        &serde_json::to_value(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// 按指纹合并写入（同一张证书重复导入不产生重复条目）
fn record_managed_certs(new_entries: Vec<ManagedCert>) -> Result<(), String> {
    if new_entries.is_empty() {
        return Ok(());
    }
    let mut manifest = read_managed_certs()?;
    for entry in new_entries {
        match manifest
            .certificates
            .iter_mut()
            .find(|existing| existing.fingerprint == entry.fingerprint)
        {
            // 重复导入：保留"最早那次"的所有权判定（以第一次为准）
            Some(existing) => {
                existing.at = existing.at.min(entry.at);
                // 启动时只合并 WorkBuddy bundle，尚未操作 Root 库；导入时补齐归属。
                if existing.added_to_root_store.is_none() {
                    existing.added_to_root_store = entry.added_to_root_store;
                }
            }
            None => manifest.certificates.push(entry),
        }
    }
    fs::create_dir_all(crate::cfg_dir()).map_err(|e| e.to_string())?;
    crate::sync::write_json_atomic(
        &managed_certs_path(),
        &serde_json::to_value(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// 撤销本应用建立的 CA 信任。
///
/// `managed_pem` 是清空前 `~/.cc-manager/ca-cert.pem` 的内容 —— 即本应用曾
/// **合并进 WorkBuddy CLI**、并在 Windows 上**加入当前用户 Root 库**的那批证书。
///
/// 返回逐条结果（成功与失败都列出）。调用方必须据此**如实**汇报，
/// **不得**在部分失败时宣称"已清空所有 CA 证书" —— 那正是本条要修的缺陷。
pub(crate) fn revoke_managed_ca(managed_pem: &str) -> Vec<String> {
    let mut notes = vec![];
    let mut manifest = match read_managed_certs() {
        Ok(manifest) => manifest,
        Err(e) => return vec![format!("❌ {e}")],
    };
    manifest.certificates = selected_managed_certs(&manifest, managed_pem);

    if manifest.certificates.is_empty() {
        // **没有所有权记录就不撤销。**
        // 撤销不成只是"没做成"；而误撤销会打断机器原有的信任链 —— 两者不对称。
        // 这条路径主要出现在"用旧版本导入过证书"的存量用户上，给出可操作的补救说明。
        // 注意这里用 **⚠️ 而不是 ❌**：没有记录只是"外部撤销这一步做不了"，
        // 本地 bundle 是应用独占管理的，必须照常清掉。
        // 上一版标成 ❌ ⇒ 只用顶栏导入过的用户**永远清不掉**（功能回归，见审查）。
        if !pem_blocks(managed_pem).is_empty() {
            notes.push(
                "⚠️ 没有这些证书的所有权记录（多半是旧版本导入的），因此「未撤销外部信任」（WorkBuddy ca.pem / Windows 用户根证书库）。
                 本地信任库已照常清空。如需撤销外部信任：先重新导入一次这些证书（会建立所有权记录），再执行清空；
                 或手工检查 WorkBuddy 的 ca.pem 与 Windows 用户「受信任根证书」库。"
                    .into(),
            );
        }
        return notes;
    }

    // ① WorkBuddy 的合并 bundle：只摘掉**确认由本应用加入**的那些
    let owned_in_bundle: String = manifest
        .certificates
        .iter()
        .filter(|cert| cert.added_to_workbuddy)
        .map(|cert| {
            format!(
                "{}
",
                cert.pem
            )
        })
        .collect();
    if owned_in_bundle.is_empty() {
        notes.push("所有权记录中没有本应用加入 WorkBuddy bundle 的证书。".into());
    } else {
        match find_executable() {
            Some(executable) => {
                match workbuddy_ca_path_for(executable.as_path()).filter(|path| path.is_file()) {
                    Some(target) => match fs::read_to_string(&target) {
                        Ok(existing) => {
                            let left = remove_managed_pem_blocks(&existing, &owned_in_bundle);
                            if left == existing {
                                notes
                                    .push(format!("{} 里没有本应用添加的证书。", target.display()));
                            } else if let Err(error) = write_text_atomic(&target, &left) {
                                notes.push(format!(
                                    "❌ 未能从 {} 移除证书：{error}",
                                    target.display()
                                ));
                            } else {
                                notes.push(format!(
                                    "已从 {} 移除本应用添加的证书。",
                                    target.display()
                                ));
                            }
                        }
                        Err(error) => {
                            notes.push(format!("❌ 读取 {} 失败：{error}", target.display()))
                        }
                    },
                    None => notes.push("未找到 WorkBuddy CLI 目录，跳过 CLI 证书清理。".into()),
                }
            }
            None => notes.push("未检测到 WorkBuddy 安装，跳过 CLI 证书清理。".into()),
        }
    }

    // ② Windows 用户 Root 库：只删**确认由本应用加入**的
    #[cfg(target_os = "windows")]
    {
        let owned: Vec<&ManagedCert> = manifest
            .certificates
            .iter()
            .filter(|cert| cert.added_to_root_store == Some(true))
            .collect();
        let skipped = manifest
            .certificates
            .iter()
            .filter(|cert| cert.added_to_root_store.is_none())
            .count();
        if skipped > 0 {
            notes.push(format!(
                "有 {skipped} 张证书在导入时无法判定是否已在根证书库中，「未撤销」（保守处理：漏撤销只留下信任，误撤销会打断别人的信任链）。"
            ));
        }
        if owned.is_empty() {
            notes.push("没有确认由本应用加入 Windows 用户根证书库的证书。".into());
        } else {
            let mut removed = 0usize;
            let mut failures = vec![];
            for cert in owned {
                match access_root_certificate(&cert.pem, true) {
                    Ok(true) => removed += 1,
                    Ok(false) => {} // 已被移除，重复清理成功。
                    Err(error) => failures.push(error),
                }
            }
            if removed > 0 {
                notes.push(format!(
                    "已从当前 Windows 用户的受信任根证书库移除 {removed} 张证书。"
                ));
            }
            for failure in failures {
                notes.push(format!("❌ 未能从 Windows 用户根证书库移除：{failure}"));
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        notes.push(format!(
            "macOS 未向系统钥匙串写入证书（所有权记录 {} 条），无需撤销系统信任。",
            manifest.certificates.len()
        ));
    }

    notes
}

/// 含**明文密钥**的 WorkBuddy 配置文件，供权限检查使用。
/// 注意：这些文件在 Windows 与 macOS 上都是未加密的 JSON（`workbuddy.rs` 内
/// 全部 write 路径都走 `write_document`），所以它们才是权限检查的主要对象，
/// 而不是 macOS 钥匙串 / Windows DPAPI 所覆盖的网关 Key。
///
/// **必须连带写入 `write_document` 派生出的所有产物**：backup / previous 里存的
/// 同样是含明文 apiKey 的旧内容，而且是历史版本（可能在本轮收紧权限之前生成）
/// 留下的，权限很可能比主文件松。只查主文件会谎报"均仅限本人读取"。
pub(crate) fn credential_file_paths() -> Vec<PathBuf> {
    let mut paths = vec![];
    for base in [gateway_path(), organizations_path(), models_path()] {
        paths.push(base.clone());
        paths.push(base.with_extension("cc-manager.backup.json"));
        paths.push(base.with_extension("cc-manager.previous.json"));
        // 兼容旧版本固定临时名；新版本使用唯一临时名，并把可能因崩溃遗留的文件
        // 从目录中枚举出来，避免权限检查漏报含明文 apiKey 的残留文件。
        paths.push(base.with_extension("cc-manager.tmp"));
        let prefix = format!(
            "{}.cc-manager.",
            base.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        );
        if let Some(parent) = base.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                paths.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"))
                }));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn read_organizations() -> Result<Vec<WorkBuddyOrganization>, String> {
    let path = organizations_path();
    if !path.exists() {
        let legacy = read_gateway_config()?;
        if legacy.api_key.is_empty() {
            return Ok(Vec::new());
        }
        let selected_models = read_document(&models_path())
            .ok()
            .and_then(|document| document.get("models").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|model| {
                model.get("url").and_then(Value::as_str) == Some(legacy.url.as_str())
                    && model.get("apiKey").and_then(Value::as_str) == Some(legacy.api_key.as_str())
            })
            .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        return Ok(vec![WorkBuddyOrganization {
            id: "organization-default".into(),
            name: "公司网关".into(),
            model_prefix: String::new(),
            url: legacy.url,
            api_key: legacy.api_key,
            selected_models,
        }]);
    }
    let value: Value = serde_json::from_str(
        &fs::read_to_string(&path)
            .map_err(|error| format!("读取 WorkBuddy 组织配置失败：{error}"))?,
    )
    .map_err(|error| format!("WorkBuddy 组织配置不是有效 JSON：{error}"))?;
    let needs_model_prefix_migration = organization_config_needs_model_prefix(&value);
    let organizations: Vec<WorkBuddyOrganization> = serde_json::from_value(value)
        .map_err(|error| format!("WorkBuddy 组织配置不是有效 JSON：{error}"))?;
    if needs_model_prefix_migration {
        write_organizations(&organizations)?;
    }
    Ok(organizations)
}

fn organization_config_needs_model_prefix(value: &Value) -> bool {
    value
        .as_array()
        .map(|items| {
            items.iter().any(|item| {
                item.as_object()
                    .map(|raw| !raw.contains_key("modelPrefix"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn validate_organization(
    name: &mut String,
    model_prefix: &mut String,
    url: &mut String,
) -> Result<(), String> {
    *name = name.trim().to_string();
    *model_prefix = model_prefix.trim().to_string();
    *url = url.trim().trim_end_matches('/').to_string();
    if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err("组织名称不能为空，且最多 80 个字符。".into());
    }
    if model_prefix.len() > 80 || model_prefix.chars().any(char::is_control) {
        return Err("模型前缀最多 80 个字符，且不能包含控制字符。".into());
    }
    let parsed = url::Url::parse(url).map_err(|_| "网关地址不是有效 URL。")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("网关地址必须以 http:// 或 https:// 开头。".into());
    }
    // 员工 Key 走 Bearer，远程 http 下就是明文过网 —— 只放行 loopback
    if !crate::transport_is_loopback_or_secure(url) {
        return Err(crate::PLAINTEXT_TRANSPORT_REJECTED.into());
    }
    Ok(())
}

fn organization_by_id(id: &str) -> Result<WorkBuddyOrganization, String> {
    read_organizations()?
        .into_iter()
        .find(|organization| organization.id == id)
        .ok_or_else(|| "未找到该组织配置，请刷新页面后重试。".into())
}

fn model_belongs_to_organization(model: &Value, organization: &WorkBuddyOrganization) -> bool {
    match model.get(ORGANIZATION_OWNER_FIELD).and_then(Value::as_str) {
        Some(owner) => owner == organization.id,
        None => {
            model.get("url").and_then(Value::as_str) == Some(organization.url.as_str())
                && model.get("apiKey").and_then(Value::as_str)
                    == Some(organization.api_key.as_str())
        }
    }
}

fn repair_managed_models() -> Result<(), String> {
    let path = models_path();
    if !path.exists() {
        return Ok(());
    }
    let organizations = read_organizations()?;
    if organizations.is_empty() {
        return Ok(());
    }
    let mut document = read_document(&path)?;
    let Some(models) = document.get_mut("models").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let mut changed = false;
    for model in models {
        let Some(owner) = model.get(ORGANIZATION_OWNER_FIELD).and_then(Value::as_str) else {
            continue;
        };
        let Some(organization) = organizations.iter().find(|item| item.id == owner) else {
            continue;
        };
        let Some(model_id) = model.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        let expected_url = openai_api_base_url(&organization.url)?;
        // WorkBuddy injects `name` into the system prompt as the model identity.
        // It must match the real gateway model id; an organization label here would
        // make the model incorrectly identify itself as that organization/provider.
        let (expected_name, expected_vendor) =
            managed_model_identity(&model_id, &organization.model_prefix);
        if let Some(raw) = model.as_object_mut() {
            if raw.get("url").and_then(Value::as_str) != Some(expected_url.as_str()) {
                raw.insert("url".into(), Value::String(expected_url));
                changed = true;
            }
            if raw.get("name").and_then(Value::as_str) != Some(expected_name.as_str()) {
                raw.insert("name".into(), Value::String(expected_name));
                changed = true;
            }
            if raw.get("vendor").and_then(Value::as_str) != Some(expected_vendor.as_str()) {
                raw.insert("vendor".into(), Value::String(expected_vendor));
                changed = true;
            }
        }
    }
    if changed {
        write_document(&path, &document)?;
    }
    Ok(())
}

fn managed_model_identity(model_id: &str, model_prefix: &str) -> (String, String) {
    let prefix = model_prefix.trim();
    let name = if prefix.is_empty() {
        model_id.to_string()
    } else {
        prefix.to_string()
    };
    (name, "user".to_string())
}

fn write_organizations(organizations: &[WorkBuddyOrganization]) -> Result<(), String> {
    let value = serde_json::to_value(organizations)
        .map_err(|error| format!("序列化 WorkBuddy 组织配置失败：{error}"))?;
    write_document(&organizations_path(), &value)
}

fn new_organization_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("organization-{millis}")
}

#[derive(Clone, Default)]
struct StoredGatewayConfig {
    url: String,
    api_key: String,
}

fn read_gateway_config() -> Result<StoredGatewayConfig, String> {
    let path = gateway_path();
    if !path.exists() {
        return Ok(StoredGatewayConfig {
            url: String::new(),
            api_key: String::new(),
        });
    }
    let value: Value = serde_json::from_str(
        &fs::read_to_string(&path)
            .map_err(|error| format!("读取 WorkBuddy 全局配置失败：{error}"))?,
    )
    .map_err(|error| format!("WorkBuddy 全局配置不是有效 JSON：{error}"))?;
    Ok(StoredGatewayConfig {
        url: value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        api_key: value
            .get("apiKey")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn write_gateway_config(config: &StoredGatewayConfig) -> Result<(), String> {
    let path = gateway_path();
    let document = serde_json::json!({ "url": config.url, "apiKey": config.api_key });
    write_document(&path, &document)
}

fn revision(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing".into(),
        Err(_) => "unreadable".into(),
    }
}

fn read_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("读取 WorkBuddy 模型配置失败：{error}"))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("WorkBuddy models.json 不是有效 JSON：{error}"))?;
    // WorkBuddy 升级后会把"无自定义模型"的 models.json 重置成顶层空数组 `[]`，
    // 它自己读得动（日志里 "Loaded custom models config" 成功）。我们以前只认对象
    // `{}`，于是把这个合法的空配置误判成"已损坏"、弹红叉、禁止保存。
    // 空数组语义上等价于空对象（无模型），归一化后放行；再由本 app 首次保存写回对象结构。
    // 非空数组仍属未知结构（历史上模型始终存在对象的 models 字段里），保持报错。
    if value.as_array().is_some_and(|items| items.is_empty()) {
        return Ok(Value::Object(Map::new()));
    }
    if !value.is_object() {
        return Err("WorkBuddy models.json 顶层必须是 JSON 对象。".into());
    }
    Ok(value)
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn bool_field(value: &Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn number_field(value: &Value, key: &str, default: u64) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn parse_models(document: &Value, gateway: &StoredGatewayConfig) -> Vec<WorkBuddyModel> {
    let available = document
        .get("availableModels")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>());
    let mut models = document
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let id = string_field(value, "id");
            if id.is_empty() {
                return None;
            }
            Some(WorkBuddyModel {
                name: {
                    let name = string_field(value, "name");
                    if name.is_empty() {
                        id.clone()
                    } else {
                        name
                    }
                },
                vendor: string_field(value, "vendor"),
                url: string_field(value, "url"),
                has_api_key: !string_field(value, "apiKey").is_empty(),
                max_input_tokens: number_field(value, "maxInputTokens", 128_000),
                max_output_tokens: number_field(value, "maxOutputTokens", 8_192),
                supports_tool_call: bool_field(value, "supportsToolCall", true),
                supports_images: bool_field(value, "supportsImages", false),
                supports_reasoning: bool_field(value, "supportsReasoning", false),
                use_custom_protocol: bool_field(value, "useCustomProtocol", true),
                visible: available
                    .as_ref()
                    .map(|items| items.contains(&id.as_str()))
                    .unwrap_or(true),
                uses_global_key: !gateway.api_key.is_empty()
                    && string_field(value, "url") == gateway.url
                    && string_field(value, "apiKey") == gateway.api_key,
                id,
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    models
}

fn common_executable_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(target_os = "windows") {
        for root in [
            std::env::var_os("LOCALAPPDATA"),
            std::env::var_os("ProgramFiles"),
            std::env::var_os("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        {
            let root = PathBuf::from(root);
            candidates.push(root.join("Programs/WorkBuddy/WorkBuddy.exe"));
            candidates.push(root.join("WorkBuddy/WorkBuddy.exe"));
        }
    } else if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/Applications/WorkBuddy.app"));
        candidates.push(crate::home().join("Applications/WorkBuddy.app"));
    }
    candidates
}

fn clean_executable_path(value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"');
    let value = value.strip_suffix(",0").unwrap_or(value).trim_matches('"');
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// 生产路径上只有 macOS 会调用它（进程命令行解析出 .app 路径）；
/// 其它平台只有测试用到，所以在那里显式允许 dead_code，
/// 而不是删掉它 —— 删了 macOS 就编译不过。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_app_from_process_command(command: &str) -> Option<PathBuf> {
    let marker = "WorkBuddy.app/Contents/MacOS/";
    let marker_start = command.find(marker)?;
    let app_end = marker_start + "WorkBuddy.app".len();
    let prefix = command[..app_end].trim().trim_matches('"');
    let path_start = prefix.find('/')?;
    Some(PathBuf::from(&prefix[path_start..]))
}

#[cfg(target_os = "windows")]
fn powershell_path(script: &str) -> Option<PathBuf> {
    let mut command = Command::new("powershell");
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NO_WINDOW);
    let script = format!(
        "$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new(); {script}"
    );
    let output = command
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
        .and_then(|value| clean_executable_path(&value))
}

#[cfg(target_os = "windows")]
fn running_executable() -> Option<PathBuf> {
    powershell_path(
        "Get-Process -Name WorkBuddy -ErrorAction SilentlyContinue | Where-Object { $_.Path } | Select-Object -First 1 -ExpandProperty Path",
    )
}

#[cfg(not(target_os = "windows"))]
fn running_executable() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ps")
            .args(["-axo", "command="])
            .output()
            .ok()?;
        return String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(macos_app_from_process_command);
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn saved_executable() -> Option<PathBuf> {
    let path = installation_path();
    let value: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    value
        .get("executablePath")
        .and_then(Value::as_str)
        .and_then(clean_executable_path)
}

fn save_executable(executable: &Path) -> Result<(), String> {
    write_document(
        &installation_path(),
        &serde_json::json!({ "executablePath": executable.display().to_string() }),
    )
}

#[cfg(target_os = "windows")]
fn registry_executable() -> Option<PathBuf> {
    let script = r#"Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -match '^WorkBuddy' } | Select-Object -First 1 -ExpandProperty DisplayIcon"#;
    powershell_path(script)
}

#[cfg(not(target_os = "windows"))]
fn registry_executable() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "windows")]
fn shortcut_executable() -> Option<PathBuf> {
    powershell_path(
        r#"$roots = @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('CommonDesktopDirectory'), [Environment]::GetFolderPath('StartMenu'), [Environment]::GetFolderPath('CommonStartMenu')) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }; $shell = New-Object -ComObject WScript.Shell; Get-ChildItem -Path $roots -Filter '*.lnk' -File -Recurse -ErrorAction SilentlyContinue | ForEach-Object { $target = $shell.CreateShortcut($_.FullName).TargetPath; if ([IO.Path]::GetFileName($target) -ieq 'WorkBuddy.exe') { $target } } | Select-Object -First 1"#,
    )
}

#[cfg(not(target_os = "windows"))]
fn shortcut_executable() -> Option<PathBuf> {
    None
}

fn is_workbuddy_executable(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    if cfg!(target_os = "windows") {
        let is_named_workbuddy = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("WorkBuddy.exe"));
        return is_named_workbuddy
            && workbuddy_cli_dir_for(path).is_some_and(|cli| cli.join("product.json").exists());
    }
    if cfg!(target_os = "macos") {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("WorkBuddy.app"))
            && product_json_for(path).is_some_and(|product| product.exists());
    }
    false
}

fn find_executable() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if std::env::var_os("CC_MANAGER_TEST_HOME").is_some() {
        return saved_executable().filter(|path| is_workbuddy_executable(path));
    }
    saved_executable()
        .filter(|path| is_workbuddy_executable(path))
        .or_else(|| running_executable().filter(|path| is_workbuddy_executable(path)))
        .or_else(|| registry_executable().filter(|path| is_workbuddy_executable(path)))
        .or_else(|| shortcut_executable().filter(|path| is_workbuddy_executable(path)))
        .or_else(|| {
            common_executable_candidates()
                .into_iter()
                .find(|path| is_workbuddy_executable(path))
        })
}

#[derive(Clone, Copy)]
enum WorkBuddyPlatform {
    Windows,
    Macos,
}

fn current_workbuddy_platform() -> Option<WorkBuddyPlatform> {
    if cfg!(target_os = "windows") {
        Some(WorkBuddyPlatform::Windows)
    } else if cfg!(target_os = "macos") {
        Some(WorkBuddyPlatform::Macos)
    } else {
        None
    }
}

fn workbuddy_cli_dir_for_platform(
    executable: &Path,
    platform: WorkBuddyPlatform,
) -> Option<PathBuf> {
    match platform {
        WorkBuddyPlatform::Windows => {
            // This branch is also exercised on macOS CI. `Path::parent()` follows
            // the host path syntax, so it cannot split a Windows path there.
            let executable = executable.to_string_lossy();
            let directory_end = executable.rfind(['\\', '/'])?;
            let directory = &executable[..directory_end];
            let separator = if directory.contains('\\') { "\\" } else { "/" };
            Some(PathBuf::from(format!(
                "{directory}{separator}resources{separator}app.asar.unpacked{separator}cli"
            )))
        }
        WorkBuddyPlatform::Macos => {
            Some(executable.join("Contents/Resources/app.asar.unpacked/cli"))
        }
    }
}

fn workbuddy_cli_dir_for(executable: &Path) -> Option<PathBuf> {
    workbuddy_cli_dir_for_platform(executable, current_workbuddy_platform()?)
}

fn product_json_for(executable: &Path) -> Option<PathBuf> {
    workbuddy_cli_dir_for(executable).map(|dir| dir.join("product.json"))
}

fn workbuddy_ca_path_for(executable: &Path) -> Option<PathBuf> {
    workbuddy_cli_dir_for(executable).map(|dir| dir.join("ca.pem"))
}

/// 目前**只有测试**在用（生产走 `workbuddy_ca_path_for`）。
/// 保留是因为它把"平台 → 路径"的映射单独钉住，测试价值独立于调用点。
#[cfg(test)]
fn workbuddy_ca_path_for_platform(
    executable: &Path,
    platform: WorkBuddyPlatform,
) -> Option<PathBuf> {
    let directory = workbuddy_cli_dir_for_platform(executable, platform)?;
    match platform {
        WorkBuddyPlatform::Windows => Some(PathBuf::from(format!(
            "{}\\ca.pem",
            directory.to_string_lossy()
        ))),
        WorkBuddyPlatform::Macos => Some(directory.join("ca.pem")),
    }
}

fn pem_certificates(text: &str) -> Vec<String> {
    text.split("-----BEGIN CERTIFICATE-----")
        .skip(1)
        .filter_map(|tail| {
            let (body, _) = tail.split_once("-----END CERTIFICATE-----")?;
            Some(format!(
                "-----BEGIN CERTIFICATE-----{}-----END CERTIFICATE-----",
                body
            ))
        })
        .collect()
}

fn normalize_pem(certificate: &str) -> String {
    certificate.replace(['\r', '\n'], "")
}

fn merge_ca_bundle(existing: &str, managed: &str) -> String {
    let mut merged = existing.to_string();
    let mut normalized = normalize_pem(existing);
    for certificate in pem_certificates(managed) {
        let candidate = normalize_pem(&certificate);
        if normalized.contains(&candidate) {
            continue;
        }
        if !merged.is_empty() && !merged.ends_with('\n') {
            merged.push('\n');
        }
        merged.push_str(&certificate);
        merged.push('\n');
        normalized.push_str(&candidate);
    }
    merged
}

fn sync_workbuddy_ca_bundle(executable: &Path, bundle: &Path) -> Result<PathBuf, String> {
    let target = workbuddy_ca_path_for(executable).ok_or("当前平台不支持同步 WorkBuddy CA。")?;
    let parent = target
        .parent()
        .filter(|path| path.is_dir())
        .ok_or_else(|| "未找到 WorkBuddy CLI 目录；请重新检测安装位置。".to_string())?;
    let managed =
        fs::read_to_string(bundle).map_err(|error| format!("读取 CA bundle 失败：{error}"))?;
    let existing = crate::read_optional_text(&target)?;
    let merged = merge_ca_bundle(&existing, &managed);
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ownership = pem_blocks(&managed)
        .into_iter()
        .map(|pem| ManagedCert {
            fingerprint: crate::content_hash(pem.as_bytes()),
            added_to_workbuddy: !pem_blocks(&existing).contains(&pem),
            pem,
            added_to_root_store: None,
            at,
        })
        .collect();
    if merged != existing {
        write_text_atomic(&target, &merged).map_err(|error| {
            format!(
                "写入 WorkBuddy CA 文件失败（{}）：{error}",
                parent.display()
            )
        })?;
    }
    record_managed_certs(ownership)?;
    Ok(target)
}

fn workbuddy_ca_bundle_is_synced(executable: &Path, bundle: &Path) -> bool {
    let Some(target) = workbuddy_ca_path_for(executable) else {
        return false;
    };
    let Ok(managed) = fs::read_to_string(bundle) else {
        return false;
    };
    let Ok(installed) = fs::read_to_string(target) else {
        return false;
    };
    let installed = normalize_pem(&installed);
    let certificates = pem_certificates(&managed);
    !certificates.is_empty()
        && certificates
            .iter()
            .all(|certificate| installed.contains(&normalize_pem(certificate)))
}

fn installed_version(executable: &Path) -> Option<String> {
    let path = product_json_for(executable)?;
    let value: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    value
        .get("genieVersion")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn environment_for(path: &Path, document: Result<&Value, &String>) -> WorkBuddyEnvironment {
    let executable = find_executable();
    let version = executable.as_deref().and_then(installed_version);
    let config_exists = path.exists();
    let config_valid = document.is_ok();
    let detail = if executable.is_none() {
        "未检测到 WorkBuddy 安装；仍可提前保存模型配置，安装后会自动读取。".to_string()
    } else if !config_valid {
        "已检测到 WorkBuddy，但 models.json 已损坏；为避免覆盖，当前禁止保存。".to_string()
    } else if config_exists {
        "WorkBuddy 已就绪，模型配置支持热加载。".to_string()
    } else {
        "WorkBuddy 已就绪；保存第一个模型时会创建 models.json。".to_string()
    };
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "other"
    };
    WorkBuddyEnvironment {
        found: executable.is_some(),
        platform: platform.into(),
        executable_path: executable.as_ref().map(|value| value.display().to_string()),
        version,
        config_path: path.display().to_string(),
        config_exists,
        config_valid,
        detail,
        platform_ui: platform_ui(platform),
    }
}

fn build_state_unlocked() -> WorkBuddyState {
    let path = models_path();
    let repair_warning = repair_managed_models().err();
    let document = read_document(&path);
    let gateway = read_gateway_config().unwrap_or_else(|_| StoredGatewayConfig {
        url: String::new(),
        api_key: String::new(),
    });
    let mut warnings = Vec::new();
    if let Some(error) = repair_warning {
        warnings.push(error);
    }
    let models = match &document {
        Ok(value) => parse_models(value, &gateway),
        Err(error) => {
            warnings.push(error.clone());
            Vec::new()
        }
    };
    let environment = environment_for(&path, document.as_ref());
    let stored_organizations = match read_organizations() {
        Ok(organizations) => organizations,
        Err(error) => {
            warnings.push(error);
            Vec::new()
        }
    };
    let organizations = stored_organizations
        .into_iter()
        .map(|organization| WorkBuddyOrganizationState {
            id: organization.id,
            name: organization.name,
            model_prefix: organization.model_prefix,
            url: organization.url,
            selected_models: organization.selected_models,
            has_api_key: !organization.api_key.is_empty(),
        })
        .collect();
    WorkBuddyState {
        environment,
        gateway: WorkBuddyGatewayConfig {
            url: gateway.url,
            has_api_key: !gateway.api_key.is_empty(),
        },
        organizations,
        models,
        gateway_revision: revision(&gateway_path()),
        organizations_revision: revision(&organizations_path()),
        revision: revision(&path),
        warnings,
    }
}

fn validate_model(model: &mut WorkBuddyModelInput, require_key: bool) -> Result<(), String> {
    model.id = model.id.trim().to_string();
    model.name = model.name.trim().to_string();
    model.vendor = model.vendor.trim().to_string();
    model.url = model.url.trim().to_string();
    model.api_key = model.api_key.take().map(|value| value.trim().to_string());
    if model.id.is_empty() || model.id.len() > 160 || model.id.chars().any(char::is_control) {
        return Err("模型 ID 不能为空、不能包含控制字符，且最长 160 个字符。".into());
    }
    if model.name.is_empty() {
        model.name = model.id.clone();
    }
    if model.vendor.is_empty() {
        model.vendor = "MaaS Gateway".into();
    }
    let parsed = url::Url::parse(&model.url).map_err(|_| "API 地址不是有效 URL。")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("API 地址必须以 http:// 或 https:// 开头。".into());
    }
    // 模型请求同样带 Bearer Key，远程 http 一并拒绝
    if !crate::transport_is_loopback_or_secure(&model.url) {
        return Err(crate::PLAINTEXT_TRANSPORT_REJECTED.into());
    }
    if model.use_custom_protocol && !parsed.path().ends_with("/chat/completions") {
        return Err("启用完整地址直连时，API 地址必须以 /chat/completions 结尾。".into());
    }
    if require_key && model.api_key.as_deref().unwrap_or_default().is_empty() {
        return Err("新建模型必须填写员工 Key。".into());
    }
    if model.max_input_tokens == 0 || model.max_output_tokens == 0 {
        return Err("输入和输出 Token 上限必须大于 0。".into());
    }
    Ok(())
}

fn document_temp_path(path: &Path) -> PathBuf {
    path.with_extension(format!("cc-manager.{}.tmp", crate::sync::unique_token()))
}

fn write_document(path: &Path, document: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or("WorkBuddy 配置目录无效。")?;
    fs::create_dir_all(parent).map_err(|error| format!("创建 WorkBuddy 配置目录失败：{error}"))?;
    let text = serde_json::to_string_pretty(document)
        .map_err(|error| format!("序列化 WorkBuddy 配置失败：{error}"))?;
    // 唯一临时名可避免两个进程/应用实例在落盘前互相截断同一临时文件。
    // 完整 read-modify-write 仍由 WRITE_LOCK 串行化；修订号用于发现外部程序改写。
    let tmp = document_temp_path(path);
    let backup = path.with_extension("cc-manager.backup.json");
    let previous = path.with_extension("cc-manager.previous.json");
    fs::write(&tmp, format!("{text}\n"))
        .map_err(|error| format!("写入 WorkBuddy 临时配置失败：{error}"))?;
    // 每个产物生成后**立刻**收紧权限，不能只收拾最终文件：
    //   - `fs::copy` 会连同源文件的权限位一起复制，源可能是 0644；
    //   - `previous` 来自改名，继承的正是原文件（可能 0644）的权限；
    //   - 这些产物里都含**明文 apiKey**（见 docs/凭证分域清单），漏一个就前功尽弃。
    restrict_credential_file(&tmp)?;
    if path.exists() {
        fs::copy(path, &backup).map_err(|error| format!("备份 WorkBuddy 配置失败：{error}"))?;
        restrict_credential_file(&backup)?;
        if previous.exists() {
            fs::remove_file(&previous).map_err(|error| format!("清理旧临时文件失败：{error}"))?;
        }
        fs::rename(path, &previous)
            .map_err(|error| format!("准备替换 WorkBuddy 配置失败：{error}"))?;
        restrict_credential_file(&previous)?;
    }
    if let Err(error) = fs::rename(&tmp, path) {
        if previous.exists() && !path.exists() {
            let _ = fs::rename(&previous, path);
        }
        let _ = fs::remove_file(&tmp);
        return Err(format!("替换 WorkBuddy 配置失败：{error}"));
    }
    let _ = fs::remove_file(previous);
    restrict_credential_file(path)?;
    Ok(())
}

/// 收紧凭证文件权限：unix 下设 0600（仅本人可读写）。
///
/// Windows 分支不做事：威胁边界只到"同机其他普通用户"，而用户目录的继承 ACL
/// 已经覆盖了这一点（实见 docs/凭证分域清单-2026-09-12.md）。管理员/SYSTEM 不属于
/// 文件权限能可靠防御的范围，所以不做显式 DACL —— 见该文档的威胁边界定义。
fn restrict_credential_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("限制 WorkBuddy 配置文件权限失败：{error}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn set_model_fields(target: &mut Map<String, Value>, model: &WorkBuddyModelInput) {
    target.insert("id".into(), Value::String(model.id.clone()));
    target.insert("name".into(), Value::String(model.name.clone()));
    target.insert("vendor".into(), Value::String(model.vendor.clone()));
    target.insert("url".into(), Value::String(model.url.clone()));
    target.insert("maxInputTokens".into(), Value::from(model.max_input_tokens));
    target.insert(
        "maxOutputTokens".into(),
        Value::from(model.max_output_tokens),
    );
    target.insert(
        "supportsToolCall".into(),
        Value::Bool(model.supports_tool_call),
    );
    target.insert("supportsImages".into(), Value::Bool(model.supports_images));
    target.insert(
        "supportsReasoning".into(),
        Value::Bool(model.supports_reasoning),
    );
    target.insert(
        "useCustomProtocol".into(),
        Value::Bool(model.use_custom_protocol),
    );
    if let Some(key) = model.api_key.as_ref().filter(|value| !value.is_empty()) {
        target.insert("apiKey".into(), Value::String(key.clone()));
    }
}

fn ensure_expected_revision(path: &Path, expected: &str) -> Result<(), String> {
    if revision(path) != expected {
        return Err("WorkBuddy 配置已被其他程序修改。请刷新页面后重试，当前更改尚未写入。".into());
    }
    Ok(())
}

#[tauri::command]
pub fn workbuddy_state() -> WorkBuddyState {
    let _guard = write_guard();
    build_state_unlocked()
}

#[tauri::command]
pub fn set_workbuddy_executable(path: String) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    let expected = if cfg!(target_os = "macos") {
        "WorkBuddy.app"
    } else {
        "WorkBuddy.exe"
    };
    let candidate = clean_executable_path(&path).ok_or_else(|| format!("请选择 {expected}。"))?;
    if !is_workbuddy_executable(&candidate) {
        return Err(format!(
            "所选路径不是有效的 {expected}，或应用中缺少 WorkBuddy CLI/product.json。"
        ));
    }
    let executable = candidate.canonicalize().unwrap_or(candidate);
    // 检测结果**先持久化**：那正是本命令的职责，不该因为后面的 CA 同步失败而回退。
    save_executable(&executable)?;

    // CA 同步写成**可报告的非阻断步骤**：装到 Program Files / /Applications 时非管理员
    // 写不进去，原先把整条命令判失败 —— 但安装位置其实已经保存了（部分提交 + 误导性报错）。
    let mut note = None;
    let certificate = crate::union_ca_bundle_path();
    if certificate.exists() {
        if let Err(error) = sync_workbuddy_ca_bundle(&executable, &certificate) {
            note = Some(format!(
                "已记住 WorkBuddy 安装位置，但未能写入其安装目录的 CA（{error}）。\\
                 这通常是权限不足；本应用与系统层面的信任不受影响，启动 WorkBuddy 时仍会通过环境变量注入 CA。"
            ));
        }
    }
    let mut state = build_state_unlocked();
    if let Some(note) = note {
        state.warnings.push(note);
    }
    Ok(state)
}

#[tauri::command]
pub fn save_workbuddy_gateway(
    mut request: SaveWorkBuddyGatewayRequest,
) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    request.url = request.url.trim().trim_end_matches('/').to_string();
    let parsed = url::Url::parse(&request.url).map_err(|_| "网关地址不是有效 URL。")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("网关地址必须以 http:// 或 https:// 开头。".into());
    }
    // 与 organization / model 同一条规则。**不能因为"前端暂时没有调用点"就省掉**：
    // IPC 命令是可以被直接调用的，后端不能靠前端不可达来维持安全边界。
    if !crate::transport_is_loopback_or_secure(&request.url) {
        return Err(crate::PLAINTEXT_TRANSPORT_REJECTED.into());
    }
    // 与 model 写入同级的并发保护：文件被别的程序改过就拒绝覆盖
    ensure_expected_revision(&gateway_path(), &request.expected_revision)?;
    ensure_expected_revision(&models_path(), &request.expected_models_revision)?;
    let previous = read_gateway_config()?;
    let api_key = request
        .api_key
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| previous.api_key.clone());
    if api_key.is_empty() {
        return Err("首次保存全局配置必须填写员工 Key。".into());
    }
    let next = StoredGatewayConfig {
        url: request.url,
        api_key,
    };

    let models_file = models_path();
    if models_file.exists() && !previous.api_key.is_empty() {
        let mut document = read_document(&models_file)?;
        if let Some(models) = document.get_mut("models").and_then(Value::as_array_mut) {
            let mut changed = false;
            for model in models {
                let uses_previous = model.get("url").and_then(Value::as_str)
                    == Some(previous.url.as_str())
                    && model.get("apiKey").and_then(Value::as_str)
                        == Some(previous.api_key.as_str());
                if uses_previous {
                    if let Some(raw) = model.as_object_mut() {
                        raw.insert("url".into(), Value::String(next.url.clone()));
                        raw.insert("apiKey".into(), Value::String(next.api_key.clone()));
                        changed = true;
                    }
                }
            }
            if changed {
                write_document(&models_file, &document)?;
            }
        }
    }
    write_gateway_config(&next)?;
    Ok(build_state_unlocked())
}

#[tauri::command]
pub fn save_workbuddy_organization(
    mut request: SaveWorkBuddyOrganizationRequest,
) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    validate_organization(
        &mut request.name,
        &mut request.model_prefix,
        &mut request.url,
    )?;
    // 与 model 写入同级的并发保护：文件被别的程序改过就拒绝覆盖
    ensure_expected_revision(&organizations_path(), &request.expected_revision)?;
    ensure_expected_revision(&models_path(), &request.expected_models_revision)?;
    let mut organizations = read_organizations()?;
    let index = request.id.as_deref().and_then(|id| {
        organizations
            .iter()
            .position(|organization| organization.id == id)
    });
    let api_key = request
        .api_key
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| index.map(|position| organizations[position].api_key.clone()))
        .ok_or("首次保存组织时必须填写系统 Key。")?;

    if let Some(position) = index {
        let previous = organizations[position].clone();
        organizations[position].name = request.name;
        organizations[position].model_prefix = request.model_prefix;
        organizations[position].url = request.url;
        organizations[position].api_key = api_key;

        if (previous.url != organizations[position].url
            || previous.api_key != organizations[position].api_key)
            && models_path().exists()
        {
            let mut document = read_document(&models_path())?;
            if let Some(models) = document.get_mut("models").and_then(Value::as_array_mut) {
                for model in models
                    .iter_mut()
                    .filter(|model| model_belongs_to_organization(model, &previous))
                {
                    if let Some(raw) = model.as_object_mut() {
                        raw.insert(
                            "url".into(),
                            Value::String(openai_api_base_url(&organizations[position].url)?),
                        );
                        raw.insert(
                            "apiKey".into(),
                            Value::String(organizations[position].api_key.clone()),
                        );
                        if let Some(model_id) =
                            raw.get("id").and_then(Value::as_str).map(str::to_string)
                        {
                            let (model_name, model_vendor) = managed_model_identity(
                                &model_id,
                                &organizations[position].model_prefix,
                            );
                            raw.insert("name".into(), Value::String(model_name));
                            raw.insert("vendor".into(), Value::String(model_vendor));
                        }
                        raw.insert(
                            ORGANIZATION_OWNER_FIELD.into(),
                            Value::String(organizations[position].id.clone()),
                        );
                    }
                }
            }
            write_document(&models_path(), &document)?;
        }
    } else {
        organizations.push(WorkBuddyOrganization {
            id: new_organization_id(),
            name: request.name,
            model_prefix: request.model_prefix,
            url: request.url,
            api_key,
            selected_models: Vec::new(),
        });
    }
    write_organizations(&organizations)?;
    Ok(build_state_unlocked())
}

#[tauri::command]
pub fn delete_workbuddy_organization(
    request: DeleteWorkBuddyOrganizationRequest,
) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    ensure_expected_revision(
        &organizations_path(),
        &request.expected_organizations_revision,
    )?;
    ensure_expected_revision(&models_path(), &request.expected_models_revision)?;
    let organization = organization_by_id(request.id.trim())?;
    let mut organizations = read_organizations()?;
    if models_path().exists() {
        let mut document = read_document(&models_path())?;
        let removed_ids = document
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|model| model_belongs_to_organization(model, &organization))
            .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
            .collect::<HashSet<_>>();
        if let Some(models) = document.get_mut("models").and_then(Value::as_array_mut) {
            models.retain(|model| !model_belongs_to_organization(model, &organization));
        }
        if let Some(available) = document
            .get_mut("availableModels")
            .and_then(Value::as_array_mut)
        {
            available.retain(|value| {
                value
                    .as_str()
                    .map(|id| !removed_ids.contains(id))
                    .unwrap_or(true)
            });
        }
        write_document(&models_path(), &document)?;
    }
    organizations.retain(|item| item.id != organization.id);
    write_organizations(&organizations)?;
    Ok(build_state_unlocked())
}

#[tauri::command]
pub fn apply_workbuddy_organization_models(
    request: ApplyWorkBuddyOrganizationModelsRequest,
) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    ensure_expected_revision(
        &organizations_path(),
        &request.expected_organizations_revision,
    )?;
    ensure_expected_revision(&models_path(), &request.expected_models_revision)?;
    let mut organizations = read_organizations()?;
    let organization_index = organizations
        .iter()
        .position(|organization| organization.id == request.organization_id)
        .ok_or("未找到该组织配置，请刷新页面后重试。")?;
    let organization = organizations[organization_index].clone();
    let mut selected = request
        .models
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    selected.sort();
    selected.dedup();
    if selected
        .iter()
        .any(|id| id.len() > 160 || id.chars().any(char::is_control))
    {
        return Err("模型 ID 无效。".into());
    }
    let selected_set = selected.iter().cloned().collect::<HashSet<_>>();
    let path = models_path();
    let mut document = read_document(&path)?;
    let root = document
        .as_object_mut()
        .ok_or("WorkBuddy models.json 顶层必须是对象。")?;
    let existing_ids = root
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    let available_is_all = root
        .get("availableModels")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true);
    if available_is_all {
        root.insert(
            "availableModels".into(),
            Value::Array(existing_ids.into_iter().map(Value::String).collect()),
        );
    }
    let models = root
        .entry("models")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("WorkBuddy models.json 的 models 字段必须是数组。")?;

    let owned_ids = models
        .iter()
        .filter(|model| model_belongs_to_organization(model, &organization))
        .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<HashSet<_>>();

    for id in &selected {
        if let Some(existing) = models
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(id.as_str()))
        {
            if !model_belongs_to_organization(existing, &organization) {
                return Err(format!(
                    "模型 {id} 已由其他组织或手动配置占用。WorkBuddy 不支持两个网关使用同一个模型 ID。"
                ));
            }
        }
    }

    models.retain(|model| {
        if !model_belongs_to_organization(model, &organization) {
            return true;
        }
        model
            .get("id")
            .and_then(Value::as_str)
            .map(|id| selected_set.contains(id))
            .unwrap_or(false)
    });
    for id in &selected {
        let mut raw = models
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        raw.insert("id".into(), Value::String(id.clone()));
        let (model_name, model_vendor) = managed_model_identity(id, &organization.model_prefix);
        raw.insert("name".into(), Value::String(model_name));
        raw.insert("vendor".into(), Value::String(model_vendor));
        raw.insert(
            "url".into(),
            Value::String(openai_api_base_url(&organization.url)?),
        );
        raw.insert("apiKey".into(), Value::String(organization.api_key.clone()));
        raw.insert("maxInputTokens".into(), Value::from(128_000));
        raw.insert("maxOutputTokens".into(), Value::from(8_192));
        raw.insert("supportsToolCall".into(), Value::Bool(true));
        raw.insert("supportsImages".into(), Value::Bool(false));
        raw.insert("supportsReasoning".into(), Value::Bool(false));
        raw.insert("useCustomProtocol".into(), Value::Bool(false));
        raw.insert(
            ORGANIZATION_OWNER_FIELD.into(),
            Value::String(organization.id.clone()),
        );
        if let Some(position) = models
            .iter()
            .position(|model| model.get("id").and_then(Value::as_str) == Some(id.as_str()))
        {
            models[position] = Value::Object(raw);
        } else {
            models.push(Value::Object(raw));
        }
    }
    let available = root
        .get_mut("availableModels")
        .and_then(Value::as_array_mut)
        .ok_or("WorkBuddy models.json 的 availableModels 字段必须是数组。")?;
    let previously_selected = organization
        .selected_models
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    available.retain(|value| {
        value
            .as_str()
            .map(|id| {
                !owned_ids.contains(id)
                    && !previously_selected.contains(id)
                    && !selected_set.contains(id)
            })
            .unwrap_or(true)
    });
    available.extend(selected.iter().cloned().map(Value::String));
    write_document(&path, &document)?;

    organizations[organization_index].selected_models = selected;
    write_organizations(&organizations)?;
    Ok(build_state_unlocked())
}

fn validate_ca_certificate(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| format!("读取证书文件失败：{error}"))?;
    if !metadata.is_file() {
        return Err("请选择一个证书文件。".into());
    }
    if metadata.len() > 1024 * 1024 {
        return Err("证书文件不能超过 1 MB。".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("读取证书文件失败：{error}"))?;
    let text = String::from_utf8_lossy(&bytes);
    if !text.contains("-----BEGIN CERTIFICATE-----") || !text.contains("-----END CERTIFICATE-----")
    {
        return Err("请选择管理员提供的 PEM 格式 CA 证书。".into());
    }
    Ok(())
}

#[tauri::command]
pub fn import_workbuddy_ca(path: String) -> Result<String, String> {
    let certificate = PathBuf::from(path.trim());
    validate_ca_certificate(&certificate)?;

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        return Err("当前平台暂不支持从界面导入 WorkBuddy CA 证书。".into());
    }

    crate::import_cert(path)?;
    let executable = find_executable().ok_or("未检测到 WorkBuddy 安装，无法同步 CLI CA。")?;

    // **所有权预检必须在写入之前做**：写进去之后就再也分不清
    // "是本应用加的" 与 "本来就在那里"（审查 F3）。
    let source_pems = pem_blocks(&fs::read_to_string(&certificate).unwrap_or_default());
    let workbuddy_before = workbuddy_ca_path_for(executable.as_path())
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let workbuddy_blocks_before = pem_blocks(&workbuddy_before);
    let recorded_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut owned: Vec<ManagedCert> = source_pems
        .iter()
        .map(|block| ManagedCert {
            fingerprint: crate::content_hash(block.as_bytes()),
            pem: block.clone(),
            added_to_workbuddy: !workbuddy_blocks_before.contains(block),
            added_to_root_store: None,
            at: recorded_at,
        })
        .collect();

    // 写 WorkBuddy 安装目录**可能因权限失败**（装在 Program Files / /Applications 且非管理员）。
    // 它只是 CLI 侧的补充路径，真正生效的是应用信任库与 Windows 用户根证书库 ——
    // 原先这里用 `?`，会让整个导入报错，但**应用信任库其实已经改了**（部分提交 + 误导性报错）。
    // 现在降级为可报告的非阻断步骤，并在失败时如实标注。
    let (target, bundle_note) = match sync_workbuddy_ca_bundle(
        &executable,
        &crate::union_ca_bundle_path(),
    ) {
        Ok(target) => (target, String::new()),
        Err(error) => {
            // 没写进去就不能声称 bundle 里有本应用加的证书
            for entry in &mut owned {
                entry.added_to_workbuddy = false;
            }
            (
                PathBuf::new(),
                format!(
                    "⚠️ 未能写入 WorkBuddy 安装目录的 CA（{error}）。这通常是权限不足，不影响本应用与系统层面的信任。WorkBuddy 若仍报证书错误：请以管理员身份重试，或用普通方式启动 WorkBuddy（启动时会通过环境变量注入 CA）。"
                ),
            )
        }
    };

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // 加库**之前**逐个查是否已在根证书库里 —— 已在的不算本应用建立，将来不撤销
        // root_store_contains 返回的是"**已经存在**"；而字段语义是"**由本应用新增**"。
        // 上一版漏了取反 ⇒ 别人装的证书被记成我们装的（清理时误删），
        // 我们装的被记成 false（清理时反而留下）。见审查。
        for entry in &mut owned {
            entry.added_to_root_store = ownership_from_presence(root_store_contains(&entry.pem));
        }
        let output = Command::new("certutil.exe")
            .args(["-user", "-addstore", "-f", "Root"])
            .arg(&certificate)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| format!("无法启动 Windows 证书导入：{error}"))?;
        // 加库没成功就不算"本应用新增"——否则清理时会去删一张根本没加进去的证书
        if !output.status.success() {
            for entry in &mut owned {
                entry.added_to_root_store = Some(false);
            }
        }
        let windows_note = if output.status.success() {
            "同时已加入当前 Windows 用户的受信任根证书库。".to_string()
        } else {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if detail.is_empty() {
                format!(
                    "Windows 用户证书库导入未成功（退出码：{:?}），但不影响 WorkBuddy 自定义模型。",
                    output.status.code()
                )
            } else {
                format!("Windows 用户证书库导入未成功，但不影响 WorkBuddy 自定义模型：{detail}")
            }
        };
        // **所有权记录属于导入事务的一部分**：写不进去就没有可撤销依据，
        // 以后清理会因缺少依据而拒绝撤销。不能静默吞掉。
        if let Err(error) = record_managed_certs(owned) {
            return Err(format!(
                "证书已写入应用信任库与系统，但「所有权记录」写入失败：{error}。
\n                 没有这份记录，将来「清空 CA」无法确认哪些是本应用添加的，会拒绝撤销。
\n                 请修复配置目录写入权限后重新导入一次。"
            ));
        }
        // 这里的 `return` 在 **Windows 上**是多余的（下面的 macOS 块被 cfg 掉、
        // 它就是最后一句），但在 macOS 上**不是** —— 后面还有 macOS 专属分支要跑。
        // 两个平台各自的 clippy 会得出相反结论，故显式允许并在此说明。
        #[allow(clippy::needless_return)]
        return Ok(format!(
            "{}{} 请完全退出 WorkBuddy（包括系统托盘）后重新打开。",
            bundle_note_or(&bundle_note, &target),
            windows_note
        ));
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(error) = record_managed_certs(owned) {
            return Err(format!(
                "证书已写入，但「所有权记录写入失败」：{error}。没有这份记录，将来「清空 CA」无法确认哪些是本应用添加的。请修复配置目录写入权限后重新导入一次。"
            ));
        }
        Ok(format!(
            "{}请完全退出 WorkBuddy 后重新打开。",
            bundle_note_or(&bundle_note, &target)
        ))
    }
}

#[tauri::command]
pub async fn list_workbuddy_models(
    request: ListWorkBuddyModelsRequest,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = request
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(global_api_key)
            .or_else(|| request.id.as_deref().and_then(|id| api_key_for(id).ok()))
            .ok_or("请先填写员工 Key，再获取网关模型列表。")?;
        let endpoint = openai_models_endpoint(&request.url)?;
        let response = gateway_client()?
            .get(endpoint)
            .bearer_auth(&key)
            .send()
            .map_err(request_error)?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|error| format!("读取模型列表失败：{error}"))?;
        if status == 401 || status == 403 {
            return Err(
                "网关拒绝获取模型列表：该接口未向当前员工 Key 开放，或 Key 无 OpenAI 协议权限。"
                    .into(),
            );
        }
        if !(200..300).contains(&status) {
            // 先脱敏再截断：反过来会把密钥截成半截留在正文里
            return Err(format!(
                "网关模型列表接口返回 HTTP {status}：{}",
                crate::redact_gateway_text(&body, Some(&key))
                    .chars()
                    .take(240)
                    .collect::<String>()
            ));
        }
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| format!("网关模型列表不是有效 JSON：{error}"))?;
        let items = value
            .get("data")
            .and_then(Value::as_array)
            .or_else(|| value.as_array())
            .ok_or("网关响应中没有标准模型列表 data。")?;
        let mut models = items
            .iter()
            .filter_map(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        models = openai_compatible_model_ids(models);
        if models.is_empty() {
            return Err("网关没有返回可供 WorkBuddy 使用的 OpenAI 协议模型。".into());
        }
        Ok(models)
    })
    .await
    .map_err(|error| format!("WorkBuddy 模型列表任务异常：{error}"))?
}

fn openai_compatible_model_ids(mut models: Vec<String>) -> Vec<String> {
    models.retain(|id| !id.to_ascii_lowercase().starts_with("claude-"));
    models.sort();
    models.dedup();

    let normalized = models
        .iter()
        .map(|id| id.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    models.retain(|id| {
        let lower = id.to_ascii_lowercase();
        lower.starts_with('o') || !normalized.contains(&format!("o{lower}"))
    });
    models
}

#[tauri::command]
pub async fn list_workbuddy_organization_models(id: String) -> Result<Vec<String>, String> {
    let organization = organization_by_id(id.trim())?;
    list_workbuddy_models(ListWorkBuddyModelsRequest {
        id: None,
        url: organization.url,
        api_key: Some(organization.api_key),
    })
    .await
}

#[tauri::command]
pub async fn check_workbuddy_certificate(
    url: String,
) -> Result<WorkBuddyCertificateStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let parsed = url::Url::parse(url.trim()).map_err(|_| "网关地址不是有效 URL。")?;
        if parsed.scheme() == "http" {
            // 远程 http 已在保存时被拒，能走到这里的只可能是本机回环
            return Ok(WorkBuddyCertificateStatus {
                state: "notRequired".into(),
                detail: "网关使用本机回环 HTTP，不需要 TLS 证书，流量不出本机。".into(),
            });
        }
        if parsed.scheme() != "https" {
            return Err("网关地址必须以 http:// 或 https:// 开头。".into());
        }
        match gateway_client()?.get(parsed).send() {
            Ok(_) => {
                let bundle = crate::union_ca_bundle_path();
                if bundle.is_file() {
                    if let Some(executable) = find_executable() {
                        if !workbuddy_ca_bundle_is_synced(&executable, &bundle) {
                            return Ok(WorkBuddyCertificateStatus {
                                state: "untrusted".into(),
                                detail: "管理中心可以访问网关，但 CA 尚未同步到 WorkBuddy CLI；请重新导入证书。".into(),
                            });
                        }
                    }
                }
                Ok(WorkBuddyCertificateStatus {
                    state: "trusted".into(),
                    detail: "网关 TLS 握手成功，WorkBuddy CLI CA 已同步。".into(),
                })
            }
            Err(error) => {
                let detail = request_error(error);
                let state = if detail.starts_with("TLS 证书校验失败") {
                    "untrusted"
                } else {
                    "unreachable"
                };
                Ok(WorkBuddyCertificateStatus {
                    state: state.into(),
                    detail,
                })
            }
        }
    })
    .await
    .map_err(|error| format!("WorkBuddy 证书检测任务异常：{error}"))?
}

#[tauri::command]
pub fn save_workbuddy_model(
    mut request: SaveWorkBuddyModelRequest,
) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    let path = models_path();
    if request.model.use_global_key {
        let gateway = read_gateway_config()?;
        if gateway.api_key.is_empty() {
            return Err("请先保存 WorkBuddy 全局网关和员工 Key。".into());
        }
        request.model.url = gateway.url;
        request.model.api_key = Some(gateway.api_key);
    }
    ensure_expected_revision(&path, &request.expected_revision)?;
    let mut document = read_document(&path)?;
    let root = document
        .as_object_mut()
        .ok_or("WorkBuddy models.json 顶层必须是对象。")?;
    // availableModels 缺失或为空代表“全部可见”。首次创建该字段时先纳入存量模型，
    // 避免保存一个新模型后让用户已有模型从 WorkBuddy 选择器消失。
    let available_models_is_all = root
        .get("availableModels")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true);
    if available_models_is_all {
        let existing_ids = root
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| value.get("id").and_then(Value::as_str))
            .map(|id| Value::String(id.to_string()))
            .collect::<Vec<_>>();
        root.insert("availableModels".into(), Value::Array(existing_ids));
    }
    let models = root
        .entry("models")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("WorkBuddy models.json 的 models 字段必须是数组。")?;
    let previous_id = request
        .previous_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let index = previous_id
        .and_then(|id| {
            models
                .iter()
                .position(|value| value.get("id").and_then(Value::as_str) == Some(id))
        })
        .or_else(|| {
            models.iter().position(|value| {
                value.get("id").and_then(Value::as_str) == Some(request.model.id.trim())
            })
        });
    if !request.model.use_global_key
        && request
            .model
            .api_key
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        let gateway = read_gateway_config()?;
        let existing_uses_global = index
            .and_then(|position| models.get(position))
            .map(|value| {
                !gateway.api_key.is_empty()
                    && value.get("url").and_then(Value::as_str) == Some(gateway.url.as_str())
                    && value.get("apiKey").and_then(Value::as_str) == Some(gateway.api_key.as_str())
            })
            .unwrap_or(false);
        if index.is_none() || existing_uses_global {
            return Err("切换为其他 Key 时，必须填写当前模型的独立员工 Key。".into());
        }
    }
    validate_model(&mut request.model, index.is_none())?;
    if models.iter().enumerate().any(|(candidate, value)| {
        Some(candidate) != index
            && value.get("id").and_then(Value::as_str) == Some(request.model.id.as_str())
    }) {
        return Err("已经存在同名模型 ID。".into());
    }
    let mut raw = index
        .and_then(|position| models.get(position).cloned())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    set_model_fields(&mut raw, &request.model);
    if raw
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .is_empty()
    {
        return Err("该模型没有可用的员工 Key。".into());
    }
    if let Some(position) = index {
        models[position] = Value::Object(raw);
    } else {
        models.push(Value::Object(raw));
    }
    let available = root
        .entry("availableModels")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("WorkBuddy models.json 的 availableModels 字段必须是数组。")?;
    if let Some(previous) = previous_id.filter(|id| *id != request.model.id) {
        available.retain(|value| value.as_str() != Some(previous));
    }
    available.retain(|value| value.as_str() != Some(request.model.id.as_str()));
    if request.model.visible {
        available.push(Value::String(request.model.id.clone()));
    }
    write_document(&path, &document)?;
    Ok(build_state_unlocked())
}

#[tauri::command]
pub fn delete_workbuddy_model(
    request: DeleteWorkBuddyModelRequest,
) -> Result<WorkBuddyState, String> {
    let _guard = write_guard();
    let path = models_path();
    ensure_expected_revision(&path, &request.expected_revision)?;
    let mut document = read_document(&path)?;
    let root = document
        .as_object_mut()
        .ok_or("WorkBuddy models.json 顶层必须是对象。")?;
    let models = root
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or("WorkBuddy models.json 的 models 字段必须是数组。")?;
    let before = models.len();
    models.retain(|value| value.get("id").and_then(Value::as_str) != Some(request.id.as_str()));
    if models.len() == before {
        return Err("未找到要删除的 WorkBuddy 模型。".into());
    }
    if let Some(available) = root
        .get_mut("availableModels")
        .and_then(Value::as_array_mut)
    {
        available.retain(|value| value.as_str() != Some(request.id.as_str()));
    }
    write_document(&path, &document)?;
    Ok(build_state_unlocked())
}

fn api_key_for(id: &str) -> Result<String, String> {
    let document = read_document(&models_path())?;
    document
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|value| value.get("id").and_then(Value::as_str) == Some(id))
        })
        .and_then(|value| value.get("apiKey"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "该模型没有已保存的员工 Key，请重新填写。".into())
}

fn global_api_key() -> Option<String> {
    read_gateway_config()
        .ok()
        .map(|config| config.api_key)
        .filter(|value| !value.is_empty())
}

fn execute_json_request(
    request_url: &str,
    key: &str,
    body: &Value,
    extra_headers: &[(&str, &str)],
) -> Result<(u16, String), String> {
    let client = gateway_client()?;
    let mut request = client.post(request_url).bearer_auth(key).json(body);
    for (name, value) in extra_headers {
        request = request.header(*name, *value);
    }
    let response = request.send().map_err(request_error)?;
    let status = response.status().as_u16();
    let response_body = response
        .text()
        .map_err(|error| format!("读取网关响应失败：{error}"))?;
    Ok((status, response_body))
}

/// 读应用导入的 CA bundle，拆成逐张证书。
/// 读不到 / 为空 = 用户没导入过，返回空表（不是错误）。
fn app_ca_certificates() -> Result<Vec<reqwest::Certificate>, String> {
    let Ok(text) = fs::read_to_string(crate::union_ca_bundle_path()) else {
        return Ok(vec![]);
    };
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    reqwest::Certificate::from_pem_bundle(text.as_bytes())
        .map_err(|error| format!("读取已导入的 CA 证书失败：{error}"))
}

/// 管理中心的网关客户端。
///
/// 信任集必须是**平台信任根 ∪ 应用导入的 CA**，与 claude 一致
/// （Node 的 `NODE_EXTRA_CA_CERTS` 也是**追加**语义）。原先只用平台信任库，
/// 于是 macOS 上出现"WorkBuddy 靠 NODE_EXTRA_CA_CERTS 连得上，管理中心却报证书失败"。
fn gateway_client() -> Result<reqwest::blocking::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30));
    // **已核实** reqwest 0.13.4 的语义（读的是本机实际编译的那份源码，
    // `client.rs:756-772`）：`root_certs` 非空时走
    // `rustls_platform_verifier::Verifier::new_with_extra_roots(..)`，
    // 即**平台根 + 额外根**，不会把公有 CA 替换掉 —— 与 curl 的 `--cacert`（替换）**相反**。
    // 所以这里加根是安全的；下面的测试会持续钉住这个语义，防止 reqwest 升级后翻转。
    for certificate in app_ca_certificates()? {
        builder = builder.add_root_certificate(certificate);
    }
    builder
        .build()
        .map_err(|error| format!("创建网关连接失败：{error}"))
}

fn request_error(error: reqwest::Error) -> String {
    let mut detail = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        detail.push_str(": ");
        detail.push_str(&cause.to_string());
        source = cause.source();
    }
    let lower = detail.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("unknown issuer") {
        "TLS 证书校验失败。请先导入管理员提供的 MaaS Gateway CA 根证书。".to_string()
    } else {
        format!("网关连接失败：{detail}")
    }
}

fn anthropic_endpoint(openai_endpoint: &str) -> Result<String, String> {
    validate_request_url(openai_endpoint)?;
    let mut parsed = url::Url::parse(openai_endpoint).map_err(|_| "API 地址不是有效 URL。")?;
    parsed.set_path("/anthropic/v1/messages");
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn openai_chat_endpoint(gateway_url: &str) -> Result<String, String> {
    validate_request_url(gateway_url)?;
    let mut parsed = url::Url::parse(gateway_url).map_err(|_| "API 地址不是有效 URL。")?;
    let path = parsed.path().trim_end_matches('/');
    let endpoint_path = if path.ends_with("/v1/chat/completions") {
        path.to_string()
    } else if path.ends_with("/v1") {
        format!("{path}/chat/completions")
    } else {
        format!("{path}/v1/chat/completions")
    };
    parsed.set_path(&endpoint_path);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn openai_api_base_url(gateway_url: &str) -> Result<String, String> {
    let chat_endpoint = openai_chat_endpoint(gateway_url)?;
    let mut parsed = url::Url::parse(&chat_endpoint).map_err(|_| "API 地址不是有效 URL。")?;
    let path = parsed
        .path()
        .trim_end_matches("/chat/completions")
        .to_string();
    parsed.set_path(&path);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn openai_models_endpoint(gateway_url: &str) -> Result<String, String> {
    validate_request_url(gateway_url)?;
    let mut parsed = url::Url::parse(gateway_url).map_err(|_| "网关地址不是有效 URL。")?;
    let path = parsed.path().trim_end_matches('/');
    let base = path.strip_suffix("/chat/completions").unwrap_or(path);
    let endpoint_path = if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    };
    parsed.set_path(&endpoint_path);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn selected_managed_certs(manifest: &ManagedCerts, requested_pem: &str) -> Vec<ManagedCert> {
    let requested = pem_blocks(requested_pem);
    manifest
        .certificates
        .iter()
        .filter(|cert| requested.contains(&cert.pem))
        .cloned()
        .collect()
}

fn validate_request_url(value: &str) -> Result<(), String> {
    if !crate::valid_base_url(value) {
        return Err("网关地址必须使用 HTTPS；HTTP 仅允许本机回环地址。已中止请求。".into());
    }
    Ok(())
}

fn classify_response(status: u16, body: &str, api_key: Option<&str>) -> WorkBuddyTestResult {
    // 网关/代理可能在错误正文里回显 Authorization 头，必须后端脱敏（React 只防 XSS）
    let short = crate::redact_gateway_text(body, api_key)
        .chars()
        .take(280)
        .collect::<String>();
    let detail = match status {
        200..=299 => "连接成功，WorkBuddy 可以通过该模型调用 MaaS Gateway。".into(),
        400 => format!("网关返回 400，请检查模型 ID 和请求兼容性：{short}"),
        401 => "网关返回 401：员工 Key 错误、已禁用或已轮换。".into(),
        403 if body.contains("model_access_denied") => {
            "员工 Key 有效，但当前账号没有该模型的调用权限。请选择其他模型，或联系管理员授权。"
                .into()
        }
        403 => "网关返回 403：员工 Key 有效，但当前账号没有执行该请求的权限。".into(),
        429 => "网关返回 429：当前员工额度或频率已达到限制。".into(),
        503 => "网关返回 503：管理员尚未配置该模型的上游服务商。".into(),
        _ => format!("网关返回 HTTP {status}：{short}"),
    };
    WorkBuddyTestResult {
        ok: (200..300).contains(&status),
        status_code: status,
        detail,
    }
}

#[tauri::command]
pub async fn test_workbuddy_model(
    request: TestWorkBuddyModelRequest,
) -> Result<WorkBuddyTestResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut model = WorkBuddyModelInput {
            id: request.id,
            name: String::new(),
            vendor: String::new(),
            url: request.url,
            api_key: request.api_key,
            max_input_tokens: 1,
            max_output_tokens: 1,
            supports_tool_call: true,
            supports_images: false,
            supports_reasoning: false,
            use_custom_protocol: request.use_custom_protocol,
            visible: true,
            use_global_key: request.use_global_key,
        };
        validate_model(&mut model, false)?;
        let key = model
            .api_key
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                if model.use_global_key {
                    global_api_key()
                } else {
                    api_key_for(&model.id).ok()
                }
            })
            .ok_or("请先保存全局 Key，或为当前模型填写其他 Key。")?;
        let request_url = if model.use_custom_protocol {
            model.url.clone()
        } else {
            openai_chat_endpoint(&model.url)?
        };
        let body = serde_json::json!({
            "model": model.id,
            "messages": [{"role": "user", "content": "只回复 OK"}],
            "max_tokens": 8,
            "stream": false
        });
        let (status, response_body) = execute_json_request(&request_url, &key, &body, &[])?;
        if status == 401 {
            let anthropic_url = anthropic_endpoint(&request_url)?;
            let anthropic_body = serde_json::json!({
                "model": model.id,
                "messages": [{"role": "user", "content": "只回复 OK"}],
                "max_tokens": 8,
                "stream": false
            });
            let (anthropic_status, _) = execute_json_request(
                &anthropic_url,
                &key,
                &anthropic_body,
                &[("anthropic-version", "2023-06-01")],
            )?;
            let detail = if anthropic_status == 401 || anthropic_status == 403 {
                "同一 Key 在 OpenAI 和 Anthropic 路由都被网关拒绝。请确认粘贴到 WorkBuddy 的 Key 与 Claude Code 当前实际使用的 Key 完全一致。".to_string()
            } else {
                format!(
                    "员工 Key 有效（Anthropic 路由返回 HTTP {anthropic_status}），但 WorkBuddy 所需的 OpenAI /v1/chat/completions 路由返回 401。请管理员为该员工 Key 开放 OpenAI 协议访问。"
                )
            };
            return Ok(WorkBuddyTestResult {
                ok: false,
                status_code: status,
                detail,
            });
        }
        Ok(classify_response(status, &response_body, Some(&key)))
    })
    .await
    .map_err(|error| format!("WorkBuddy 测试任务异常：{error}"))?
}

fn configure_workbuddy_command(command: &mut Command, certificate: &Path) {
    command.env("NODE_EXTRA_CA_CERTS", certificate);
}

// ---------------- macOS 启动：open --env ----------------
//
// 用 `/usr/bin/open --env` 而**不是**直接执行 `.app/Contents/MacOS/` 内的二进制：
// 后者会绕过 LaunchServices，影响应用激活、单实例、工作目录与生命周期。
//
// `open --env` 自 **macOS 13 (Ventura)** 起才支持。低于该版本时**必须明确报错**，
// 不得静默按普通方式启动并显示成功 —— 那会让用户以为自定义 CA 已生效，
// 实际连不上自签网关却毫无提示。
//
// 下面几个是**纯逻辑**，刻意不加 `#[cfg(target_os = "macos")]`，
// 这样在 Windows 上也能跑测试（CLAUDE.md：平台逻辑应在任一开发平台可测）。

/// macOS 13 起 `open` 才支持 `--env`
const MACOS_OPEN_ENV_MIN_MAJOR: u32 = 13;

/// 解析 `sw_vers -productVersion` 的主版本号（"15.6" → 15）
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_macos_major(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.trim().parse().ok()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_open_supports_env(major: u32) -> bool {
    major >= MACOS_OPEN_ENV_MIN_MAJOR
}

/// 构造 `open` 的参数。**只有需要自定义 CA 时才带 `--env`**；
/// 不需要时保持普通启动，因此不受 macOS 版本限制。
///
/// 路径不做事先转义：这里经 `Command::arg` 走 argv，不经过 shell，
/// 所以含空格 / 引号 / 中文的路径天然是一个完整参数。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_open_args(executable: &Path, ca: Option<&Path>) -> Vec<String> {
    let mut args = vec![];
    if let Some(ca) = ca {
        args.push("--env".to_string());
        args.push(format!("NODE_EXTRA_CA_CERTS={}", ca.display()));
    }
    args.push(executable.display().to_string());
    args
}

/// `open --env` 是否可用。`None` = 查不到版本（此时按"不可用"处理，宁可报错也不静默降级）。
///
/// 不按 `target_os` 条件编译：调用点是运行期 `cfg!(target_os = "macos")`，
/// 两个分支在任一平台都要能通过编译，这样 CI 在 Windows 上也能类型检查到它。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_open_env_support() -> Option<bool> {
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_major(&String::from_utf8_lossy(&output.stdout)).map(macos_open_supports_env)
}

/// macOS 能否按预期启动（纯逻辑，任一平台可测）。
///
/// **已在运行是一条独立的失败路径**：`open --env` 只能给**新启动**的进程注入环境变量；
/// WorkBuddy 已经在跑时 LaunchServices 只是把它激活，变量进不去。
/// 此时若返回成功，用户会以为 CA 已生效 —— 实际没有，然后来报"导入了证书还是连不上"。
fn launch_decision(
    ca_required: bool,
    already_running: bool,
    env_support: Option<bool>,
) -> Result<(), String> {
    if !ca_required {
        // 不需要自定义 CA：普通启动既不受版本限制，也不怕已在运行
        return Ok(());
    }
    if already_running {
        return Err(
            "WorkBuddy 正在运行，无法为它补上自定义 CA（已启动的进程改不了环境变量）。\
             请完全退出 WorkBuddy（包括系统托盘）后，再点一次「打开 WorkBuddy」。"
                .into(),
        );
    }
    match env_support {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "当前 macOS 版本不支持带自定义 CA 启动 WorkBuddy（open --env 需要 macOS {MACOS_OPEN_ENV_MIN_MAJOR} 及以上）。请直接启动 WorkBuddy，或把网关 CA 导入系统信任库。"
        )),
        None => Err(
            "无法确定当前 macOS 版本，因此不能确认 open 是否支持 --env 注入自定义 CA。请直接启动 WorkBuddy，或把网关 CA 导入系统信任库。".into(),
        ),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launch_workbuddy_macos(executable: &Path, ca: Option<&Path>) -> Result<(), String> {
    let ca_required = ca.is_some();
    // 已在运行时不必再去问版本（结论已经是失败），省一次 sw_vers
    let already_running = ca_required && running_executable().is_some();
    let env_support = if ca_required && !already_running {
        macos_open_env_support()
    } else {
        None
    };
    launch_decision(ca_required, already_running, env_support)?;

    let mut command = Command::new("/usr/bin/open");
    for arg in macos_open_args(executable, ca) {
        command.arg(arg);
    }
    let output = command
        .output()
        .map_err(|error| format!("启动 WorkBuddy 失败：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("启动 WorkBuddy 失败：{stderr}"));
    }
    Ok(())
}

#[tauri::command]
pub fn launch_workbuddy() -> Result<String, String> {
    let executable = find_executable().ok_or("未检测到 WorkBuddy 安装。")?;
    let certificate = crate::union_ca_bundle_path();
    // **写 WorkBuddy 安装目录里的 ca.pem 不能作为启动的前提。**
    // WorkBuddy 常装在 Program Files / /Applications，普通用户写不进去；
    // 原先带 `?` 会让"打开 WorkBuddy"整个失败，而真正有效的机制是**进程环境注入**
    // （Windows 的 NODE_EXTRA_CA_CERTS / macOS 的 open --env），那条路本来就能走通。
    // 所以改成尽力而为，失败只作为提示返回，不阻断启动。
    let ca_sync_warning = if certificate.is_file()
        && cfg!(any(target_os = "windows", target_os = "macos"))
    {
        sync_workbuddy_ca_bundle(&executable, &certificate)
            .err()
            .map(|error| {
                format!("未能写入 WorkBuddy 安装目录的 CA（{error}），已改用启动时注入环境变量。")
            })
    } else {
        None
    };
    if cfg!(target_os = "windows") {
        // **Windows 也要做这条判定**：WorkBuddy 同样是单实例应用，
        // 已在运行时新进程只会唤醒旧进程，NODE_EXTRA_CA_CERTS 进不去 ——
        // 不加检查就会在安装目录写入也失败的情况下，**谎报"CA 已生效"**。
        // Windows 用 Command::env 直接注入，没有 macOS 的 open --env 版本门槛，故 env_support = Some(true)。
        let ca = certificate.is_file().then_some(certificate.as_path());
        let running = ca.is_some() && running_executable().is_some();
        launch_decision(ca.is_some(), running, Some(true))?;

        let mut command = Command::new(&executable);
        if certificate.is_file() {
            configure_workbuddy_command(&mut command, &certificate);
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
            .spawn()
            .map_err(|error| format!("启动 WorkBuddy 失败：{error}"))?;
    } else if cfg!(target_os = "macos") {
        // 只有确实需要自定义 CA 时才注入环境变量；不需要时保持普通启动。
        // 注意用的是 output() 而非 spawn()：要拿到退出码，才能在不支持 --env
        // 或启动失败时报错，而不是静默显示成功。
        let ca = certificate.is_file().then_some(certificate.as_path());
        launch_workbuddy_macos(&executable, ca)?;
    } else {
        return Err("当前平台暂不支持自动打开 WorkBuddy。".into());
    }
    Ok(ca_sync_warning.unwrap_or_else(|| "已启动 WorkBuddy。".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用示例网关。刻意**不用**真实内网地址：本仓库要公开，
    /// 测试夹具里也不该留组织内网地址。用 IANA 保留的 example.com。
    const TEST_ENDPOINT: &str = "https://gateway.example.com:8080";

    #[test]
    fn parses_models_without_exposing_api_key() {
        let document = serde_json::json!({
            "models": [{
                "id": "glm-5.2",
                "name": "GLM 5.2",
                "apiKey": "gw-sk-secret",
                "url": TEST_ENDPOINT,
                "unknown": "kept"
            }],
            "availableModels": ["glm-5.2"]
        });
        let models = parse_models(
            &document,
            &StoredGatewayConfig {
                url: TEST_ENDPOINT.into(),
                api_key: "gw-sk-secret".into(),
            },
        );
        assert_eq!(models.len(), 1);
        assert!(models[0].has_api_key);
        assert!(models[0].visible);
    }

    #[test]
    fn empty_available_models_keeps_all_models_visible() {
        let document = serde_json::json!({
            "models": [{
                "id": "glm-5.2",
                "apiKey": "gw-sk-secret",
                "url": TEST_ENDPOINT
            }],
            "availableModels": []
        });
        let models = parse_models(&document, &StoredGatewayConfig::default());
        assert!(models[0].visible);
    }

    #[test]
    fn request_endpoints_reject_remote_plaintext_and_allow_loopback() {
        for endpoint in [
            openai_models_endpoint,
            openai_chat_endpoint,
            anthropic_endpoint,
        ] {
            assert!(endpoint("http://gateway.example.com").is_err());
            assert!(endpoint("http://127.0.0.1:18765").is_ok());
            assert!(endpoint("https://gateway.example.com").is_ok());
            assert!(endpoint("file:///tmp/config").is_err());
        }
    }

    #[test]
    fn revocation_selection_does_not_include_other_gateways_certificates() {
        let cert = |tag: &str| ManagedCert {
            fingerprint: tag.into(),
            pem: pem_blocks(&format!(
                "-----BEGIN CERTIFICATE-----\n{tag}\n-----END CERTIFICATE-----"
            ))[0]
                .clone(),
            added_to_workbuddy: true,
            added_to_root_store: Some(true),
            at: 0,
        };
        let a = cert("AAAA");
        let b = cert("BBBB");
        let manifest = ManagedCerts {
            certificates: vec![a.clone(), b],
        };
        let selected = selected_managed_certs(&manifest, &a.pem);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].fingerprint, "AAAA");
        assert!(selected_managed_certs(&manifest, "").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn root_store_query_distinguishes_valid_certificate_from_parse_failure() {
        assert!(access_root_certificate(TEST_CA_PEM, false).is_ok());
        assert!(access_root_certificate("invalid", false).is_err());
    }

    #[test]
    fn validates_gateway_root_url() {
        let mut model = WorkBuddyModelInput {
            id: "glm-5.2".into(),
            name: String::new(),
            vendor: String::new(),
            url: TEST_ENDPOINT.into(),
            api_key: Some("gw-sk-test".into()),
            max_input_tokens: 128_000,
            max_output_tokens: 8_192,
            supports_tool_call: true,
            supports_images: false,
            supports_reasoning: true,
            use_custom_protocol: false,
            visible: true,
            use_global_key: true,
        };
        validate_model(&mut model, true).unwrap();
        assert_eq!(model.name, "glm-5.2");
        assert_eq!(model.vendor, "MaaS Gateway");
        model.url = "ftp://gateway.example.com".into();
        assert!(validate_model(&mut model, true).is_err());
    }

    #[test]
    fn derives_anthropic_probe_from_openai_endpoint() {
        assert_eq!(
            anthropic_endpoint(TEST_ENDPOINT).unwrap(),
            "https://gateway.example.com:8080/anthropic/v1/messages"
        );
    }

    #[test]
    fn derives_openai_chat_endpoint_from_gateway_root() {
        assert_eq!(
            openai_chat_endpoint(TEST_ENDPOINT).unwrap(),
            "https://gateway.example.com:8080/v1/chat/completions"
        );
    }

    #[test]
    fn derives_workbuddy_openai_api_base_from_gateway_root() {
        assert_eq!(
            openai_api_base_url("https://gateway.example.com:8080").unwrap(),
            "https://gateway.example.com:8080/v1"
        );
        assert_eq!(
            openai_api_base_url("https://gateway.example.com/v1/chat/completions").unwrap(),
            "https://gateway.example.com/v1"
        );
    }

    #[test]
    fn keeps_only_workbuddy_openai_model_aliases() {
        let models = openai_compatible_model_ids(vec![
            "claude-zhipu-5.2".into(),
            "glm-5.2".into(),
            "Oglm-5.2".into(),
            "deepseek-v4-pro".into(),
            "claude-dsv4-pro".into(),
            "qwen3.8-max".into(),
            "OQwen3.8-max".into(),
        ]);

        assert_eq!(models, vec!["OQwen3.8-max", "Oglm-5.2", "deepseek-v4-pro"]);
    }

    #[test]
    fn managed_model_identity_uses_optional_explicit_prefix() {
        assert_eq!(
            managed_model_identity("deepseek-v4-pro", ""),
            ("deepseek-v4-pro".into(), "user".into())
        );
        assert_eq!(
            managed_model_identity("deepseek-v4-pro", "company"),
            ("company".into(), "user".into())
        );
    }

    #[test]
    fn detects_organization_configs_missing_model_prefix() {
        assert!(organization_config_needs_model_prefix(&serde_json::json!([
            {"id": "old", "name": "旧网关", "url": TEST_ENDPOINT}
        ])));
        assert!(!organization_config_needs_model_prefix(
            &serde_json::json!([
                {"id": "new", "name": "新网关", "modelPrefix": "", "url": TEST_ENDPOINT}
            ])
        ));
    }

    #[test]
    fn derives_openai_models_endpoint_from_gateway_root() {
        assert_eq!(
            openai_models_endpoint(TEST_ENDPOINT).unwrap(),
            "https://gateway.example.com:8080/v1/models"
        );
    }

    #[test]
    fn known_gateway_errors_have_actionable_messages() {
        assert!(classify_response(401, "", None).detail.contains("Key"));
        assert!(
            classify_response(403, r#"{"error":{"code":"model_access_denied"}}"#, None)
                .detail
                .contains("模型的调用权限")
        );
        assert!(classify_response(429, "", None).detail.contains("额度"));
        assert!(classify_response(503, "", None).detail.contains("上游"));
    }

    /// 测试专用自签 CA —— 只用于钉住「额外根不会替换平台信任根」这一语义，无任何真实用途
    const TEST_CA_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDGzCCAgOgAwIBAgIUPeeHnV6GKf6BMtXfJ1xdmJIG398wDQYJKoZIhvcNAQEL
BQAwHTEbMBkGA1UEAwwSQ0MgTWFuYWdlciBUZXN0IENBMB4XDTI2MDkxMjA0MTUx
MFoXDTM2MDkwOTA0MTUxMFowHTEbMBkGA1UEAwwSQ0MgTWFuYWdlciBUZXN0IENB
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAxHH6sGW/LeWm55eholZd
uzupyk2w2WoCZcKrUnOpXEedRCYKrRrBBSw2LWX54gS72sUIXONqAo1dvLWgpO10
ooOY4xoDWPnKxLwW3pUR12NHr03r4ceM8X+OP8NI21EAI8giITr1J45YQM8rf55A
dBN0tCf5/bh3V3R9OdNsDa+ron+9Csk2OQZRZ/3KMgLR9Etk6hC7lzUFb7vkic73
/3QGA98ywTXmFhUZGUQ3msN0W4hPGutLWLKxgnvHiARqSe/xlJaQ3udAOTzu3Y3D
WlnSqiFDrLeey5pLo+6qs9PvFZzw369wxChXbFoFGezoWPSVBVRxjVpHdKNH/VF/
HwIDAQABo1MwUTAdBgNVHQ4EFgQU9Tu5H9RGqv9DtZ4Az0jmgsAlCRQwHwYDVR0j
BBgwFoAU9Tu5H9RGqv9DtZ4Az0jmgsAlCRQwDwYDVR0TAQH/BAUwAwEB/zANBgkq
hkiG9w0BAQsFAAOCAQEAB6J5AbeCCsgz5NRlvQBxhbhClssU866DZ8gAN+2I1Hbu
jUcOIbnN93/tMSgnEgXx7MPxHsTs7lLA1SiE98yM20lIrMAXMYczviKOiimjhe13
pPzxxfrDMmBEJ5ZwON+l8RtAjsF+/51rMjcn6Bb/Pbt1/LERKftQLFczM/GoyeFf
QNwhLO2hzfGrrk7qLc3U7gwRIFB0FrBpOyrUxrEMcSsuARgMYrWeqoQ9beicB2hW
4V8CveqDKpfe2ik47i89rUIGM4B8iZsjyPDndT1g7rJvs6Ocz3JJcNyMtKYb6vOF
jlM7HEs96XujSVLwEU310EvCiXpwSj/ZloPLVtVd0g==
-----END CERTIFICATE-----"#;

    #[test]
    fn adding_app_ca_does_not_replace_platform_roots() {
        // 回归护栏（审查 F6）。已核实 reqwest 0.13.4 走 new_with_extra_roots
        // （平台根 ∪ 额外根），但**升级 reqwest 可能翻转这个语义** —— 一旦变成
        // 像 curl 的 --cacert 那样"替换"，所有用公有 CA 的网关都会被打挂。
        //
        // 做法：带上一张自签 CA，再去连公有 CA 的站点 —— 必须仍然成功。
        //
        // 先做一次**对照请求**判断网络是否可用。上一版是靠"错误信息里含不含
        // certificate/tls 字样"来区分回归与环境问题 —— 那是不可靠的：
        // 偶发的 "tls handshake timeout" 会被误判成回归，让这条测试随机打挂 CI。
        let _ = rustls::crypto::ring::default_provider().install_default();
        let plain = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        if plain.get("https://www.example.com").send().is_err() {
            return; // 无网络环境：跳过（本测试验证的是信任集语义，不是连通性）
        }

        let certificate = reqwest::Certificate::from_pem(TEST_CA_PEM.as_bytes()).unwrap();
        let client = reqwest::blocking::Client::builder()
            .add_root_certificate(certificate)
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        // 对照已成功，这一步再失败就只可能是信任集被改坏
        let response = client
            .get("https://www.example.com")
            .send()
            .expect("带上应用 CA 后连不上公有站点 —— 平台信任根被替换了");
        assert!(
            response.status().is_success(),
            "公有站点返回 {}，平台信任根可能被替换了",
            response.status()
        );
    }

    #[test]
    fn gateway_error_body_never_leaks_a_credential() {
        let key = "gw-sk-LIVE-EMPLOYEE-KEY-0001";
        // 网关回显 Authorization 头是最常见的一种；也必须挡住**别的** Key
        let echoed = format!(r#"{{"error":"bad bearer {key}"}}"#);
        let detail = classify_response(500, &echoed, Some(key)).detail;
        assert!(!detail.contains(key), "{detail}");
        assert!(detail.contains("<已隐去>"), "{detail}");

        let other = r#"{"error":"upstream said: Bearer gw-sk-SOME-OTHER-KEY-9999"}"#;
        let detail = classify_response(500, other, None).detail;
        assert!(!detail.contains("SOME-OTHER-KEY-9999"), "{detail}");

        // 普通词不能被误伤
        let benign = classify_response(500, r#"{"note":"sk-1 too short"}"#, None).detail;
        assert!(benign.contains("sk-1"), "{benign}");
    }

    #[test]
    fn workbuddy_launch_inherits_extra_ca_bundle() {
        let mut command = Command::new("workbuddy-test");
        let certificate = Path::new(r"C:\ca-cert.pem");

        configure_workbuddy_command(&mut command, certificate);

        let configured = command
            .get_envs()
            .find(|(name, _)| *name == "NODE_EXTRA_CA_CERTS")
            .and_then(|(_, value)| value)
            .map(PathBuf::from);
        assert_eq!(configured.as_deref(), Some(certificate));
    }

    // ---------------- macOS 启动参数（纯逻辑，Windows 上也可跑） ----------------

    #[test]
    fn macos_open_args_omit_env_without_custom_ca() {
        // 不需要自定义 CA 时保持普通启动，因此不受 macOS 版本限制
        let args = macos_open_args(Path::new("/Applications/WorkBuddy.app"), None);
        assert_eq!(args, vec!["/Applications/WorkBuddy.app"]);
        assert!(!args.iter().any(|arg| arg.contains("--env")));
    }

    #[test]
    fn macos_open_args_inject_ca_env_when_needed() {
        let args = macos_open_args(
            Path::new("/Applications/WorkBuddy.app"),
            Some(Path::new("/Users/hq/.cc-manager/ca-cert.pem")),
        );
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "--env");
        assert_eq!(
            args[1],
            "NODE_EXTRA_CA_CERTS=/Users/hq/.cc-manager/ca-cert.pem"
        );
        assert_eq!(args[2], "/Applications/WorkBuddy.app");
    }

    #[test]
    fn macos_open_args_keep_paths_with_spaces_and_chinese_intact() {
        // 走 argv 不经 shell，所以含空格 / 引号 / 中文的路径天然是一个完整参数，
        // 不需要（也不应该）自己做转义。这里钉住"一个路径 = 一个参数"。
        let app = Path::new("/Applications/我的 WorkBuddy.app");
        let ca = Path::new("/Users/hq/库/Application Support/cc manager/ca-cert.pem");
        let args = macos_open_args(app, Some(ca));
        assert_eq!(args.len(), 3, "{args:?}");
        assert_eq!(args[1], format!("NODE_EXTRA_CA_CERTS={}", ca.display()));
        assert_eq!(args[2], app.display().to_string());
    }

    #[test]
    fn launch_decision_blocks_when_already_running() {
        // 回归（审查 F5 + F7）：open --env / Command::env 都只能给**新启动**的进程注入环境变量。
        // WorkBuddy 已在跑时 LaunchServices 只激活它，变量进不去 —— 必须报错而不是返回成功。
        let err = launch_decision(true, true, Some(true)).unwrap_err();
        assert!(err.contains("完全退出"), "{err}");
        assert!(err.contains("托盘"), "{err}");
        // 已在运行时版本探测结果无关紧要，仍是同一结论
        assert!(launch_decision(true, true, None).is_err());
        assert!(launch_decision(true, true, Some(false)).is_err());
    }

    #[test]
    fn launch_decision_allows_plain_launch_always() {
        // 不需要自定义 CA 时：不受版本限制，也不怕已在运行
        assert!(launch_decision(false, false, None).is_ok());
        assert!(launch_decision(false, true, None).is_ok());
    }

    #[test]
    fn launch_decision_reports_version_problems_precisely() {
        assert!(launch_decision(true, false, Some(true)).is_ok());
        let old = launch_decision(true, false, Some(false)).unwrap_err();
        assert!(old.contains("不支持"), "{old}");
        // 查不到版本不能当成"可用"，也不能静默按普通方式启动
        let unknown = launch_decision(true, false, None).unwrap_err();
        assert!(unknown.contains("无法确定"), "{unknown}");
    }

    #[test]
    fn macos_open_env_gate_requires_ventura_or_newer() {
        assert!(macos_open_supports_env(13));
        assert!(macos_open_supports_env(15)); // Sequoia
        assert!(!macos_open_supports_env(12)); // Monterey：不支持 --env
        assert!(!macos_open_supports_env(11));
        assert!(!macos_open_supports_env(0));
    }

    #[test]
    fn parse_macos_major_handles_sw_vers_output_shapes() {
        assert_eq!(parse_macos_major("15.6"), Some(15));
        assert_eq!(parse_macos_major("13.0.1"), Some(13));
        assert_eq!(parse_macos_major("12.7.6"), Some(12));
        assert_eq!(parse_macos_major("15.6\n"), Some(15)); // sw_vers 输出带换行
        assert_eq!(parse_macos_major(" 14 "), Some(14));
        // 解析不出来必须给 None，由调用方按"不可用"报错，不能当成可用
        assert_eq!(parse_macos_major(""), None);
        assert_eq!(parse_macos_major("abc"), None);
    }

    // ---------------- 写入并发保护（P0-B#10） ----------------

    #[test]
    fn stale_revision_is_rejected_instead_of_overwriting() {
        let dir = std::env::temp_dir().join(format!(
            "ccm-wb-rev-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cc-manager-organizations.json");
        fs::write(&path, "[]").unwrap();
        let current = revision(&path);

        // 文件没动 → 放行
        assert!(ensure_expected_revision(&path, &current).is_ok());

        // 别的程序改过 → 旧 revision 必须被拒，而不是静默 last-writer-wins
        fs::write(&path, r#"[{"changed":true}]"#).unwrap();
        let error = ensure_expected_revision(&path, &current).unwrap_err();
        assert!(error.contains("已被其他程序修改"), "{error}");
        assert!(error.contains("尚未写入"), "{error}");

        // 文件不存在时 revision 是 "missing"，不是崩溃
        let absent = dir.join("nope.json");
        assert_eq!(revision(&absent), "missing");
        assert!(ensure_expected_revision(&absent, "missing").is_ok());
        assert!(ensure_expected_revision(&absent, "whatever").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_requests_require_a_revision_so_the_guard_cannot_be_skipped() {
        // 请求体缺 expected_revision 时必须反序列化失败。
        // 否则调用方只要不传这个字段就能绕过并发保护，而保护还显示为"已存在"。
        let org = serde_json::json!({ "name": "org", "url": "https://gw.example.com" });
        assert!(serde_json::from_value::<SaveWorkBuddyOrganizationRequest>(org).is_err());

        let gateway = serde_json::json!({ "url": "https://gw.example.com" });
        assert!(serde_json::from_value::<SaveWorkBuddyGatewayRequest>(gateway).is_err());

        // 只校验组织/网关文件还不够：这两个操作也会改 models.json。
        let org_without_models_revision = serde_json::json!({
            "expectedRevision": "org-rev",
            "name": "org",
            "url": "https://gw.example.com"
        });
        assert!(serde_json::from_value::<SaveWorkBuddyOrganizationRequest>(
            org_without_models_revision
        )
        .is_err());

        let apply_without_revisions = serde_json::json!({
            "organizationId": "org",
            "models": []
        });
        assert!(
            serde_json::from_value::<ApplyWorkBuddyOrganizationModelsRequest>(
                apply_without_revisions
            )
            .is_err()
        );
    }

    #[test]
    fn ownership_flips_presence_semantics() {
        // 回归（审查 Critical）：字段语义是"由本应用新增"，而查询返回的是"已存在"。
        // 方向写反的后果是不对称的：误删别人的信任链 vs 留下自己的 —— 前者严重得多。
        assert_eq!(
            ownership_from_presence(Some(true)),
            Some(false),
            "已存在 ⇒ 不是我们加的"
        );
        assert_eq!(
            ownership_from_presence(Some(false)),
            Some(true),
            "不存在 ⇒ 是我们加的"
        );
        // 查不出来就别猜：既不算我们加的，也不算别人的
        assert_eq!(ownership_from_presence(None), None);
    }

    // ---------------- CA 撤销（审查 F3） ----------------

    fn pem(tag: &str) -> String {
        format!("-----BEGIN CERTIFICATE-----\n{tag}\n-----END CERTIFICATE-----")
    }

    #[test]
    fn remove_managed_pem_blocks_keeps_foreign_certificates() {
        // sync_workbuddy_ca_bundle 是**合并**写入的，撤销时绝不能顺手删掉别人的证书
        let managed = format!("{}\n", pem("MANAGED-A"));
        let existing = format!(
            "{}\n{}\n{}\n",
            pem("FOREIGN-1"),
            pem("MANAGED-A"),
            pem("FOREIGN-2")
        );
        let left = remove_managed_pem_blocks(&existing, &managed);
        assert!(!left.contains("MANAGED-A"), "{left}");
        assert!(left.contains("FOREIGN-1"), "{left}");
        assert!(left.contains("FOREIGN-2"), "{left}");
    }

    #[test]
    fn certificate_cleanup_preserves_valid_pem_encoding_and_comments() {
        let existing = format!("# keep this comment\n{TEST_CA_PEM}\n{}\n", pem("REMOVED"));
        let left = remove_managed_pem_blocks(&existing, &pem("REMOVED"));
        assert!(left.contains(TEST_CA_PEM));
        assert!(left.starts_with("# keep this comment"));
        reqwest::Certificate::from_pem(pem_certificates(&left)[0].as_bytes()).unwrap();
    }

    #[test]
    fn remove_managed_pem_blocks_handles_multiple_and_crlf() {
        // 换行风格不同也要能比对上是同一张证书
        let managed = format!("{}\r\n{}\r\n", pem("M1"), pem("M2"));
        let existing = format!("{}\n{}\n", pem("M1"), pem("M2"));
        assert_eq!(remove_managed_pem_blocks(&existing, &managed), "");
    }

    #[test]
    fn remove_managed_pem_blocks_is_noop_without_managed_certs() {
        let existing = format!("{}\n", pem("FOREIGN"));
        // 没有可撤销的内容时，绝不能把别人的证书删掉
        assert_eq!(remove_managed_pem_blocks(&existing, ""), existing);
        assert_eq!(
            remove_managed_pem_blocks(&existing, "no pem here"),
            existing
        );
    }

    #[test]
    fn pem_blocks_ignores_truncated_input() {
        // 只有 BEGIN 没有 END：不能 panic，也不能凭空造出一块
        assert!(pem_blocks("-----BEGIN CERTIFICATE-----\nAAAA").is_empty());
        assert!(pem_blocks("").is_empty());
        assert_eq!(pem_blocks(&pem("X")).len(), 1);
    }

    #[test]
    fn credential_file_paths_cover_write_document_artifacts() {
        // 回归：曾经只返回 3 个主文件，于是 write_document 派生出的
        // backup / previous / tmp（同样含明文 apiKey）完全不在权限检查范围内，
        // 界面却照样显示"含密钥的文件均仅限本人读取"。
        let paths = credential_file_paths();
        assert!(
            paths.len() >= 12,
            "至少覆盖 3 个主文件 × 4 种产物：{paths:?}"
        );
        let names: Vec<String> = paths
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        for base in [
            "cc-manager-gateway.json",
            "cc-manager-organizations.json",
            "models.json",
        ] {
            assert!(names.contains(&base.to_string()), "缺少主文件 {base}");
        }
        assert!(
            names.iter().any(|name| name.contains("backup")),
            "{names:?}"
        );
        assert!(
            names.iter().any(|name| name.contains("previous")),
            "{names:?}"
        );
        assert!(names.iter().any(|name| name.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn write_document_uses_unique_temporary_paths() {
        let path = Path::new("models.json");
        let first = document_temp_path(path);
        let second = document_temp_path(path);
        assert_ne!(first, second);
        for candidate in [first, second] {
            let name = candidate.file_name().unwrap().to_string_lossy();
            assert!(name.starts_with("models.cc-manager."), "{name}");
            assert!(name.ends_with(".tmp"), "{name}");
        }
    }

    #[test]
    fn write_guard_serializes_complete_mutations() {
        use std::sync::mpsc;

        let first = write_guard();
        let (sent, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _second = write_guard();
            sent.send(()).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
        drop(first);
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn every_workbuddy_config_mutation_takes_the_write_guard() {
        let source = include_str!("workbuddy.rs");
        for name in [
            "set_workbuddy_executable",
            "save_workbuddy_gateway",
            "save_workbuddy_organization",
            "delete_workbuddy_organization",
            "apply_workbuddy_organization_models",
            "save_workbuddy_model",
            "delete_workbuddy_model",
        ] {
            let signature = format!("pub fn {name}");
            let start = source
                .find(&signature)
                .unwrap_or_else(|| panic!("缺少 {name}"));
            let tail = &source[start..];
            let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
            assert!(
                tail[..end].contains("let _guard = write_guard();"),
                "{name} 必须在读取配置之前获取完整写锁"
            );
        }
    }

    #[test]
    fn platform_ui_covers_windows_and_macos_without_frontend_branching() {
        let windows = platform_ui("windows");
        assert_eq!(windows.executable_extensions, ["exe"]);
        assert!(windows
            .ca_import_consequences
            .iter()
            .any(|line| line.contains("Windows")));

        let macos = platform_ui("macos");
        assert_eq!(macos.executable_extensions, ["app"]);
        assert!(macos
            .ca_import_consequences
            .iter()
            .any(|line| line.contains("macOS")));
    }

    #[cfg(unix)]
    #[test]
    fn write_document_restricts_every_artifact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "ccm-wb-perm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");
        // 关键场景：这是个 app 之外创建的 0644 旧文件（WorkBuddy 自己写的）。
        // fs::copy 会连同 0644 一起复制给 backup，所以只收紧最终文件是不够的。
        fs::write(&path, "{\"models\":[]}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_document(&path, &serde_json::json!({ "models": [] })).unwrap();

        // 成功路径结束后留下的产物：主文件 + backup（previous 会被清掉）
        for target in [path.clone(), path.with_extension("cc-manager.backup.json")] {
            assert!(target.exists(), "{} 应当存在", target.display());
            let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                mode,
                0o600,
                "{} 权限是 {:o}，应被收紧到 600（含明文 apiKey）",
                target.display(),
                mode
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_document_normalizes_empty_array_and_rejects_unknown_shapes() {
        let dir = std::env::temp_dir().join(format!(
            "ccm-wb-doc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models.json");

        // WorkBuddy 升级后把无模型的配置重置成顶层空数组：必须当成空配置放行，
        // 而不是报"顶层必须是 JSON 对象"、禁止保存（这正是用户看到的红叉根因）。
        fs::write(&path, "[]").unwrap();
        assert_eq!(read_document(&path).unwrap(), Value::Object(Map::new()));

        // 合法对象原样返回。
        fs::write(&path, r#"{"models":[]}"#).unwrap();
        assert_eq!(
            read_document(&path).unwrap(),
            serde_json::json!({ "models": [] })
        );

        // 非空数组是未知结构（模型历来存在对象的 models 字段里），保持报错以免误覆盖。
        fs::write(&path, "[1]").unwrap();
        assert!(read_document(&path).is_err());

        // 其它标量顶层同样拒绝。
        fs::write(&path, "42").unwrap();
        assert!(read_document(&path).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn derives_workbuddy_cli_ca_path_from_executable() {
        let executable = Path::new(r"D:\Program Files\WorkBuddy\WorkBuddy.exe");
        assert_eq!(
            workbuddy_ca_path_for_platform(executable, WorkBuddyPlatform::Windows),
            Some(PathBuf::from(
                r"D:\Program Files\WorkBuddy\resources\app.asar.unpacked\cli\ca.pem"
            ))
        );
    }

    #[test]
    fn derives_workbuddy_cli_ca_path_from_macos_app() {
        let executable = Path::new("/Applications/WorkBuddy.app");
        assert_eq!(
            workbuddy_ca_path_for_platform(executable, WorkBuddyPlatform::Macos),
            Some(PathBuf::from(
                "/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/cli/ca.pem"
            ))
        );
    }

    #[test]
    fn derives_macos_app_from_running_process_command() {
        assert_eq!(
            macos_app_from_process_command(
                "/Users/test/Apps/WorkBuddy.app/Contents/MacOS/WorkBuddy --flag"
            ),
            Some(PathBuf::from("/Users/test/Apps/WorkBuddy.app"))
        );
        assert_eq!(
            macos_app_from_process_command("/System/Applications/Finder.app/Contents/MacOS/Finder"),
            None
        );
    }

    #[test]
    fn cleans_registry_and_manual_executable_paths() {
        assert_eq!(
            clean_executable_path(r#"  "D:\Apps\WorkBuddy\WorkBuddy.exe",0  "#),
            Some(PathBuf::from(r"D:\Apps\WorkBuddy\WorkBuddy.exe"))
        );
        assert_eq!(clean_executable_path("   "), None);
    }

    #[test]
    fn rejects_missing_workbuddy_executable() {
        assert!(!is_workbuddy_executable(Path::new(
            r"Z:\missing\WorkBuddy.exe"
        )));
    }

    #[test]
    fn merges_managed_certificates_without_removing_or_duplicating_existing_ones() {
        let existing = "-----BEGIN CERTIFICATE-----\nEXISTING\n-----END CERTIFICATE-----\n";
        let managed = "-----BEGIN CERTIFICATE-----\nEXISTING\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nMANAGED\n-----END CERTIFICATE-----\n";

        let merged = merge_ca_bundle(existing, managed);

        assert_eq!(merged.matches("EXISTING").count(), 1);
        assert_eq!(merged.matches("MANAGED").count(), 1);
        assert_eq!(merge_ca_bundle(&merged, managed), merged);
    }
}
