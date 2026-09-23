use super::{McpLocator, McpPaths, McpService, McpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

const PREFERENCES_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpUpdateInfo {
    pub locator: McpLocator,
    pub supported: bool,
    pub check_enabled: bool,
    pub ecosystem: Option<String>,
    pub package_name: Option<String>,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub reason: String,
    /// 已把包版本替换为 latest_version 的完整配置。前端仍必须走现有 preview/apply，
    /// 不能直接写文件。
    pub next_config: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpUpdateReport {
    pub entries: Vec<McpUpdateInfo>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UpdatePreferences {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, bool>,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ecosystem {
    Npm,
    Pypi,
}

impl Ecosystem {
    fn label(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pypi => "PyPI",
        }
    }
}

#[derive(Clone, Debug)]
enum SpecLocation {
    Arg(usize),
    ArgPrefix { index: usize, prefix: String },
}

#[derive(Clone, Debug)]
struct PackageSource {
    ecosystem: Ecosystem,
    package_name: String,
    current_version: Option<String>,
    location: SpecLocation,
}

fn locator_key(locator: &McpLocator) -> String {
    serde_json::to_string(locator).unwrap_or_else(|_| {
        format!(
            "{:?}:{}:{}:{}",
            locator.scope,
            locator.name,
            locator.instance_id.as_deref().unwrap_or_default(),
            locator.project_path.as_deref().unwrap_or_default()
        )
    })
}

fn read_preferences(paths: &McpPaths) -> Result<UpdatePreferences, String> {
    let path = paths.update_preferences();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UpdatePreferences::default())
        }
        Err(error) => return Err(format!("读取 MCP 更新设置失败：{error}")),
    };
    let preferences: UpdatePreferences =
        serde_json::from_str(&text).map_err(|error| format!("MCP 更新设置格式错误：{error}"))?;
    if preferences.version != PREFERENCES_VERSION {
        return Err(format!(
            "不支持的 MCP 更新设置版本：{}",
            preferences.version
        ));
    }
    Ok(preferences)
}

