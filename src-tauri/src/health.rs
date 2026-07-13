// 健康检查与诊断:
// 1) model_pin_warnings / fix_model_pin —— 检测实例(和主账户)的 settings.json
//    里 /model 钉死了具体型号(绕过 ANTHROPIC_DEFAULT_*_MODEL 档位映射),支持一键还原。
// 2) health_check —— 按需跑一组自检(claude CLI、终端集成、共享链接、证书、网关连通),
//    网关探测并行执行,只在用户点开健康面板时才发起。
// 3) export_diagnostics —— 把自检结果 + 脱敏配置 + 同步日志尾部汇总成一个文本报告,
//    落到桌面,同事出问题时直接把文件发给管理员。

use crate::{load, profile_names, Profile, MAIN_PROFILE_KEY, MARK};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------- 模型档位钉死检测 ----------------

// /model 只有选这些档位别名时,模型映射(ANTHROPIC_DEFAULT_*_MODEL)才拦得住;
// 其余值都是具体型号 ID,会原样发给网关、绕过映射。
const MODEL_ALIASES: [&str; 5] = ["default", "opus", "sonnet", "haiku", "opusplan"];

fn is_model_alias(m: &str) -> bool {
    let m = m.trim().to_ascii_lowercase();
    // sonnet[1m] 这类长上下文档位也算别名档位
    let base = m.strip_suffix("[1m]").unwrap_or(&m);
    MODEL_ALIASES.contains(&base)
}

// profile 键 → 该副本 settings.json 的路径(__main__ = 主账户)
fn settings_path_for(profile: &str) -> PathBuf {
    if profile == MAIN_PROFILE_KEY {
        crate::home().join(".claude").join("settings.json")
    } else {
        crate::home()
            .join(".claude-split")
            .join(profile)
            .join(".claude")
            .join("settings.json")
    }
}

// settings.json 里钉死的具体型号;别名档位/无 model 字段/文件缺失均返回 None
fn pinned_model_in(path: &Path) -> Option<String> {
    let doc: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let m = doc.get("model")?.as_str()?.trim().to_string();
    if m.is_empty() || is_model_alias(&m) {
        None
    } else {
        Some(m)
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelPinWarning {
    pub profile: String, // 实例名;主账户为 __main__
    pub model: String,   // 被钉死的具体型号
    pub settings_path: String,
}

fn collect_model_pin_warnings(list: &[Profile]) -> Vec<ModelPinWarning> {
    let mut out = vec![];
    let has_router = list.iter().any(|p| p.type_ == "router");
    // 主账户钉死具体型号本身合法,但在 home 目录启动实例时会以"项目级配置"
    // 身份覆盖实例档位,所以只在存在路由实例时才提醒。
    if has_router {
        let p = settings_path_for(MAIN_PROFILE_KEY);
        if let Some(m) = pinned_model_in(&p) {
            out.push(ModelPinWarning {
                profile: MAIN_PROFILE_KEY.into(),
                model: m,
                settings_path: p.display().to_string(),
            });
        }
    }
    // account 类型实例直连官方,钉具体型号是合法用法,只查 router
    for prof in list.iter().filter(|p| p.type_ == "router") {
        let p = settings_path_for(&prof.name);
        if let Some(m) = pinned_model_in(&p) {
            out.push(ModelPinWarning {
                profile: prof.name.clone(),
                model: m,
                settings_path: p.display().to_string(),
            });
        }
    }
    out
}

#[tauri::command]
pub fn model_pin_warnings() -> Vec<ModelPinWarning> {
    collect_model_pin_warnings(&load())
}

// 一键还原:删掉 settings.json 的 model 字段(回到默认档位),其余内容原样保留
#[tauri::command]
pub fn fix_model_pin(profile: String) -> Result<String, String> {
    // 只允许操作已知副本,防止前端传来任意路径片段
    let known = profile == MAIN_PROFILE_KEY
        || load().iter().any(|p| p.name == profile);
    if !known {
        return Err("未找到该实例".into());
    }
    let path = settings_path_for(&profile);
    let text = fs::read_to_string(&path).map_err(|e| format!("读取 settings.json 失败：{e}"))?;
    let mut doc: Value =
        serde_json::from_str(&text).map_err(|e| format!("settings.json 不是有效 JSON：{e}"))?;
    let obj = doc
        .as_object_mut()
        .ok_or("settings.json 顶层不是对象")?;
    if obj.remove("model").is_none() {
        return Ok("该副本没有钉死模型，无需还原。".into());
    }
    crate::sync::write_json_atomic(&path, &doc).map_err(|e| format!("写回失败：{e}"))?;
    Ok("已还原为档位别名（默认档）。正在运行的 claude 会话需重启后生效。".into())
}

// ---------------- 健康检查 ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthItem {
    pub id: String,
    pub label: String,
    pub status: String, // "ok" | "warn" | "fail"
    pub detail: String,
}

fn item(id: &str, label: &str, status: &str, detail: String) -> HealthItem {
    HealthItem {
        id: id.into(),
        label: label.into(),
        status: status.into(),
        detail,
    }
}

// shell 配置文件里是否还留着集成行(同事重装终端配置后最容易丢的就是这行)
fn integration_line_present() -> (bool, String) {
    if cfg!(target_os = "windows") {
        let out = crate::ps_command()
            .args(["-NoProfile", "-Command", "$PROFILE"])
            .output();
        let profile_path = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => return (false, format!("无法定位 PowerShell $PROFILE：{e}")),
        };
        if profile_path.is_empty() {
            return (false, "无法定位 PowerShell $PROFILE".into());
        }
        let content = fs::read_to_string(&profile_path).unwrap_or_default();
        if content.contains(MARK) {
            (true, format!("$PROFILE 已接入（{profile_path}）"))
        } else {
            (false, format!("$PROFILE 里没有集成行（{profile_path}），请在实例配置里点一次「保存并接入终端」"))
        }
    } else {
        for rc in [".zshrc", ".bashrc"] {
            let p = crate::home().join(rc);
            if fs::read_to_string(&p).unwrap_or_default().contains(MARK) {
                return (true, format!("~/{rc} 已接入"));
            }
        }
        (
            false,
            "~/.zshrc 和 ~/.bashrc 都没有集成行，请在实例配置里点一次「保存并接入终端」".into(),
        )
    }
}

