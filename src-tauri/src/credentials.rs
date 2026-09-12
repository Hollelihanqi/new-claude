// 保持旧 PowerShell DPAPI 十六进制格式与钥匙串 service/account，升级无需重新输入 Key。
#[cfg(target_os = "macos")]
fn account() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".into())
}

#[cfg(target_os = "macos")]
pub fn store(name: &str, token: &str) -> Result<Option<String>, String> {
    security_framework::passwords::set_generic_password(
        &format!("{}:{name}", crate::KEYCHAIN_PREFIX),
        &account(),
        token.as_bytes(),
    )
    .map_err(|e| format!("钥匙串保存失败：{e}"))?;
    Ok(None)
}

#[cfg(target_os = "macos")]
pub fn read(name: &str, _encrypted: Option<&str>) -> Result<String, String> {
    let bytes = security_framework::passwords::get_generic_password(
        &format!("{}:{name}", crate::KEYCHAIN_PREFIX),
        &account(),
    )
    .map_err(|e| format!("钥匙串读取失败：{e}"))?;
    String::from_utf8(bytes).map_err(|_| "Key 编码无效".into())
}

#[cfg(target_os = "macos")]
pub fn clear(name: &str) -> Result<(), String> {
    security_framework::passwords::delete_generic_password(
        &format!("{}:{name}", crate::KEYCHAIN_PREFIX),
        &account(),
    )
    .map_err(|e| format!("钥匙串清除失败：{e}"))
}

#[cfg(windows)]
fn dpapi(input: &mut [u8], protect: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len().try_into().map_err(|_| "Key 长度超出限制")?,
        pbData: input.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // 输入在调用期间有效；输出由 DPAPI 分配，复制后清零并用 LocalFree 释放。
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &source,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &source,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(format!(
            "DPAPI 操作失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    if output.cbData == 0 {
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        return Ok(Vec::new());
    }
    let bytes = unsafe {
        let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let bytes = buffer.to_vec();
        for byte in buffer {
            std::ptr::write_volatile(byte, 0);
        }
        LocalFree(output.pbData.cast());
        bytes
    };
    Ok(bytes)
}

#[cfg(windows)]
pub fn store(_name: &str, token: &str) -> Result<Option<String>, String> {
    let mut bytes: Vec<u8> = token.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let result = dpapi(&mut bytes, true).map(|encrypted| Some(hex::encode(encrypted)));
    for byte in &mut bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    result
}

#[cfg(windows)]
pub fn read(_name: &str, encrypted: Option<&str>) -> Result<String, String> {
    let mut encrypted = hex::decode(
        encrypted
            .filter(|s| !s.is_empty())
            .ok_or("该环境没有保存 Key")?,
    )
    .map_err(|_| "已保存 Key 的格式无效")?;
    let mut bytes = dpapi(&mut encrypted, false)?;
    let result = if bytes.len() % 2 != 0 {
        Err("Key 编码无效".into())
    } else {
        let mut units = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        let result = String::from_utf16(&units).map_err(|_| "Key 编码无效".into());
        for unit in &mut units {
            unsafe {
                std::ptr::write_volatile(unit, 0);
            }
        }
        result
    };
    for byte in &mut bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    result
}

#[cfg(not(target_os = "macos"))]
pub fn clear(_name: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn store(_name: &str, _token: &str) -> Result<Option<String>, String> {
    Err("当前平台不支持安全存储 token".into())
}
#[cfg(not(any(windows, target_os = "macos")))]
pub fn read(_name: &str, _encrypted: Option<&str>) -> Result<String, String> {
    Err("当前平台不支持安全存储 token".into())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn windows_dpapi_roundtrip() {
        let token = "test-only-中文-key-'\"-🙂";
        let encrypted = super::store("test", token).unwrap().unwrap();
        assert_eq!(super::read("test", Some(&encrypted)).unwrap(), token);
        assert!(super::read("test", Some("invalid")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_credentials_remain_compatible_with_powershell() {
        use std::io::Write;
        use std::process::Stdio;
        // 只通过 stdin 传测试数据，测试也不把凭证拼进进程参数。
        fn powershell(script: &str, input: &str) -> String {
            let mut child = crate::ps_command()
                // 测试可能从 PowerShell 7 启动，不能把它的模块目录传给 Windows PowerShell 5。
                .env_remove("PSModulePath")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "测试凭证转换失败：{}",
                String::from_utf8_lossy(&result.stderr)
            );
            String::from_utf8(result.stdout).unwrap().trim().to_owned()
        }
        let token = "test-only-compat-key";
        let legacy = powershell("$s=[Console]::In.ReadToEnd(); ConvertTo-SecureString -String $s -AsPlainText -Force | ConvertFrom-SecureString", token);
        assert_eq!(super::read("test", Some(&legacy)).unwrap(), token);
        let encrypted = super::store("test", token).unwrap().unwrap();
        let decoded = powershell("$s=ConvertTo-SecureString ([Console]::In.ReadToEnd()); $p=[Runtime.InteropServices.Marshal]::SecureStringToBSTR($s); try { [Runtime.InteropServices.Marshal]::PtrToStringBSTR($p) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($p) }", &encrypted);
        assert_eq!(decoded, token);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "需要可用且已解锁的测试钥匙串"]
    fn macos_keychain_roundtrip() {
        let name = format!("credential-test-{}", std::process::id());
        super::store(&name, "test-only-key").unwrap();
        let result = super::read(&name, None);
        super::clear(&name).unwrap();
        assert_eq!(result.unwrap(), "test-only-key");
    }
}
