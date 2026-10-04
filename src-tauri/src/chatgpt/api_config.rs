//! Per-instance Responses API routing. The official login cache remains untouched.
use super::*;
use std::collections::HashSet;
use std::error::Error;
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;
use toml_edit::{value, DocumentMut, Item, Table};

const PROVIDER: &str = "pathmux_managed_api";
const SNAPSHOT: &str = "api-routing.json";
const CATALOG: &str = "pathmux-model-catalog.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub(super) base_url: String,
    model: String,
    models: Vec<Model>,
    has_key: bool,
    pub active: bool,
    pub(super) compatibility_enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub id: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub manual_model: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    pub id: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Previous {
    model_provider: Option<String>,
    model: Option<String>,
    #[serde(default)]
    api_model: Option<String>,
    #[serde(default)]
    model_catalog_json: Option<String>,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    compatibility_enabled: bool,
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("codex/config.toml")
}
fn snapshot_path(dir: &Path) -> PathBuf {
    dir.join(SNAPSHOT)
}

fn catalog_path(dir: &Path) -> PathBuf {
    dir.join("codex").join(CATALOG)
}

fn document(dir: &Path) -> Result<DocumentMut, String> {
    storage::verify_config(dir)?;
    fs::read_to_string(config_path(dir))
        .map_err(|e| e.to_string())?
        .parse::<DocumentMut>()
        .map_err(|_| "实例配置无法解析".into())
}

fn provider(doc: &DocumentMut) -> Option<&Item> {
    doc.get("model_providers")?.get(PROVIDER)
}

pub fn summary(dir: &Path) -> Result<Option<Summary>, String> {
    let doc = document(dir)?;
    let Some(route) = provider(&doc) else {
        return Ok(None);
    };
    let active = doc.get("model_provider").and_then(Item::as_str) == Some(PROVIDER);
    let previous = storage::read_json::<Previous>(&snapshot_path(dir)).ok();
    let stored_model = previous.as_ref().and_then(|p| p.api_model.clone());
    Ok(Some(Summary {
        base_url: previous
            .as_ref()
            .and_then(|p| p.api_base_url.clone())
            .unwrap_or_else(|| {
                route
                    .get("base_url")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .into()
            }),
        model: if active {
            doc.get("model")
                .and_then(Item::as_str)
                .unwrap_or("")
                .to_string()
        } else {
            stored_model.unwrap_or_default()
        },
        models: read_catalog(&catalog_path(dir)).unwrap_or_default(),
        has_key: route
            .get("experimental_bearer_token")
            .and_then(Item::as_str)
            .is_some_and(|key| !key.is_empty()),
        active,
        compatibility_enabled: active && previous.is_some_and(|p| p.compatibility_enabled),
    }))
}

fn validated_base_url(input: &str) -> Result<String, String> {
    let base_url = input.trim().trim_end_matches('/').to_string();
    let url = url::Url::parse(&base_url).map_err(|_| "请输入有效的 API 地址")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if !matches!(url.scheme(), "https" | "http")
        || (url.scheme() == "http" && !local)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API 地址需使用 HTTPS；本机地址可使用 HTTP，且不能包含凭据或参数".into());
    }
    if url.path().ends_with("/responses") {
        return Err("请填写 API 基础地址（通常以 /v1 结尾），不要包含 /responses".into());
    }
    Ok(base_url)
}

fn validated_key(input: &Option<String>) -> Result<Option<String>, String> {
    let api_key = input
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if api_key
        .as_ref()
        .is_some_and(|s| s.len() > 8192 || s.chars().any(char::is_control))
    {
        return Err("API Key 格式无效".into());
    }
    Ok(api_key)
}

fn key_for(dir: &Path, base_url: &str, new_key: Option<String>) -> Result<String, String> {
    if let Some(key) = new_key {
        return Ok(key);
    }
    let doc = document(dir)?;
    let route = provider(&doc).ok_or("请填写 API Key")?;
    let saved_base_url = storage::read_json::<Previous>(&snapshot_path(dir))
        .ok()
        .and_then(|p| p.api_base_url)
        .or_else(|| {
            route
                .get("base_url")
                .and_then(Item::as_str)
                .map(str::to_owned)
        });
    if saved_base_url.as_deref() != Some(base_url) {
        return Err("API 地址已更改，请重新填写对应的 API Key".into());
    }
    route
        .get("experimental_bearer_token")
        .and_then(Item::as_str)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "请填写 API Key".into())
}