fn run_health_checks() -> Vec<HealthItem> {
    let list = load();
    let names = profile_names(&list);
    let mut items = vec![];

    // 1. claude CLI
    match crate::claude_cli::resolve_claude_exe() {
        Some(p) => items.push(item("claude", "Claude Code CLI", "ok", format!("已找到：{}", p.display()))),
        None => items.push(item(
            "claude",
            "Claude Code CLI",
            "fail",
            "未找到 claude 可执行文件，请先安装 Claude Code。".into(),
        )),
    }

    // 2. 终端集成
    if names.is_empty() {
        items.push(item("integration", "终端集成", "warn", "还没有创建任何实例。".into()));
    } else {
        let script = if cfg!(target_os = "windows") {
            crate::ps_path()
        } else {
            crate::sh_path()
        };
        if !script.is_file() {
            items.push(item(
                "integration",
                "终端集成",
                "fail",
                format!("集成脚本缺失（{}），请点一次「保存并接入终端」重建。", script.display()),
            ));
        } else {
            let (ok, detail) = integration_line_present();
            items.push(item("integration", "终端集成", if ok { "ok" } else { "fail" }, detail));
        }
    }

    // 3. 共享目录链接
    if !names.is_empty() {
        let probs = crate::sync::broken_links(&names);
        if probs.is_empty() {
            items.push(item(
                "links",
                "共享目录链接",
                "ok",
                format!("{} 个实例的 skills/plugins/agents/commands 链接完好。", names.len()),
            ));
        } else {
            items.push(item(
                "links",
                "共享目录链接",
                "warn",
                format!("发现异常：{}。下次启动 claude 或本程序时会自动修复。", probs.join("、")),
            ));
        }
    }

    // 4. CA 证书
    let cert_count = crate::count_certs();
    let has_router = list.iter().any(|p| p.type_ == "router");
    if cert_count > 0 {
        items.push(item("cert", "CA 证书", "ok", format!("信任库中共 {cert_count} 张证书。")));
    } else if has_router {
        items.push(item(
            "cert",
            "CA 证书",
            "warn",
            "未导入任何证书。若公司网关用自签名证书，需在右上角「CA 证书」导入。".into(),
        ));
    } else {
        items.push(item("cert", "CA 证书", "ok", "未导入（没有路由实例，无需证书）。".into()));
    }

    // 5. 各路由实例的网关连通 + Key 有效性（并行探测）
    let routers: Vec<Profile> = list.iter().filter(|p| p.type_ == "router").cloned().collect();
    let handles: Vec<_> = routers
        .into_iter()
        .map(|p| {
            std::thread::spawn(move || {
                let res = if p.base_url.trim().is_empty() {
                    Err("未配置网关地址".to_string())
                } else {
                    crate::decrypt_token(&p)
                        .and_then(|t| crate::detect_models(p.base_url.clone(), t))
                };
                (p.name, res)
            })
        })
        .collect();
    for h in handles {
        if let Ok((name, res)) = h.join() {
            match res {
                Ok(models) => items.push(item(
                    &format!("gateway:{name}"),
                    &format!("网关连通（{name}）"),
                    "ok",
                    format!("可达，Key 有效，{} 个可用模型。", models.len()),
                )),
                Err(e) => items.push(item(
                    &format!("gateway:{name}"),
                    &format!("网关连通（{name}）"),
                    "fail",
                    e,
                )),
            }
        }
    }

    // 6. 模型档位钉死告警
    for w in collect_model_pin_warnings(&load()) {
        let who = if w.profile == MAIN_PROFILE_KEY {
            "主账户".to_string()
        } else {
            format!("实例 {}", w.profile)
        };
        items.push(item(
            &format!("modelpin:{}", w.profile),
            &format!("模型映射（{who}）"),
            "warn",
            format!(
                "{who} 的 /model 钉死了具体型号「{}」，会绕过模型映射。可在实例配置页一键还原。",
                w.model
            ),
        ));
    }

    items
}

