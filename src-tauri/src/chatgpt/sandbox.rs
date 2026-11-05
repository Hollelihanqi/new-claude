//! Windows 沙箱账号是全机共享的本地账号，但密码按 CODEX_HOME 各存一份。
//!
//! 上游实现明确拒绝 reparse point：provisioning 用 `OBJ_DONT_REPARSE` 打开
//! `.sandbox-secrets`，写入用临时文件 + rename。因此不能把多份凭证合成一个
//! 物理实体（junction 会让官方 setup 直接失败，硬链接会被 rename 换掉）。
//!
//! 兼容的做法是保留每个 home 的真实目录，并在客户端启动前做一次确定性收敛：
//! 找出唯一仍能登录的那份，原子复制到其余 home；新 home 预置 setup marker，
//! 让它直接沿用现有账号，不再执行 setup 改掉全局密码。
//!
//! macOS 的沙箱走 Seatbelt，不创建本地账号，相关入口为空操作。

use std::fs;
use std::path::{Path, PathBuf};

pub const SECRETS_DIR: &str = ".sandbox-secrets";
pub const SECRETS_FILE: &str = "sandbox_users.json";
pub const SANDBOX_DIR: &str = ".sandbox";
pub const MARKER_FILE: &str = "setup_marker.json";

/// 在实例创建、启动前调用：确保该实例拿到机器上仍然有效的沙箱凭证。
pub fn prepare_profile(profile_dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let root = profile_dir
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| "实例目录层级异常".to_string())?;
        let home = profile_dir.join("codex");
        let mut homes = homes(root)?;
        if !homes.iter().any(|candidate| candidate == &home) {
            homes.push(home);
        }
        reconcile(&homes, imp::valid)?;
        seed_markers(&homes)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = profile_dir;
        Ok(())
    }
}

/// 启动时对全部托管实例和默认 home 做一次收敛。
#[cfg(windows)]
pub fn prepare_machine() -> Result<(), String> {
    let root = super::storage::root()?;
    let homes = homes(&root)?;
    reconcile(&homes, imp::valid)?;
    seed_markers(&homes)
}

#[cfg(windows)]
fn homes(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut homes: Vec<PathBuf> = Vec::new();
    for profile in &super::storage::registry(root)?.profiles {
        homes.push(root.join("profiles").join(&profile.id).join("codex"));
    }
    let default = crate::home().join(".codex");
    if default.exists() && !homes.contains(&default) {
        homes.push(default);
    }
    Ok(homes)
}

/// 找到唯一仍能登录的凭证，原子复制到其余 home。
/// 没有任何可用副本时保持原样，交给客户端重新 setup。
fn reconcile(homes: &[PathBuf], valid: impl Fn(&Path) -> bool) -> Result<(), String> {
    let paths: Vec<PathBuf> = homes
        .iter()
        .map(|home| home.join(SECRETS_DIR).join(SECRETS_FILE))
        .collect();
    // 热路径：所有副本完全一致时不需要解密或登录验证。
    if let Some(first) = paths.first().and_then(|path| fs::read(path).ok()) {
        if paths
            .iter()
            .all(|path| fs::read(path).ok().as_deref() == Some(first.as_slice()))
        {
            return Ok(());
        }
    }

    let mut source = None;
    for path in &paths {
        if valid(path) {
            source = Some(path.clone());
            break;
        }
    }
    let Some(source) = source else {
        #[cfg(windows)]
        crate::sync::log_line("沙箱凭证：没有可登录的副本，等待客户端重新 setup");
        return Ok(());
    };
    let bytes = fs::read(&source).map_err(|e| format!("无法读取沙箱凭证：{e}"))?;
    let mut repaired = 0usize;
    for home in homes {
        let dir = home.join(SECRETS_DIR);
        let target = dir.join(SECRETS_FILE);
        if target == source {
            continue;
        }
        if fs::read(&target).ok().as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建沙箱凭证目录：{e}"))?;
        // 客户端也用临时文件 + rename 写入，保持一致避免读到半截内容。
        crate::sync::write_bytes_atomic(&target, &bytes)
            .map_err(|e| format!("无法同步沙箱凭证：{e}"))?;
        repaired += 1;
    }
    if repaired > 0 {
        #[cfg(windows)]
        crate::sync::log_line(&format!(
            "沙箱凭证已收敛：使用 {}，修复 {repaired} 个实例",
            source.display()
        ));
    }
    Ok(())
}