fn is_private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback() || ip.is_unicast_link_local(),
    }
}

pub(super) fn is_private_gateway_url(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Ipv4(ip)) => is_private_address(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_private_address(IpAddr::V6(ip)),
        Some(url::Host::Domain(host)) => {
            if host == "localhost" || host.ends_with(".localhost") {
                return true;
            }
            // Split DNS commonly gives an internal hostname a private VPN address.
            // Bypass the proxy for that destination as well, regardless of brand.
            (host, url.port_or_known_default().unwrap_or(443))
                .to_socket_addrs()
                .map(|addresses| {
                    addresses
                        .into_iter()
                        .any(|address| is_private_address(address.ip()))
                })
                .unwrap_or(false)
        }
        None => false,
    }
}

fn discover_from(base_url: &str, key: &str) -> Result<Vec<Model>, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15));
    let private_gateway = is_private_gateway_url(base_url);
    if private_gateway {
        // The system proxy may not honor macOS proxy exceptions for private IPs.
        // Let the OS route this connection through WireGuard directly.
        builder = builder.no_proxy();
    }
    if let Ok(bundle) = fs::read(crate::union_ca_bundle_path()) {
        if !bundle.is_empty() {
            let certs = reqwest::Certificate::from_pem_bundle(&bundle)
                .map_err(|_| "已导入的 CA 证书无法解析".to_string())?;
            if private_gateway {
                // macOS SecTrust can reject the corporate gateway leaf as noncompliant,
                // even when the imported CA validates it. WebPKI validates against
                // exactly the CA certificates the user imported into PathMux.
                builder = builder.tls_certs_only(certs);
            } else {
                for cert in certs {
                    builder = builder.add_root_certificate(cert);
                }
            }
        }
    }
    let client = builder
        .build()
        .map_err(|_| "无法创建 API 连接".to_string())?;
    let response = client
        .get(format!("{base_url}/models"))
        .bearer_auth(key)
        .send()
        .map_err(|error| {
            let mut message = error.to_string();
            let mut cause = error.source();
            while let Some(next) = cause {
                message.push_str(": ");
                message.push_str(&next.to_string());
                cause = next.source();
            }
            let cause = message.to_ascii_lowercase();
            if cause.contains("certificate") || cause.contains("unknown issuer") {
                "网关证书未受信任，请先在 PathMux 中导入管理员提供的 CA 证书".to_string()
            } else if cause.contains("timed out") || cause.contains("timeout") {
                "连接网关超时，请检查网关地址、网络连接和 VPN 路由".to_string()
            } else {
                format!("读取网关模型列表失败：{message}")
            }
        })?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("网关拒绝 API Key（HTTP 401），请确认地址和密钥属于同一网关".into());
    }
    if !response.status().is_success() {
        return Err(format!("读取网关模型列表失败：HTTP {}", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .take(512 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取网关模型列表失败".to_string())?;
    if bytes.len() > 512 * 1024 {
        return Err("网关模型列表过大".into());
    }
    let payload: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "网关返回的模型列表格式无效".to_string())?;
    let entries = payload
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or("网关未返回 OpenAI 格式的模型列表（data 数组）")?;
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for entry in entries {
        let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) || !seen.insert(id) {
            continue;
        }
        let name = entry
            .get("display_name")
            .or_else(|| entry.get("name"))
            .and_then(serde_json::Value::as_str)
            .filter(|name| {
                !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
            })
            .unwrap_or(id);
        models.push(Model {
            id: id.into(),
            name: name.into(),
        });
        if models.len() >= 256 {
            break;
        }
    }
    if models.is_empty() {
        return Err("网关没有返回可用模型，请确认 /v1/models 接口和 API Key 权限".into());
    }
    Ok(models)
}

pub fn discover(dir: &Path, request: &DiscoverRequest) -> Result<Vec<Model>, String> {
    let base_url = validated_base_url(&request.base_url)?;
    let key = key_for(dir, &base_url, validated_key(&request.api_key)?)?;
    discover_from(&base_url, &key)
}

fn catalog(models: &[Model]) -> serde_json::Value {
    let entries: Vec<_> = models.iter().enumerate().map(|(index, model)| serde_json::json!({
        "slug": model.id,
        "display_name": model.name,
        "description": model.name,
        "base_instructions": "You are a coding assistant. Help the user work in their local workspace.",
        "default_reasoning_level": "high",
        "supported_reasoning_levels": [
            {"effort": "none", "description": "No reasoning"},
            {"effort": "high", "description": "High reasoning"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1000 + index,
        "supports_reasoning_summaries": true,
        "default_reasoning_summary": "none",
        "support_verbosity": false,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": 128000,
        "max_context_window": 128000,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": false
    })).collect();
    serde_json::json!({"models": entries})
}

fn read_catalog(path: &Path) -> Result<Vec<Model>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let payload: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(payload
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(Model {
                id: entry.get("slug")?.as_str()?.to_string(),
                name: entry.get("display_name")?.as_str()?.to_string(),
            })
        })
        .collect())
}