fn write_preferences(paths: &McpPaths, preferences: &UpdatePreferences) -> Result<(), String> {
    let path = paths.update_preferences();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建设置目录失败：{error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(preferences)
        .map_err(|error| format!("序列化 MCP 更新设置失败：{error}"))?;
    crate::sync::write_bytes_atomic(&path, &bytes)
        .map_err(|error| format!("保存 MCP 更新设置失败：{error}"))?;
    crate::sync::restrict_credential_permissions(&path)
        .map_err(|error| format!("收紧 MCP 更新设置权限失败：{error}"))?;
    Ok(())
}

fn command_name(config: &Map<String, Value>) -> String {
    config
        .get("command")
        .and_then(Value::as_str)
        .and_then(|command| {
            std::path::Path::new(command)
                .file_stem()
                .and_then(|name| name.to_str())
        })
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn config_args(config: &Map<String, Value>) -> Vec<String> {
    config
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn exact_version(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_prefix('v').unwrap_or(value);
    if value.is_empty()
        || value.eq_ignore_ascii_case("latest")
        || value
            .chars()
            .any(|character| matches!(character, '^' | '~' | '*' | '<' | '>' | '=' | ' ' | ','))
    {
        return None;
    }
    value
        .chars()
        .next()?
        .is_ascii_digit()
        .then(|| value.to_string())
}

fn parse_npm_spec(spec: &str) -> Option<(String, Option<String>)> {
    if spec.starts_with('-') || spec.contains("//") || spec.starts_with('.') {
        return None;
    }
    let split = if spec.starts_with('@') {
        let slash = spec.find('/')?;
        spec[slash + 1..]
            .rfind('@')
            .map(|offset| slash + 1 + offset)
    } else {
        spec.rfind('@').filter(|index| *index > 0)
    };
    match split {
        Some(index) => Some((spec[..index].to_string(), exact_version(&spec[index + 1..]))),
        None => Some((spec.to_string(), None)),
    }
}

fn parse_pypi_spec(spec: &str) -> Option<(String, Option<String>)> {
    if spec.starts_with('-') || spec.contains("//") || spec.starts_with('.') {
        return None;
    }
    if let Some((name, version)) = spec.split_once("==") {
        return (!name.is_empty()).then(|| (name.to_string(), exact_version(version)));
    }
    if let Some((name, version)) = spec.rsplit_once('@') {
        if !name.is_empty() {
            return Some((name.to_string(), exact_version(version)));
        }
    }
    let end = spec
        .find(|character: char| matches!(character, '<' | '>' | '~' | '!' | '=' | '['))
        .unwrap_or(spec.len());
    let name = spec[..end].trim();
    (!name.is_empty()).then(|| (name.to_string(), None))
}

fn npm_source(args: &[String], start: usize) -> Option<PackageSource> {
    let mut index = start;
    while index < args.len() {
        let arg = &args[index];
        if let Some(spec) = arg.strip_prefix("--package=") {
            let (package_name, current_version) = parse_npm_spec(spec)?;
            return Some(PackageSource {
                ecosystem: Ecosystem::Npm,
                package_name,
                current_version,
                location: SpecLocation::ArgPrefix {
                    index,
                    prefix: "--package=".into(),
                },
            });
        }
        if matches!(arg.as_str(), "--package" | "-p") {
            let spec_index = index + 1;
            let (package_name, current_version) = parse_npm_spec(args.get(spec_index)?)?;
            return Some(PackageSource {
                ecosystem: Ecosystem::Npm,
                package_name,
                current_version,
                location: SpecLocation::Arg(spec_index),
            });
        }
        if arg == "--" || matches!(arg.as_str(), "-y" | "--yes" | "--quiet") {
            index += 1;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        let (package_name, current_version) = parse_npm_spec(arg)?;
        return Some(PackageSource {
            ecosystem: Ecosystem::Npm,
            package_name,
            current_version,
            location: SpecLocation::Arg(index),
        });
    }
    None
}

fn pypi_source(args: &[String], start: usize) -> Option<PackageSource> {
    let mut index = start;
    while index < args.len() {
        let arg = &args[index];
        if let Some(spec) = arg.strip_prefix("--from=") {
            let (package_name, current_version) = parse_pypi_spec(spec)?;
            return Some(PackageSource {
                ecosystem: Ecosystem::Pypi,
                package_name,
                current_version,
                location: SpecLocation::ArgPrefix {
                    index,
                    prefix: "--from=".into(),
                },
            });
        }
        if arg == "--from" {
            let spec_index = index + 1;
            let (package_name, current_version) = parse_pypi_spec(args.get(spec_index)?)?;
            return Some(PackageSource {
                ecosystem: Ecosystem::Pypi,
                package_name,
                current_version,
                location: SpecLocation::Arg(spec_index),
            });
        }
        if arg == "--" || arg.starts_with('-') {
            index += 1;
            continue;
        }
        let (package_name, current_version) = parse_pypi_spec(arg)?;
        return Some(PackageSource {
            ecosystem: Ecosystem::Pypi,
            package_name,
            current_version,
            location: SpecLocation::Arg(index),
        });
    }
    None
}

fn package_source(service: &McpService) -> Option<PackageSource> {
    if service.transport != McpTransport::Stdio {
        return None;
    }
    let command = command_name(&service.config);
    let args = config_args(&service.config);
    match command.as_str() {
        "npx" => npm_source(&args, 0),
        "npm" if matches!(args.first().map(String::as_str), Some("exec" | "x")) => {
            npm_source(&args, 1)
        }
        "uvx" => pypi_source(&args, 0),
        "uv" if args.get(0).map(String::as_str) == Some("tool")
            && args.get(1).map(String::as_str) == Some("run") =>
        {
            pypi_source(&args, 2)
        }
        _ => None,
    }
}

fn unsupported_reason(service: &McpService) -> String {
    match service.transport {
        McpTransport::Http | McpTransport::Sse | McpTransport::Ws => {
            "远程服务由服务提供方维护，无需在本机更新".into()
        }
        McpTransport::Stdio => "本地命令未包含可验证的 npm 或 PyPI 包来源".into(),
        McpTransport::Unknown => "配置来源无法识别".into(),
    }
}

fn base_info(
    service: &McpService,
    preferences: &UpdatePreferences,
) -> (McpUpdateInfo, Option<PackageSource>) {
    let source = package_source(service);
    let supported = source.is_some();
    let check_enabled = supported
        && preferences
            .entries
            .get(&locator_key(&service.locator))
            .copied()
            .unwrap_or(true);
    let info = match &source {
        Some(source) => McpUpdateInfo {
            locator: service.locator.clone(),
            supported,
            check_enabled,
            ecosystem: Some(source.ecosystem.label().into()),
            package_name: Some(source.package_name.clone()),
            current_version: source.current_version.clone(),
            latest_version: None,
            update_available: false,
            reason: if source.current_version.is_some() {
                format!("由 {} 包管理，可检测并更新", source.ecosystem.label())
            } else {
                format!(
                    "由 {} 包管理；当前配置未固定版本，只检测最新版本",
                    source.ecosystem.label()
                )
            },
            next_config: None,
        },
        None => McpUpdateInfo {
            locator: service.locator.clone(),
            supported: false,
            check_enabled: false,
            ecosystem: None,
            package_name: None,
            current_version: None,
            latest_version: None,
            update_available: false,
            reason: unsupported_reason(service),
            next_config: None,
        },
    };
    (info, source)
}

fn load_services(paths: &McpPaths, instances: &[String]) -> Vec<McpService> {
    super::storage::collect_state(paths, instances).services
}

pub(super) fn list_update_info(
    paths: &McpPaths,
    instances: &[String],
) -> Result<McpUpdateReport, String> {
    let preferences = read_preferences(paths)?;
    Ok(McpUpdateReport {
        entries: load_services(paths, instances)
            .iter()
            .map(|service| base_info(service, &preferences).0)
            .collect(),
        errors: Vec::new(),
    })
}

fn registry_latest(
    client: &reqwest::blocking::Client,
    source: &PackageSource,
) -> Result<String, String> {
    let url = match source.ecosystem {
        Ecosystem::Npm => {
            let package = url::form_urlencoded::byte_serialize(source.package_name.as_bytes())
                .collect::<String>();
            format!("https://registry.npmjs.org/{package}/latest")
        }
        Ecosystem::Pypi => {
            let package = url::form_urlencoded::byte_serialize(source.package_name.as_bytes())
                .collect::<String>();
            format!("https://pypi.org/pypi/{package}/json")
        }
    };
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("请求失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("包不存在或仓库不可用：{error}"))?;
    let body: Value = response
        .json()
        .map_err(|error| format!("仓库响应格式错误：{error}"))?;
    let version = match source.ecosystem {
        Ecosystem::Npm => body.get("version"),
        Ecosystem::Pypi => body.get("info").and_then(|info| info.get("version")),
    }
    .and_then(Value::as_str)
    .filter(|value| !value.trim().is_empty())
    .ok_or_else(|| "仓库响应缺少版本号".to_string())?;
    Ok(version.to_string())
}

fn release_parts(version: &str) -> Option<(Vec<u64>, bool)> {
    let version = version.trim().trim_start_matches('v');
    let numeric_len = version
        .char_indices()
        .find(|(_, character)| !character.is_ascii_digit() && *character != '.')
        .map(|(index, _)| index)
        .unwrap_or(version.len());
    let numeric = &version[..numeric_len];
    if numeric.is_empty() {
        return None;
    }
    let parts = numeric
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some((parts, numeric_len == version.len()))
}

fn version_is_newer(ecosystem: Ecosystem, current: &str, latest: &str) -> bool {
    if current == latest {
        return false;
    }
    if ecosystem == Ecosystem::Npm {
        if let (Ok(current), Ok(latest)) = (
            semver::Version::parse(current.trim_start_matches('v')),
            semver::Version::parse(latest.trim_start_matches('v')),
        ) {
            return latest > current;
        }
    }
    match (release_parts(current), release_parts(latest)) {
        (Some((mut current, current_stable)), Some((mut latest, latest_stable))) => {
            let width = current.len().max(latest.len());
            current.resize(width, 0);
            latest.resize(width, 0);
            match latest.cmp(&current) {
                Ordering::Greater => true,
                Ordering::Equal => latest_stable && !current_stable,
                Ordering::Less => false,
            }
        }
        _ => false,
    }
}

fn config_with_version(
    service: &McpService,
    source: &PackageSource,
    latest: &str,
) -> Option<Map<String, Value>> {
    source.current_version.as_ref()?;
    let mut config = service.config.clone();
    let mut args = config_args(&config);
    let spec = match source.ecosystem {
        Ecosystem::Npm => format!("{}@{latest}", source.package_name),
        Ecosystem::Pypi => format!("{}=={latest}", source.package_name),
    };
    match &source.location {
        SpecLocation::Arg(index) => *args.get_mut(*index)? = spec,
        SpecLocation::ArgPrefix { index, prefix } => {
            *args.get_mut(*index)? = format!("{prefix}{spec}")
        }
    }
    config.insert(
        "args".into(),
        Value::Array(args.into_iter().map(Value::String).collect()),
    );
    Some(config)
}

pub(super) fn check_updates(
    paths: &McpPaths,
    instances: &[String],
    target: Option<&McpLocator>,
) -> Result<McpUpdateReport, String> {
    let preferences = read_preferences(paths)?;
    let services = load_services(paths, instances);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| format!("创建版本检测客户端失败：{error}"))?;
    let mut cache = BTreeMap::<(String, String), Result<String, String>>::new();
    let mut entries = Vec::new();
    let mut errors = Vec::new();

    for service in &services {
        if target.is_some_and(|target| target != &service.locator) {
            continue;
        }
        let (mut info, source) = base_info(service, &preferences);
        let Some(source) = source else {
            entries.push(info);
            continue;
        };
        if !info.check_enabled {
            entries.push(info);
            continue;
        }
        let cache_key = (
            source.ecosystem.label().to_string(),
            source.package_name.clone(),
        );
        let result = cache
            .entry(cache_key)
            .or_insert_with(|| registry_latest(&client, &source))
            .clone();
        match result {
            Ok(latest) => {
                info.update_available = source
                    .current_version
                    .as_deref()
                    .is_some_and(|current| version_is_newer(source.ecosystem, current, &latest));
                if info.update_available {
                    info.next_config = config_with_version(service, &source, &latest);
                }
                info.latest_version = Some(latest);
            }
            Err(error) => {
                errors.push(format!("{}：{}", service.locator.name, error));
            }
        }
        entries.push(info);
    }
    Ok(McpUpdateReport { entries, errors })
}

pub(super) fn set_update_check(
    paths: &McpPaths,
    instances: &[String],
    target: &McpLocator,
    enabled: bool,
) -> Result<McpUpdateInfo, String> {
    let services = load_services(paths, instances);
    let service = services
        .iter()
        .find(|service| &service.locator == target)
        .ok_or_else(|| "MCP 配置已变化，请刷新后重试".to_string())?;
    let mut preferences = read_preferences(paths)?;
    let (info, source) = base_info(service, &preferences);
    if source.is_none() {
        return Err(info.reason);
    }
    preferences.entries.insert(locator_key(target), enabled);
    write_preferences(paths, &preferences)?;
    Ok(base_info(service, &preferences).0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{McpEffectiveState, McpScope};

    fn service(command: &str, args: &[&str]) -> McpService {
        McpService {
            locator: McpLocator {
                scope: McpScope::User,
                name: "demo".into(),
                instance_id: None,
                project_path: None,
            },
            transport: McpTransport::Stdio,
            raw_transport: None,
            config: serde_json::from_value(serde_json::json!({
                "command": command,
                "args": args,
            }))
            .unwrap(),
            enabled: true,
            effective_state: McpEffectiveState::Effective,
            shadowed_by: Vec::new(),
            shadowed_context_count: 0,
            source_id: "user".into(),
            revision: "r".into(),
            sensitive_paths: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn recognizes_scoped_npx_package_and_rewrites_only_the_version() {
        let service = service(
            "npx.cmd",
            &["-y", "@scope/server@1.2.3", "--token", "secret"],
        );
        let source = package_source(&service).unwrap();
        assert_eq!(source.package_name, "@scope/server");
        assert_eq!(source.current_version.as_deref(), Some("1.2.3"));
        let next = config_with_version(&service, &source, "1.4.0").unwrap();
        assert_eq!(next["args"][1], "@scope/server@1.4.0");
        assert_eq!(next["args"][3], "secret");
    }

    #[test]
    fn recognizes_uvx_from_package() {
        let service = service("uvx", &["--from", "mcp-demo==0.8.0", "demo"]);
        let source = package_source(&service).unwrap();
        assert_eq!(source.ecosystem, Ecosystem::Pypi);
        assert_eq!(source.package_name, "mcp-demo");
        assert_eq!(source.current_version.as_deref(), Some("0.8.0"));
    }

    #[test]
    fn bare_and_remote_services_are_not_guessed_as_packages() {
        assert!(package_source(&service("codegraph", &[])).is_none());
        let mut remote = service("", &[]);
        remote.transport = McpTransport::Http;
        remote.config = serde_json::from_value(serde_json::json!({
            "type": "http",
            "url": "https://example.com/mcp"
        }))
        .unwrap();
        assert!(package_source(&remote).is_none());
    }

    #[test]
    fn version_comparison_never_downgrades() {
        assert!(version_is_newer(Ecosystem::Npm, "1.2.0", "1.3.0"));
        assert!(!version_is_newer(Ecosystem::Npm, "2.0.0", "1.9.0"));
        assert!(version_is_newer(Ecosystem::Pypi, "1.0rc1", "1.0"));
    }

    #[test]
    fn update_preference_is_separate_from_mcp_configuration() {
        let root = std::env::temp_dir().join(format!(
            "pathmux-mcp-update-preference-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = McpPaths::for_test(root.clone());
        let locator = McpLocator {
            scope: McpScope::User,
            name: "demo".into(),
            instance_id: None,
            project_path: None,
        };
        let mut preferences = UpdatePreferences::default();
        preferences.entries.insert(locator_key(&locator), false);
        write_preferences(&paths, &preferences).unwrap();

        let loaded = read_preferences(&paths).unwrap();
        assert_eq!(loaded.entries.get(&locator_key(&locator)), Some(&false));
        assert!(paths.update_preferences().exists());
        assert!(!paths.shared_mcp_json().exists());

        std::fs::remove_dir_all(root).unwrap();
    }
}