/// 新 home 继承机器现有的 marker，客户端就不会重新 setup 改密。
/// 纯文件逻辑，保持双平台可编译，单元测试在两个平台都能覆盖。
#[cfg_attr(not(windows), allow(dead_code))]
fn seed_markers(homes: &[PathBuf]) -> Result<(), String> {
    let template = homes
        .iter()
        .map(|home| home.join(SANDBOX_DIR).join(MARKER_FILE))
        .find_map(|path| {
            fs::read_to_string(path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        });
    let Some(template) = template else {
        return Ok(());
    };
    for home in homes {
        let marker = home.join(SANDBOX_DIR).join(MARKER_FILE);
        if marker.exists() {
            continue;
        }
        if !home.join(SECRETS_DIR).join(SECRETS_FILE).exists() {
            continue;
        }
        let value = serde_json::json!({
            "version": template.get("version").cloned().unwrap_or_else(|| serde_json::json!(5)),
            "offline_username": template
                .get("offline_username")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("CodexSandboxOffline")),
            "online_username": template
                .get("online_username")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("CodexSandboxOnline")),
            "created_at": template
                .get("created_at")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("")),
            "proxy_ports": [],
            "allow_local_binding": false,
            "read_roots": [],
            "write_roots": [],
        });
        if let Some(parent) = marker.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("无法创建沙箱状态目录：{e}"))?;
        }
        // 必须不带 BOM：带 BOM 的 marker 会被客户端判为不可解析。
        fs::write(
            &marker,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("无法写入沙箱状态：{e}"))?;
    }
    Ok(())
}

#[cfg(windows)]
mod imp {
    use base64::Engine;
    use serde_json::Value;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    use windows_sys::Win32::Security::{
        LogonUserW, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT,
    };

    /// 校验一份凭证确实能登录机器上的两个沙箱账号。
    pub fn valid(path: &Path) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return false;
        };
        ["offline", "online"].iter().all(|key| {
            let Some(entry) = value.get(*key) else {
                return false;
            };
            let (Some(user), Some(password)) = (
                entry.get("username").and_then(Value::as_str),
                entry
                    .get("password")
                    .and_then(Value::as_str)
                    .and_then(unprotect),
            ) else {
                return false;
            };
            logon(user, &password)
        })
    }

    fn unprotect(encoded: &str) -> Option<String> {
        let data = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData as _) };
        String::from_utf8(bytes).ok()
    }

    fn logon(user: &str, password: &str) -> bool {
        let user: Vec<u16> = user.encode_utf16().chain(std::iter::once(0)).collect();
        let domain: Vec<u16> = ".".encode_utf16().chain(std::iter::once(0)).collect();
        let password: Vec<u16> = password.encode_utf16().chain(std::iter::once(0)).collect();
        let mut token: HANDLE = std::ptr::null_mut();
        let ok = unsafe {
            LogonUserW(
                user.as_ptr(),
                domain.as_ptr(),
                password.as_ptr(),
                LOGON32_LOGON_NETWORK,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        };
        if ok == 0 {
            return false;
        }
        unsafe { CloseHandle(token) };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(root: &Path, name: &str) -> PathBuf {
        let home = root.join(name);
        fs::create_dir_all(home.join(SECRETS_DIR)).unwrap();
        home
    }

    #[test]
    fn copies_the_only_valid_credential_to_every_home() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let a = home(root, "a");
        let b = home(root, "b");
        let c = home(root, "c");
        fs::write(a.join(SECRETS_DIR).join(SECRETS_FILE), b"good").unwrap();
        fs::write(b.join(SECRETS_DIR).join(SECRETS_FILE), b"stale").unwrap();
        let is_valid = |path: &Path| fs::read(path).is_ok_and(|bytes| bytes == b"good");
        reconcile(&[a.clone(), b.clone(), c.clone()], is_valid).unwrap();
        for home in [&a, &b, &c] {
            assert_eq!(
                fs::read(home.join(SECRETS_DIR).join(SECRETS_FILE)).unwrap(),
                b"good"
            );
        }
    }

    #[test]
    fn leaves_homes_untouched_when_no_credential_is_valid() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let a = home(root, "a");
        fs::write(a.join(SECRETS_DIR).join(SECRETS_FILE), b"stale").unwrap();
        reconcile(&[a.clone()], |_| false).unwrap();
        assert_eq!(
            fs::read(a.join(SECRETS_DIR).join(SECRETS_FILE)).unwrap(),
            b"stale"
        );
    }

    #[test]
    fn seeds_a_marker_without_bom_and_without_inherited_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let a = home(root, "a");
        let b = home(root, "b");
        fs::create_dir_all(a.join(SANDBOX_DIR)).unwrap();
        fs::write(
            a.join(SANDBOX_DIR).join(MARKER_FILE),
            br#"{"version":5,"offline_username":"CodexSandboxOffline","online_username":"CodexSandboxOnline","created_at":"2026-11-04T02:44:57.6+00:00","proxy_ports":[],"allow_local_binding":false,"read_roots":["C:/secret"],"write_roots":["C:/write"]}"#,
        )
        .unwrap();
        fs::write(b.join(SECRETS_DIR).join(SECRETS_FILE), b"secret").unwrap();
        seed_markers(&[a.clone(), b.clone()]).unwrap();
        let bytes = fs::read(b.join(SANDBOX_DIR).join(MARKER_FILE)).unwrap();
        assert_ne!(bytes.first(), Some(&0xEF));
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"read_roots\": []"));
        assert!(!text.contains("C:/secret"));
    }

    #[cfg(not(windows))]
    #[test]
    fn prepare_is_a_noop_outside_windows() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("root/profiles/id");
        fs::create_dir_all(profile.join("codex")).unwrap();
        prepare_profile(&profile).unwrap();
        assert!(!profile.join("codex").join(SECRETS_DIR).exists());
    }
}