pub fn save(dir: &Path, request: &Request) -> Result<(), String> {
    storage::plain(&snapshot_path(dir))?;
    storage::plain(&catalog_path(dir))?;
    let mut doc = document(dir)?;
    let snapshot = snapshot_path(dir);
    if provider(&doc).is_some() && !snapshot.exists() {
        return Err("发现同名的现有模型服务配置，无法确认归属；请先检查实例配置".into());
    }
    if catalog_path(dir).exists() && !snapshot.exists() {
        return Err("发现同名的现有模型目录文件，无法确认归属；请先检查实例配置".into());
    }
    let base_url = validated_base_url(&request.base_url)?;
    let model = request.model.trim().to_string();
    let key = key_for(dir, &base_url, validated_key(&request.api_key)?)?;
    let models = if request.manual_model {
        if model.is_empty() || model.len() > 128 || model.chars().any(char::is_control) {
            return Err("请输入有效的模型 ID".into());
        }
        vec![Model {
            id: model.clone(),
            name: model.clone(),
        }]
    } else {
        discover_from(&base_url, &key)?
    };
    if !models.iter().any(|item| item.id == model) {
        return Err("所选模型不在网关返回的列表中，请重新读取模型".into());
    }
    if !snapshot.exists() {
        let previous = Previous {
            model_provider: doc
                .get("model_provider")
                .and_then(Item::as_str)
                .map(str::to_owned),
            model: doc.get("model").and_then(Item::as_str).map(str::to_owned),
            api_model: Some(model.clone()),
            model_catalog_json: doc
                .get("model_catalog_json")
                .and_then(Item::as_str)
                .map(str::to_owned),
            api_base_url: Some(base_url.clone()),
            compatibility_enabled: false,
        };
        storage::write_json(&snapshot, &previous)?;
    } else {
        let mut previous: Previous = storage::read_json(&snapshot)?;
        previous.api_model = Some(model.clone());
        previous.api_base_url = Some(base_url.clone());
        storage::write_json(&snapshot, &previous)?;
    }
    let providers = doc
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    let table = providers.as_table_mut().ok_or("现有模型服务配置格式无效")?;
    let route = table.entry(PROVIDER).or_insert(Item::Table(Table::new()));
    let route = route.as_table_mut().ok_or("当前 API 配置格式无效")?;
    route["name"] = value("PathMux API");
    route["base_url"] = value(base_url);
    route["wire_api"] = value("responses");
    route["requires_openai_auth"] = value(false);
    route["experimental_bearer_token"] = value(key);
    doc["model_provider"] = value(PROVIDER);
    doc["model"] = value(model);
    doc["model_catalog_json"] = value(catalog_path(dir).to_string_lossy().as_ref());
    crate::sync::write_bytes_atomic(
        &catalog_path(dir),
        serde_json::to_vec_pretty(&catalog(&models))
            .map_err(|e| e.to_string())?
            .as_slice(),
    )
    .map_err(|e| e.to_string())?;
    crate::sync::write_bytes_atomic(&config_path(dir), doc.to_string().as_bytes())
        .map_err(|e| e.to_string())
}

