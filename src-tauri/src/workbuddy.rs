use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::error::Error as StdError;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const DEFAULT_ENDPOINT: &str = "https://10.0.147.128:8080";
const CREATE_NO_WINDOW: u32 = 0x08000000;
const ORGANIZATION_OWNER_FIELD: &str = "_ccManagerOrganizationId";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyEnvironment {
    found: bool,
    executable_path: Option<String>,
    version: Option<String>,
    config_path: String,
    config_exists: bool,
    config_valid: bool,
    detail: String,
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
    revision: String,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkBuddyOrganizationRequest {
    id: Option<String>,
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
            url: DEFAULT_ENDPOINT.into(),
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
            .unwrap_or(DEFAULT_ENDPOINT)
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
    None
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

fn product_json_for(executable: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        executable
            .parent()
            .map(|dir| dir.join("resources/app.asar.unpacked/cli/product.json"))
    } else if cfg!(target_os = "macos") {
        Some(executable.join("Contents/Resources/app.asar.unpacked/cli/product.json"))
    } else {
        None
    }
}

fn workbuddy_cli_dir_for(executable: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        executable
            .parent()
            .map(|dir| dir.join("resources/app.asar.unpacked/cli"))
    } else if cfg!(target_os = "macos") {
        Some(executable.join("Contents/Resources/app.asar.unpacked/cli"))
    } else {
        None
    }
}

fn workbuddy_ca_path_for(executable: &Path) -> Option<PathBuf> {
    workbuddy_cli_dir_for(executable).map(|dir| dir.join("ca.pem"))
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
    let existing = fs::read_to_string(&target).unwrap_or_default();
    let merged = merge_ca_bundle(&existing, &managed);
    if merged != existing {
        fs::write(&target, merged).map_err(|error| {
            format!(
                "写入 WorkBuddy CA 文件失败（{}）：{error}",
                parent.display()
            )
        })?;
    }
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
    WorkBuddyEnvironment {
        found: executable.is_some(),
        executable_path: executable.as_ref().map(|value| value.display().to_string()),
        version,
        config_path: path.display().to_string(),
        config_exists,
        config_valid,
        detail,
    }
}