#[tauri::command]
pub fn health_check() -> Vec<HealthItem> {
    run_health_checks()
}

// ---------------- 诊断报告导出 ----------------

// 不引入 chrono:用 Howard Hinnant 的 civil_from_days 算法把 epoch 秒转成 UTC 日期
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn fmt_utc(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let rem = secs % 86400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn status_tag(s: &str) -> &'static str {
    match s {
        "ok" => "[正常]",
        "warn" => "[警告]",
        _ => "[异常]",
    }
}

// 实例配置的脱敏摘要:绝不输出 token_enc / 任何密钥内容
fn profile_summary(p: &Profile) -> String {
    let mut s = format!("- {}（{}）", p.name, if p.type_ == "router" { "自定义路由" } else { "另一个账户" });
    if p.type_ == "router" {
        fn or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
            if s.is_empty() { fallback } else { s }
        }
        s += &format!(
            "\n    网关: {}\n    Key: {}\n    映射: opus={} sonnet={} haiku={}",
            or(&p.base_url, "<未配置>"),
            if p.has_token { "已保存（内容不导出）" } else { "未保存" },
            or(&p.opus_model, "<未设>"),
            or(&p.sonnet_model, "<未设>"),
            or(&p.haiku_model, "<未设>"),
        );
    }
    s
}

fn build_report(app_version: &str, items: &[HealthItem]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "其他"
    };
    let mut r = String::new();
    r += "=== cc-manager 诊断报告 ===\n";
    r += &format!("生成时间: {}\nApp 版本: v{app_version}\n平台: {platform}\n", fmt_utc(now));

    r += "\n--- 健康检查 ---\n";
    for it in items {
        r += &format!("{} {} — {}\n", status_tag(&it.status), it.label, it.detail);
    }

    r += "\n--- 实例配置（已脱敏） ---\n";
    let list = load();
    if list.is_empty() {
        r += "（还没有实例）\n";
    }
    for p in &list {
        r += &profile_summary(p);
        r.push('\n');
    }

    r += "\n--- 各副本 settings.json 的 model 字段 ---\n";
    let mut keys = vec![MAIN_PROFILE_KEY.to_string()];
    keys.extend(profile_names(&list));
    for k in keys {
        let path = settings_path_for(&k);
        let who = if k == MAIN_PROFILE_KEY { "主账户" } else { k.as_str() };
        let val = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|d| d.get("model").and_then(|m| m.as_str()).map(String::from));
        r += &format!("- {who}: {}\n", val.unwrap_or_else(|| "<未设置>".into()));
    }

    r += "\n--- 同步快照概要 ---\n";
    match fs::read_to_string(crate::cfg_dir().join("sync-snapshot.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        Some(v) => {
            if let Some(domains) = v.get("domains").and_then(|d| d.as_object()) {
                for (field, snap) in domains {
                    let n = snap
                        .get("state")
                        .and_then(|s| s.as_object())
                        .map(|o| o.len())
                        .unwrap_or(0);
                    let reps = snap
                        .get("replicas")
                        .and_then(|s| s.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    r += &format!("- {field}: {n} 项，上轮覆盖 {reps} 个副本\n");
                }
            }
        }
        None => r += "（无快照，可能还没跑过同步）\n",
    }

    r += "\n--- 同步日志（最近 200 行） ---\n";
    let log = fs::read_to_string(crate::cfg_dir().join("sync.log")).unwrap_or_default();
    if log.is_empty() {
        r += "（日志为空）\n";
    } else {
        let lines: Vec<&str> = log.lines().collect();
        let start = lines.len().saturating_sub(200);
        for l in &lines[start..] {
            r += l;
            r.push('\n');
        }
    }
    r
}

