//! Display-only identity from this profile's file credential store. Never use
//! unverified token claims to authorize requests or expose tokens to the UI.
use super::*;
use base64::{engine::general_purpose, Engine};
use serde_json::Value;

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Account {
    Chatgpt { email: String },
    ApiKey,
    SignedOut,
    Unavailable,
}

pub fn read(dir: &Path) -> Account {
    let path = dir.join("codex/auth.json");
    if storage::plain(&path).is_err() {
        return Account::Unavailable;
    }
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Account::SignedOut,
        Ok(meta) if meta.is_file() && meta.len() <= 64 * 1024 => {
            match storage::read_json::<Value>(&path) {
                Ok(auth) => identity(&auth),
                Err(_) => Account::Unavailable,
            }
        }
        _ => Account::Unavailable,
    }
}

fn identity(auth: &Value) -> Account {
    // Explicit API mode wins over leftover ChatGPT tokens.
    if auth["auth_mode"] == "apikey"
        || (auth["auth_mode"].is_null()
            && auth["OPENAI_API_KEY"]
                .as_str()
                .is_some_and(|s| !s.is_empty()))
    {
        return Account::ApiKey;
    }
    if !auth["auth_mode"].is_null() && auth["auth_mode"] != "chatgpt" {
        return Account::Unavailable;
    }
    let Some(token) = auth.pointer("/tokens/id_token").and_then(Value::as_str) else {
        return Account::Unavailable;
    };
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Account::Unavailable;
    }
    let claims = general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| general_purpose::URL_SAFE.decode(parts[1]))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let email = claims
        .as_ref()
        .and_then(|claims| claims["email"].as_str())
        .map(str::trim)
        .filter(|email| {
            email.len() <= 254
                && email.contains('@')
                && !email.chars().any(|c| c.is_control() || c.is_whitespace())
        });
    match email {
        Some(email) => Account::Chatgpt {
            email: email.into(),
        },
        None => Account::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn auth(email: &str) -> Value {
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(json!({"email": email}).to_string());
        json!({"auth_mode":"chatgpt", "tokens":{
            "id_token":format!("header.{payload}.signature"),
            "access_token":"private-access-token", "refresh_token":"private-refresh-token"
        }})
    }

    #[test]
    fn reads_each_profiles_current_identity_on_both_platform_layouts() {
        let temp = tempfile::tempdir().unwrap();
        for layout in ["windows/PathMux/chatgpt", "macos/PathMux/chatgpt"] {
            let root = temp.path().join(layout);
            let a = root.join("profiles/a");
            let b = root.join("profiles/b");
            fs::create_dir_all(a.join("codex")).unwrap();
            fs::create_dir_all(b.join("codex")).unwrap();
            assert_eq!(read(&a), Account::SignedOut);
            fs::write(a.join("codex/auth.json"), auth("a@example.com").to_string()).unwrap();
            fs::write(b.join("codex/auth.json"), auth("b@example.com").to_string()).unwrap();
            assert_eq!(
                read(&a),
                Account::Chatgpt {
                    email: "a@example.com".into()
                }
            );
            assert_eq!(
                read(&b),
                Account::Chatgpt {
                    email: "b@example.com".into()
                }
            );
            let serialized = serde_json::to_string(&read(&a)).unwrap();
            assert_eq!(
                serialized,
                r#"{"status":"chatgpt","email":"a@example.com"}"#
            );
            fs::write(
                a.join("codex/auth.json"),
                auth("new@example.com").to_string(),
            )
            .unwrap();
            assert_eq!(
                read(&a),
                Account::Chatgpt {
                    email: "new@example.com".into()
                }
            );
            fs::remove_file(a.join("codex/auth.json")).unwrap();
            assert_eq!(read(&a), Account::SignedOut);
            assert_eq!(
                read(&b),
                Account::Chatgpt {
                    email: "b@example.com".into()
                }
            );
        }
    }

    #[test]
    fn api_and_unknown_modes_never_report_leftover_account_emails() {
        let mut value = auth("old@example.com");
        value["auth_mode"] = json!("apikey");
        assert_eq!(identity(&value), Account::ApiKey);
        value["auth_mode"] = json!("future-mode");
        assert_eq!(identity(&value), Account::Unavailable);
        assert_eq!(
            identity(&json!({"OPENAI_API_KEY":"sk-secret"})),
            Account::ApiKey
        );
    }

    #[test]
    fn unreadable_identity_does_not_claim_signed_out_or_leak_credentials() {
        for value in [
            json!({}),
            json!({"tokens":{"id_token":"not-a-jwt"}}),
            json!({"tokens":{"id_token":"a.!!!.c"}}),
            auth(""),
            auth("bad\n@example.com"),
        ] {
            assert_eq!(identity(&value), Account::Unavailable);
        }
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("codex")).unwrap();
        let path = temp.path().join("codex/auth.json");
        fs::write(&path, "broken-json-private-token").unwrap();
        assert_eq!(read(temp.path()), Account::Unavailable);
        fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
        assert_eq!(read(temp.path()), Account::Unavailable);
    }
}