fn build_state() -> WorkBuddyState {
    let path = models_path();
    let repair_warning = repair_managed_models().err();
    let document = read_document(&path);
    let gateway = read_gateway_config().unwrap_or_else(|_| StoredGatewayConfig {
        url: DEFAULT_ENDPOINT.into(),
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

fn write_document(path: &Path, document: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or("WorkBuddy 配置目录无效。")?;
    fs::create_dir_all(parent).map_err(|error| format!("创建 WorkBuddy 配置目录失败：{error}"))?;
    let text = serde_json::to_string_pretty(document)
        .map_err(|error| format!("序列化 WorkBuddy 配置失败：{error}"))?;
    let tmp = path.with_extension("cc-manager.tmp");
    let backup = path.with_extension("cc-manager.backup.json");
    let previous = path.with_extension("cc-manager.previous.json");
    fs::write(&tmp, format!("{text}\n"))
        .map_err(|error| format!("写入 WorkBuddy 临时配置失败：{error}"))?;
    if path.exists() {
        fs::copy(path, &backup).map_err(|error| format!("备份 WorkBuddy 配置失败：{error}"))?;
        if previous.exists() {
            fs::remove_file(&previous).map_err(|error| format!("清理旧临时文件失败：{error}"))?;
        }
        fs::rename(path, &previous)
            .map_err(|error| format!("准备替换 WorkBuddy 配置失败：{error}"))?;
    }
    if let Err(error) = fs::rename(&tmp, path) {
        if previous.exists() && !path.exists() {
            let _ = fs::rename(&previous, path);
        }
        let _ = fs::remove_file(&tmp);
        return Err(format!("替换 WorkBuddy 配置失败：{error}"));
    }
    let _ = fs::remove_file(previous);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("限制 WorkBuddy 配置文件权限失败：{error}"))?;
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
    build_state()
}

#[tauri::command]
pub fn set_workbuddy_executable(path: String) -> Result<WorkBuddyState, String> {
    let candidate = clean_executable_path(&path).ok_or("请选择 WorkBuddy.exe。")?;
    if !is_workbuddy_executable(&candidate) {
        return Err(
            "所选文件不是有效的 WorkBuddy.exe，或安装目录中缺少 resources/app.asar.unpacked/cli/product.json。"
                .into(),
        );
    }
    let executable = candidate.canonicalize().unwrap_or(candidate);
    save_executable(&executable)?;

    let certificate = crate::cert_path();
    if certificate.exists() {
        sync_workbuddy_ca_bundle(&executable, &certificate)?;
    }
    Ok(build_state())
}

#[tauri::command]
pub fn save_workbuddy_gateway(
    mut request: SaveWorkBuddyGatewayRequest,
) -> Result<WorkBuddyState, String> {
    request.url = request.url.trim().trim_end_matches('/').to_string();
    let parsed = url::Url::parse(&request.url).map_err(|_| "网关地址不是有效 URL。")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("网关地址必须以 http:// 或 https:// 开头。".into());
    }
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
    Ok(build_state())
}

#[tauri::command]
pub fn save_workbuddy_organization(
    mut request: SaveWorkBuddyOrganizationRequest,
) -> Result<WorkBuddyState, String> {
    validate_organization(
        &mut request.name,
        &mut request.model_prefix,
        &mut request.url,
    )?;
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
    Ok(build_state())
}

#[tauri::command]
pub fn delete_workbuddy_organization(id: String) -> Result<WorkBuddyState, String> {
    let organization = organization_by_id(id.trim())?;
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
    Ok(build_state())
}

#[tauri::command]
pub fn apply_workbuddy_organization_models(
    request: ApplyWorkBuddyOrganizationModelsRequest,
) -> Result<WorkBuddyState, String> {
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
    Ok(build_state())
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
    #[cfg(target_os = "windows")]
    {
        crate::import_cert(path)?;
        let executable = find_executable().ok_or("未检测到 WorkBuddy 安装，无法同步 CLI CA。")?;
        let target = sync_workbuddy_ca_bundle(&executable, &crate::cert_path())?;

        use std::os::windows::process::CommandExt;
        let output = Command::new("certutil.exe")
            .args(["-user", "-addstore", "-f", "Root"])
            .arg(&certificate)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| format!("无法启动 Windows 证书导入：{error}"))?;
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
        return Ok(format!(
            "CA 已同步到 WorkBuddy CLI（{}）。{} 请完全退出 WorkBuddy（包括系统托盘）后重新打开。",
            target.display(),
            windows_note
        ));
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("当前版本仅支持在 Windows 中从界面导入 WorkBuddy CA 证书。".into())
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
            .bearer_auth(key)
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
            return Err(format!(
                "网关模型列表接口返回 HTTP {status}：{}",
                body.chars().take(240).collect::<String>()
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
            return Ok(WorkBuddyCertificateStatus {
                state: "notRequired".into(),
                detail: "当前网关使用 HTTP，不需要 TLS 证书。".into(),
            });
        }
        if parsed.scheme() != "https" {
            return Err("网关地址必须以 http:// 或 https:// 开头。".into());
        }
        match gateway_client()?.get(parsed).send() {
            Ok(_) => {
                let bundle = crate::cert_path();
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
    Ok(build_state())
}

#[tauri::command]
pub fn delete_workbuddy_model(
    request: DeleteWorkBuddyModelRequest,
) -> Result<WorkBuddyState, String> {
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
    Ok(build_state())
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

fn gateway_client() -> Result<reqwest::blocking::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
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
    let mut parsed = url::Url::parse(openai_endpoint).map_err(|_| "API 地址不是有效 URL。")?;
    parsed.set_path("/anthropic/v1/messages");
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn openai_chat_endpoint(gateway_url: &str) -> Result<String, String> {
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

fn classify_response(status: u16, body: &str) -> WorkBuddyTestResult {
    let short = body.chars().take(280).collect::<String>();
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
        Ok(classify_response(status, &response_body))
    })
    .await
    .map_err(|error| format!("WorkBuddy 测试任务异常：{error}"))?
}

fn configure_workbuddy_command(command: &mut Command, certificate: &Path) {
    command.env("NODE_EXTRA_CA_CERTS", certificate);
}

#[tauri::command]
pub fn launch_workbuddy() -> Result<(), String> {
    let executable = find_executable().ok_or("未检测到 WorkBuddy 安装。")?;
    if cfg!(target_os = "windows") {
        let mut command = Command::new(&executable);
        let certificate = crate::cert_path();
        if certificate.is_file() {
            sync_workbuddy_ca_bundle(&executable, &certificate)?;
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
        Command::new("open")
            .arg(executable)
            .spawn()
            .map_err(|error| format!("启动 WorkBuddy 失败：{error}"))?;
    } else {
        return Err("当前平台暂不支持自动打开 WorkBuddy。".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_models_without_exposing_api_key() {
        let document = serde_json::json!({
            "models": [{
                "id": "glm-5.2",
                "name": "GLM 5.2",
                "apiKey": "gw-sk-secret",
                "url": DEFAULT_ENDPOINT,
                "unknown": "kept"
            }],
            "availableModels": ["glm-5.2"]
        });
        let models = parse_models(
            &document,
            &StoredGatewayConfig {
                url: DEFAULT_ENDPOINT.into(),
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
                "url": DEFAULT_ENDPOINT
            }],
            "availableModels": []
        });
        let models = parse_models(&document, &StoredGatewayConfig::default());
        assert!(models[0].visible);
    }

    #[test]
    fn validates_gateway_root_url() {
        let mut model = WorkBuddyModelInput {
            id: "glm-5.2".into(),
            name: String::new(),
            vendor: String::new(),
            url: DEFAULT_ENDPOINT.into(),
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
        model.url = "ftp://10.0.147.128".into();
        assert!(validate_model(&mut model, true).is_err());
    }

    #[test]
    fn derives_anthropic_probe_from_openai_endpoint() {
        assert_eq!(
            anthropic_endpoint(DEFAULT_ENDPOINT).unwrap(),
            "https://10.0.147.128:8080/anthropic/v1/messages"
        );
    }

    #[test]
    fn derives_openai_chat_endpoint_from_gateway_root() {
        assert_eq!(
            openai_chat_endpoint(DEFAULT_ENDPOINT).unwrap(),
            "https://10.0.147.128:8080/v1/chat/completions"
        );
    }

    #[test]
    fn derives_workbuddy_openai_api_base_from_gateway_root() {
        assert_eq!(
            openai_api_base_url("https://10.0.147.128:8080").unwrap(),
            "https://10.0.147.128:8080/v1"
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
            {"id": "old", "name": "旧网关", "url": DEFAULT_ENDPOINT}
        ])));
        assert!(!organization_config_needs_model_prefix(
            &serde_json::json!([
                {"id": "new", "name": "新网关", "modelPrefix": "", "url": DEFAULT_ENDPOINT}
            ])
        ));
    }

    #[test]
    fn derives_openai_models_endpoint_from_gateway_root() {
        assert_eq!(
            openai_models_endpoint(DEFAULT_ENDPOINT).unwrap(),
            "https://10.0.147.128:8080/v1/models"
        );
    }

    #[test]
    fn known_gateway_errors_have_actionable_messages() {
        assert!(classify_response(401, "").detail.contains("Key"));
        assert!(
            classify_response(403, r#"{"error":{"code":"model_access_denied"}}"#)
                .detail
                .contains("模型的调用权限")
        );
        assert!(classify_response(429, "").detail.contains("额度"));
        assert!(classify_response(503, "").detail.contains("上游"));
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

    #[cfg(target_os = "windows")]
    #[test]
    fn derives_workbuddy_cli_ca_path_from_executable() {
        let executable = Path::new(r"D:\Program Files\WorkBuddy\WorkBuddy.exe");
        assert_eq!(
            workbuddy_ca_path_for(executable),
            Some(PathBuf::from(
                r"D:\Program Files\WorkBuddy\resources\app.asar.unpacked\cli\ca.pem"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn derives_workbuddy_cli_ca_path_from_macos_app() {
        let executable = Path::new("/Applications/WorkBuddy.app");
        assert_eq!(
            workbuddy_ca_path_for(executable),
            Some(PathBuf::from(
                "/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/cli/ca.pem"
            ))
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