// 在资源管理器 / Finder 里定位到导出的文件(尽力而为,失败不影响导出结果)
fn reveal_file(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
}

#[tauri::command]
pub fn export_diagnostics(app: tauri::AppHandle) -> Result<String, String> {
    let version = app.package_info().version.to_string();
    let items = run_health_checks();
    let report = build_report(&version, &items);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((now / 86400) as i64);
    let rem = now % 86400;
    let name = format!(
        "cc-manager-诊断-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.txt",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    );
    let dir = dirs::desktop_dir().unwrap_or_else(crate::home);
    let path = dir.join(name);
    fs::write(&path, report).map_err(|e| format!("写入诊断文件失败：{e}"))?;
    reveal_file(&path);
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_alias_recognized_case_insensitive_with_1m_suffix() {
        assert!(is_model_alias("sonnet"));
        assert!(is_model_alias("Opus"));
        assert!(is_model_alias("DEFAULT"));
        assert!(is_model_alias("opusplan"));
        assert!(is_model_alias("sonnet[1m]"));
        assert!(is_model_alias(" haiku "));
        // 具体型号 ID 都不是别名
        assert!(!is_model_alias("claude-sonnet-4-6"));
        assert!(!is_model_alias("glm-5.2"));
        assert!(!is_model_alias("claude-opus-4-7-20260101"));
        assert!(!is_model_alias(""));
    }

    fn tmp_settings(content: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "ccm-health-test-{}-{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn pinned_model_detects_concrete_id_only() {
        let pinned = tmp_settings(r#"{"model":"glm-5.2","enabledPlugins":{}}"#);
        assert_eq!(pinned_model_in(&pinned), Some("glm-5.2".to_string()));
        let _ = fs::remove_file(&pinned);

        let alias = tmp_settings(r#"{"model":"sonnet"}"#);
        assert_eq!(pinned_model_in(&alias), None);
        let _ = fs::remove_file(&alias);

        let absent = tmp_settings(r#"{"enabledPlugins":{}}"#);
        assert_eq!(pinned_model_in(&absent), None);
        let _ = fs::remove_file(&absent);

        // 文件缺失 / 非法 JSON 都不告警
        assert_eq!(pinned_model_in(Path::new("Z:/no/such/file.json")), None);
        let broken = tmp_settings("not json");
        assert_eq!(pinned_model_in(&broken), None);
        let _ = fs::remove_file(&broken);
    }

    #[test]
    fn fmt_utc_converts_known_dates() {
        assert_eq!(fmt_utc(0), "1970-01-01 00:00:00 UTC");
        // 2024-01-01 00:00:00 UTC
        assert_eq!(fmt_utc(1_704_067_200), "2024-01-01 00:00:00 UTC");
        // 2026-07-09 12:34:56 UTC（20643 天 * 86400 + 45296 秒）
        assert_eq!(fmt_utc(1_783_600_496), "2026-07-09 12:34:56 UTC");
    }

    #[test]
    fn profile_summary_never_leaks_token() {
        let p = Profile {
            name: "corp".into(),
            type_: "router".into(),
            base_url: "https://gw.example.com/anthropic".into(),
            token_enc: Some("SECRET-ENCRYPTED-BLOB".into()),
            has_token: true,
            opus_model: "glm-5.2".into(),
            sonnet_model: String::new(),
            haiku_model: String::new(),
        };
        let s = profile_summary(&p);
        assert!(!s.contains("SECRET-ENCRYPTED-BLOB"));
        assert!(s.contains("已保存（内容不导出）"));
        assert!(s.contains("glm-5.2"));
        assert!(s.contains("<未设>"));
    }
}