pub fn compatibility(dir: &Path) -> Result<Option<(String, String, bool)>, String> {
    let doc = document(dir)?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER) {
        return Ok(None);
    }
    let previous: Previous = storage::read_json(&snapshot_path(dir))?;
    let route = provider(&doc).ok_or("实例缺少 API 配置")?;
    let base_url = previous
        .api_base_url
        .or_else(|| {
            route
                .get("base_url")
                .and_then(Item::as_str)
                .filter(|url| !url.starts_with("http://127.0.0.1:"))
                .map(str::to_owned)
        })
        .ok_or("实例缺少 API 网关地址，请重新保存 API 配置")?;
    let key = route
        .get("experimental_bearer_token")
        .and_then(Item::as_str)
        .ok_or("实例缺少 API Key")?;
    Ok(Some((
        base_url,
        key.to_owned(),
        previous.compatibility_enabled,
    )))
}

pub fn set_compatibility(dir: &Path, enabled: bool) -> Result<(), String> {
    let Some((base_url, _, _)) = compatibility(dir)? else {
        return Err("仅 API 登录模式可以使用兼容模式".into());
    };
    let mut previous: Previous = storage::read_json(&snapshot_path(dir))?;
    previous.api_base_url = Some(base_url.clone());
    previous.compatibility_enabled = enabled;
    // A stopped instance can safely return to direct routing. The bridge URL
    // is selected afresh when the instance next launches in compatibility mode.
    let mut doc = document(dir)?;
    doc["model_providers"][PROVIDER]["base_url"] = value(base_url);
    crate::sync::write_bytes_atomic(&config_path(dir), doc.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    storage::write_json(&snapshot_path(dir), &previous)
}

pub fn set_runtime_bridge_url(dir: &Path, port: u16) -> Result<(), String> {
    let Some((_, _, true)) = compatibility(dir)? else {
        return Err("实例未启用 API 兼容模式".into());
    };
    let mut doc = document(dir)?;
    doc["model_providers"][PROVIDER]["base_url"] = value(format!("http://127.0.0.1:{port}/v1"));
    crate::sync::write_bytes_atomic(&config_path(dir), doc.to_string().as_bytes())
        .map_err(|e| e.to_string())
}

pub fn use_account(dir: &Path) -> Result<(), String> {
    storage::plain(&snapshot_path(dir))?;
    let mut doc = document(dir)?;
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER) {
        return Ok(());
    }
    let snapshot = snapshot_path(dir);
    if !snapshot.exists() {
        return Err("缺少 API 配置前的登录设置，无法安全恢复；请检查实例配置".into());
    }
    let previous: Previous = storage::read_json(&snapshot)?;
    match previous.model_provider {
        Some(provider) => doc["model_provider"] = value(provider),
        None => {
            doc.remove("model_provider");
        }
    }
    match previous.model {
        Some(model) => doc["model"] = value(model),
        None => {
            doc.remove("model");
        }
    }
    match previous.model_catalog_json {
        Some(path) => doc["model_catalog_json"] = value(path),
        None => {
            doc.remove("model_catalog_json");
        }
    }
    crate::sync::write_bytes_atomic(&config_path(dir), doc.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn private_vpn_gateways_bypass_system_proxy() {
        // Synthetic addresses cover private ranges without retaining a real gateway.
        for ip in [
            std::net::Ipv4Addr::new(10, 42, 0, 7),
            std::net::Ipv4Addr::new(172, 19, 0, 7),
            std::net::Ipv4Addr::new(192, 168, 42, 7),
        ] {
            let base_url = format!("https://{ip}/v1");
            assert!(is_private_gateway_url(&base_url), "{base_url}");
        }
        for base_url in [
            "http://127.0.0.1:9000/v1",
            "https://[fd00::1]/v1",
            "https://gateway.localhost/v1",
        ] {
            assert!(is_private_gateway_url(base_url), "{base_url}");
        }
        for base_url in ["https://api.openai.com/v1", "https://8.8.8.8/v1"] {
            assert!(!is_private_gateway_url(base_url), "{base_url}");
        }
    }

    fn mock_models(requests: usize) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let task = std::thread::spawn(move || {
            for incoming in listener.incoming().take(requests) {
                let mut socket = incoming.unwrap();
                let mut request = [0; 4096];
                let length = socket.read(&mut request).unwrap();
                let text = String::from_utf8_lossy(&request[..length]);
                assert!(text.starts_with("GET /v1/models "));
                assert!(text
                    .to_ascii_lowercase()
                    .contains("authorization: bearer sk-secret"));
                let body = r#"{"data":[{"id":"example-model"},{"id":"next-model","name":"Next"}]}"#;
                use std::io::Write;
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        (format!("http://{address}/v1"), task)
    }

    #[test]
    fn api_config_is_isolated_and_account_mode_restores_previous_settings() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        let a = storage::create(&root, &mut registry, "A").unwrap();
        let b = storage::create(&root, &mut registry, "B").unwrap();
        let a_dir = storage::profile_dir(&root, &a.id).unwrap();
        let b_dir = storage::profile_dir(&root, &b.id).unwrap();
        let config = config_path(&a_dir);
        fs::write(a_dir.join("codex/auth.json"), "account-login-marker").unwrap();
        let original = fs::read_to_string(&config).unwrap();
        fs::write(
            &config,
            format!("{original}model = \"old-model\"\n[custom]\nflag = true\n"),
        )
        .unwrap();
        let b_original = fs::read(&config_path(&b_dir)).unwrap();
        let (base_url, server) = mock_models(2);
        let mut req = Request {
            id: a.id,
            base_url: format!("{base_url}/"),
            model: "example-model".into(),
            api_key: Some("sk-secret".into()),
            manual_model: false,
        };
        save(&a_dir, &req).unwrap();
        let api = summary(&a_dir).unwrap().unwrap();
        assert!(api.active && api.has_key);
        assert_eq!(api.base_url, base_url);
        assert_eq!(api.models.len(), 2);
        assert!(fs::read_to_string(catalog_path(&a_dir))
            .unwrap()
            .contains("next-model"));
        assert!(!serde_json::to_string(&api).unwrap().contains("sk-secret"));
        assert_eq!(fs::read(&config_path(&b_dir)).unwrap(), b_original);
        req.api_key = None;
        req.model = "next-model".into();
        save(&a_dir, &req).unwrap();
        server.join().unwrap();
        let updated = fs::read_to_string(&config).unwrap();
        assert!(updated.contains("sk-secret"));
        assert!(updated.contains("requires_openai_auth = false"));
        assert!(updated.contains("wire_api = \"responses\""));
        assert!(updated.contains("[custom]"));
        set_compatibility(&a_dir, true).unwrap();
        assert!(summary(&a_dir).unwrap().unwrap().compatibility_enabled);
        set_runtime_bridge_url(&a_dir, 12345).unwrap();
        assert!(fs::read_to_string(&config)
            .unwrap()
            .contains("http://127.0.0.1:12345/v1"));
        assert_eq!(summary(&a_dir).unwrap().unwrap().base_url, base_url);
        set_compatibility(&a_dir, false).unwrap();
        assert!(fs::read_to_string(&config).unwrap().contains(&base_url));
        assert!(!summary(&a_dir).unwrap().unwrap().compatibility_enabled);
        use_account(&a_dir).unwrap();
        let restored = fs::read_to_string(&config).unwrap();
        assert!(!summary(&a_dir).unwrap().unwrap().active);
        assert!(restored.contains("model = \"old-model\""));
        assert!(!restored.contains("model_provider = \"pathmux_managed_api\""));
        assert!(!restored.contains("model_catalog_json ="));
        assert!(restored.contains("[custom]"));
        assert_eq!(
            fs::read_to_string(a_dir.join("codex/auth.json")).unwrap(),
            "account-login-marker"
        );
    }

    #[test]
    fn rejects_insecure_remote_endpoint_without_writing_key() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        let p = storage::create(&root, &mut registry, "A").unwrap();
        let dir = storage::profile_dir(&root, &p.id).unwrap();
        let original = fs::read(config_path(&dir)).unwrap();
        let request = Request {
            id: p.id,
            base_url: "http://remote.example/v1".into(),
            model: "x".into(),
            api_key: Some("secret".into()),
            manual_model: false,
        };
        assert!(save(&dir, &request).is_err());
        assert_eq!(fs::read(config_path(&dir)).unwrap(), original);
    }

    #[test]
    fn manual_model_keeps_gateways_without_models_endpoint_usable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut registry = Registry::default();
        let profile = storage::create(&root, &mut registry, "Manual").unwrap();
        let dir = storage::profile_dir(&root, &profile.id).unwrap();
        save(
            &dir,
            &Request {
                id: profile.id,
                base_url: "http://127.0.0.1:9/v1".into(),
                model: "manual-model".into(),
                api_key: Some("sk-test".into()),
                manual_model: true,
            },
        )
        .unwrap();
        let result = summary(&dir).unwrap().unwrap();
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].id, "manual-model");
    }
}
