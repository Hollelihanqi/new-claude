use super::*;
use std::io;

pub fn root() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir().ok_or("无法定位本地应用数据目录")?;
    Ok(base.join("PathMux").join("chatgpt"))
}

pub fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

pub fn plain(path: &Path) -> Result<(), String> {
    let mut cursor = Some(path);
    while let Some(p) = cursor {
        match fs::symlink_metadata(p) {
            Ok(m) if linked(&m) => {
                return Err(format!("数据路径包含链接，请使用独立目录：{}", p.display()))
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        cursor = p.parent();
    }
    Ok(())
}

pub fn private_dir(path: &Path) -> Result<(), String> {
    plain(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|e| e.to_string())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
        // Apply an explicit DACL to the managed root; children inherit it.
        let who = process::output(Command::new("whoami.exe").args(["/user", "/fo", "csv", "/nh"]))?;
        let sid = who
            .trim()
            .split(',')
            .next_back()
            .unwrap_or("")
            .trim_matches('"');
        if !sid.starts_with("S-1-")
            || !sid
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
        {
            return Err("无法确认当前 Windows 用户权限".into());
        }
        process::output(Command::new("icacls.exe").arg(path).args([
            "/inheritance:r",
            "/grant:r",
            &format!("*{sid}:(OI)(CI)F"),
            "*S-1-5-18:(OI)(CI)F",
        ]))?;
    }
    Ok(())
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    plain(path)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    crate::sync::write_bytes_atomic(path, &bytes).map_err(|e| e.to_string())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    plain(path)?;
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > 8 * 1024 * 1024 {
        return Err("实例登记文件过大，拒绝读取".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("实例登记文件无效：{e}"))
}

pub fn registry(root: &Path) -> Result<Registry, String> {
    plain(root)?;
    let p = root.join("registry.json");
    if !p.exists() {
        return Ok(Registry::default());
    }
    let r: Registry = read_json(&p)?;
    if r.version != 1 {
        return Err("实例登记格式不受支持，请更新 PathMux".into());
    }
    let mut ids = std::collections::HashSet::new();
    for profile in &r.profiles {
        validate_id(&profile.id)?;
        if !ids.insert(&profile.id) {
            return Err("实例 ID 重复，已停止操作".into());
        }
    }
    Ok(r)
}

pub fn save(root: &Path, r: &Registry) -> Result<(), String> {
    private_dir(root)?;
    write_json(&root.join("registry.json"), r)
}

pub fn validate_id(id: &str) -> Result<(), String> {
    match uuid::Uuid::parse_str(id) {
        Ok(v) if v.to_string() == id => Ok(()),
        _ => Err("无效的实例 ID".into()),
    }
}

pub fn name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 40 || value.chars().any(char::is_control) {
        return Err("名称应为 1–40 个字符，不能包含控制字符".into());
    }
    Ok(value.into())
}

pub fn profile_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    let p = root.join("profiles").join(id);
    plain(&p)?;
    let marker: serde_json::Value = read_json(&p.join("profile.json"))?;
    if marker["owner"] != "pathmux-chatgpt" || marker["id"] != id {
        return Err("实例目录归属不符，已停止操作".into());
    }
    for child in ["desktop", "codex", "codex/db", "codex/config.toml", "logs"] {
        plain(&p.join(child))?;
    }
    Ok(p)
}

pub fn config_text(dir: &Path) -> String {
    let db = dir.join("codex/db");
    format!("# Managed by PathMux: credentials and state stay in this profile.\nforced_login_method = \"chatgpt\"\ncli_auth_credentials_store = \"file\"\nmcp_oauth_credentials_store = \"file\"\nsqlite_home = {}\n", serde_json::to_string(&db.to_string_lossy()).unwrap())
}

pub fn verify_config(dir: &Path) -> Result<(), String> {
    let path = dir.join("codex/config.toml");
    plain(&path)?;
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "实例配置无法解析")?;
    let expected_db = dir.join("codex/db");
    if doc.get("forced_login_method").and_then(|x| x.as_str()) != Some("chatgpt")
        || doc.get("sqlite_home").and_then(|x| x.as_str()) != expected_db.to_str()
        || doc
            .get("cli_auth_credentials_store")
            .and_then(|x| x.as_str())
            != Some("file")
        || doc
            .get("mcp_oauth_credentials_store")
            .and_then(|x| x.as_str())
            != Some("file")
    {
        return Err(
            "实例的数据或凭证目录配置已改变。请恢复实例内的 sqlite_home 和 file 凭证存储后重试。"
                .into(),
        );
    }
    Ok(())
}

pub fn create(root: &Path, r: &mut Registry, display_name: &str) -> Result<Profile, String> {
    let display_name = name(display_name)?;
    if r.profiles
        .iter()
        .any(|p| p.name.to_lowercase() == display_name.to_lowercase())
    {
        return Err("该实例名称已存在".into());
    }
    private_dir(root)?;
    private_dir(&root.join("profiles"))?;
    let profile = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        name: display_name,
        created_at: now(),
        last_executable: None,
    };
    let dir = root.join("profiles").join(&profile.id);
    let result = (|| {
        private_dir(&dir)?;
        for child in ["desktop", "codex", "codex/db", "logs"] {
            private_dir(&dir.join(child))?;
        }
        write_json(
            &dir.join("profile.json"),
            &serde_json::json!({"owner":"pathmux-chatgpt", "id":profile.id}),
        )?;
        crate::sync::write_bytes_atomic(
            &dir.join("codex/config.toml"),
            config_text(&dir).as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        r.profiles.push(profile.clone());
        if let Err(e) = save(root, r) {
            r.profiles.pop();
            return Err(e);
        }
        Ok(profile)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_are_independent_and_names_never_become_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut r = Registry::default();
        let a = create(&root, &mut r, "工作 / 个人").unwrap();
        let b = create(&root, &mut r, "B").unwrap();
        let pa = profile_dir(&root, &a.id).unwrap();
        let pb = profile_dir(&root, &b.id).unwrap();
        assert_ne!(pa, pb);
        verify_config(&pa).unwrap();
        assert!(!pa.join("codex/auth.json").exists());
        assert!(create(&root, &mut r, "b").is_err());
        assert!(profile_dir(&root, "../default").is_err());
        fs::write(pa.join("codex/config.toml"), "sqlite_home = '/other'\n").unwrap();
        assert!(verify_config(&pa).is_err());
        verify_config(&pb).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn linked_profile_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let mut r = Registry::default();
        let p = create(&root, &mut r, "test").unwrap();
        let dir = profile_dir(&root, &p.id).unwrap();
        fs::remove_dir(dir.join("desktop")).unwrap();
        std::os::unix::fs::symlink(&root, dir.join("desktop")).unwrap();
        assert!(profile_dir(&root, &p.id).is_err());
    }
}
