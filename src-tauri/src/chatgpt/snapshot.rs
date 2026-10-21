//! Materialize the immutable JSONL prefix chain described by SessionMeta.history_base.
//! Database indexes are derived data: they and credentials never cross profiles.
use super::*;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
/// Largest history (decompressed bytes) this build is willing to copy.
/// The streaming pipeline keeps peak memory at roughly one record, so this is
/// a safety valve rather than an in-memory buffer size.
pub const LIMIT: u64 = 512 * 1024 * 1024;

/// Streaming reader for a rollout file. Any extension other than `.zst` is
/// treated as plain JSONL; internal staging files use `.tmp`.
pub fn open(path: &Path) -> Result<Box<dyn BufRead>, String> {
    storage::plain(path)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if path.to_string_lossy().ends_with(".jsonl.zst") {
        Ok(Box::new(BufReader::new(
            zstd::stream::read::Decoder::new(file).map_err(|_| "压缩记录无法解码")?,
        )))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn header_line(reader: &mut dyn BufRead) -> Result<(Value, u64), String> {
    let mut line = Vec::new();
    let read = reader
        .read_until(b'\n', &mut line)
        .map_err(|_| "记录读取失败")?;
    if read == 0 {
        return Err("记录为空".into());
    }
    if line.last() != Some(&b'\n') {
        return Err("记录头无法识别".into());
    }
    line.pop();
    let value: Value = serde_json::from_slice(&line).map_err(|_| "记录头无法识别")?;
    if value["type"] != "session_meta" {
        return Err("记录缺少会话元数据".into());
    }
    Ok((value, read as u64))
}

#[allow(dead_code)] // byte-oriented reader for the ignored live-client tests
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    storage::plain(path)?;
    let before = fs::metadata(path).map_err(|e| e.to_string())?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = if path.to_string_lossy().ends_with(".jsonl.zst") {
        Box::new(zstd::stream::read::Decoder::new(file).map_err(|_| "压缩记录无法解码")?)
    } else if path.extension().is_some_and(|s| s == "jsonl") {
        Box::new(file)
    } else {
        return Err("不支持的记录格式".into());
    };
    let mut bytes = Vec::new();
    reader
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "记录读取失败")?;
    let after = fs::metadata(path).map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("读取期间记录发生变化，请关闭来源后重试".into());
    }
    if bytes.len() as u64 > LIMIT {
        return Err("解压后的记录超过 32 MB 上限".into());
    }
    Ok(bytes)
}

pub fn header(bytes: &[u8]) -> Result<Value, String> {
    let line = bytes.split(|b| *b == b'\n').next().ok_or("记录为空")?;
    let value: Value = serde_json::from_slice(line).map_err(|_| "记录头无法识别")?;
    if value["type"] != "session_meta" {
        return Err("记录缺少会话元数据".into());
    }
    Ok(value)
}

pub fn header_file(path: &Path) -> Result<Value, String> {
    storage::plain(path)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = if path.to_string_lossy().ends_with(".jsonl.zst") {
        Box::new(zstd::stream::read::Decoder::new(file).map_err(|_| "压缩记录无法解码")?)
    } else if path.extension().is_some_and(|s| s == "jsonl") {
        Box::new(file)
    } else {
        return Err("不支持的记录格式".into());
    };
    let mut bytes = Vec::new();
    reader
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| "记录头无法读取")?;
    let end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .ok_or("记录头超过 64 KB 或不完整")?;
    header(&bytes[..end])
}

#[derive(Clone, Copy)]
struct Boundary {
    offset: u64,
    ordinal: u64,
}

struct Segment {
    path: PathBuf,
    meta: Value,
    inherited: u64,
    cutoff: Option<Boundary>,
}

/// Parent chain resolved from headers only, so large bodies are never held.
pub struct Plan {
    segments: Vec<Segment>,
    paginated: bool,
    completed_end: Option<u64>,
}

/// Keep the leaf's history format. New paginated sessions persist UI-visible
/// item events that the official reader does not project in legacy mode.
const DROPPED_META_KEYS: [&str; 7] = [
    "history_base",
    "forked_from_id",
    "forked_from_ordinal_exclusive",
    "parent_thread_id",
    "subagent_history_start_ordinal",
    "creator_user_id",
    "creator_account_id",
];

pub fn plan(path: &Path, candidates: &[PathBuf], completed_only: bool) -> Result<Plan, String> {
    let mut seen = HashSet::new();
    let mut segments: Vec<Segment> = vec![];
    let mut next = path.to_path_buf();
    let mut cutoff: Option<Boundary> = None;
    loop {
        if seen.len() >= 128 || !seen.insert(next.clone()) {
            return Err("会话父链存在循环或超过 128 层".into());
        }
        let meta = header_file(&next)?;
        let mode = meta["payload"]["history_mode"].as_str().unwrap_or("legacy");
        if !matches!(mode, "legacy" | "paginated") {
            return Err("未知的历史格式，请更新适配器".into());
        }
        let base = &meta["payload"]["history_base"];
        let inherited = if base.is_null() {
            0
        } else {
            base["end_ordinal_exclusive"]
                .as_u64()
                .ok_or("父会话边界缺少序号")?
        };
        segments.push(Segment {
            path: next.clone(),
            meta: meta.clone(),
            inherited,
            cutoff,
        });
        if base.is_null() {
            break;
        }
        if mode != "paginated" {
            return Err("旧格式包含未知的父历史引用".into());
        }
        let parent = base["thread_id"].as_str().ok_or("父记录标识缺失")?;
        storage::validate_id(parent)?;
        // Reverted threads retain their logical ID; the filename holds the rollout ID.
        let suffix = format!("-{parent}.jsonl");
        let matches: Vec<_> = candidates
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.strip_suffix(".zst").unwrap_or(s).ends_with(&suffix))
            })
            .collect();
        if matches.len() != 1 {
            return Err("父记录缺失或有重复表示，无法确认完整历史".into());
        }
        next = matches[0].clone();
        cutoff = Some(Boundary {
            offset: base["end_byte_offset"]
                .as_u64()
                .ok_or("父记录字节边界缺失")?,
            ordinal: inherited,
        });
    }
    let paginated = segments[0].meta["payload"]["history_mode"] == "paginated";
    let completed_end = if completed_only {
        Some(completed_offset(path)?)
    } else {
        None
    };
    Ok(Plan {
        segments,
        paginated,
        completed_end,
    })
}

impl Plan {
    pub fn paginated(&self) -> bool {
        self.paginated
    }
}

fn same_meta(meta: &Value, planned: &Value) -> bool {
    meta["payload"]["id"] == planned["payload"]["id"]
        && meta["payload"]["history_mode"] == planned["payload"]["history_mode"]
        && meta["payload"]["history_base"] == planned["payload"]["history_base"]
}

/// Streams the resolved history root-first. `transform` receives every record
/// and decides what to serialize; malformed or oversized input still fails.
pub fn emit<F>(plan: &Plan, mut transform: F) -> Result<(), String>
where
    F: FnMut(&mut Value, bool) -> Result<(), String>,
{
    // The merged history keeps only the leaf's header, and it must come first.
    let leaf = &plan.segments[0];
    {
        let mut reader = open(&leaf.path)?;
        let (mut meta, _) = header_line(&mut *reader)?;
        if !same_meta(&meta, &leaf.meta) {
            return Err("读取期间记录发生变化，请关闭来源后重试".into());
        }
        if let Some(payload) = meta["payload"].as_object_mut() {
            for key in DROPPED_META_KEYS {
                payload.remove(key);
            }
        }
        if plan.paginated {
            meta["ordinal"] = 0.into();
        }
        transform(&mut meta, true)?;
    }
    let mut ordinal = 1_u64;
    let mut total = 0_u64;
    let mut buffer = Vec::new();
    for (index, segment) in plan.segments.iter().enumerate().rev() {
        let is_leaf = index == 0;
        let before = fs::metadata(&segment.path).map_err(|e| e.to_string())?;
        let mut reader = open(&segment.path)?;
        let (meta, header_bytes) = header_line(&mut *reader)?;
        if !same_meta(&meta, &segment.meta) {
            return Err("读取期间记录发生变化，请关闭来源后重试".into());
        }
        total = total.checked_add(header_bytes).ok_or("记录大小溢出")?;
        if total > LIMIT {
            return Err("合并后的历史超过 512 MB 上限".into());
        }
        let mut consumed = header_bytes;
        let mut lines = 1_u64;
        if let Some(cutoff) = segment.cutoff {
            if consumed > cutoff.offset {
                return Err("父会话片段缺失或字节边界错误".into());
            }
        }
        loop {
            if let Some(cutoff) = segment.cutoff {
                if consumed == cutoff.offset {
                    break;
                }
            }
            buffer.clear();
            let read = reader
                .read_until(b'\n', &mut buffer)
                .map_err(|_| "记录读取失败")?;
            if read == 0 {
                break;
            }
            if let Some(end) = plan.completed_end.filter(|_| is_leaf) {
                if consumed + read as u64 > end {
                    break;
                }
            }
            if let Some(cutoff) = segment.cutoff {
                if consumed + read as u64 > cutoff.offset {
                    return Err("父会话片段缺失或字节边界错误".into());
                }
            }
            consumed += read as u64;
            lines += 1;
            total = total.checked_add(read as u64).ok_or("记录大小溢出")?;
            if total > LIMIT {
                return Err("合并后的历史超过 512 MB 上限".into());
            }
            if buffer.last() != Some(&b'\n') {
                return Err("记录未完整写入，请关闭来源后重试".into());
            }
            let body = &buffer[..buffer.len() - 1];
            if body.is_empty() {
                continue;
            }
            let mut value: Value =
                serde_json::from_slice(body).map_err(|_| "历史包含损坏的记录")?;
            if !matches!(
                value["type"].as_str(),
                Some(
                    "response_item"
                        | "event_msg"
                        | "turn_context"
                        | "compacted"
                        | "token_usage_record"
                        | "world_state"
                )
            ) {
                return Err("历史包含尚未适配的记录类型".into());
            }
            if plan.paginated {
                value["ordinal"] = ordinal.into();
                ordinal += 1;
            }
            transform(&mut value, false)?;
        }
        if let Some(cutoff) = segment.cutoff {
            if consumed != cutoff.offset {
                return Err("父会话片段缺失或字节边界错误".into());
            }
            if segment.inherited.checked_add(lines) != Some(cutoff.ordinal) {
                return Err("父会话字节与序号边界不一致".into());
            }
        }
        let after = fs::metadata(&segment.path).map_err(|e| e.to_string())?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("读取期间记录发生变化，请关闭来源后重试".into());
        }
    }
    Ok(())
}

pub fn materialize(
    path: &Path,
    candidates: &[PathBuf],
    completed_only: bool,
) -> Result<Vec<u8>, String> {
    let plan = plan(path, candidates, completed_only)?;
    let mut output = Vec::new();
    emit(&plan, |value, _| {
        serde_json::to_writer(&mut output, value).map_err(|e| e.to_string())?;
        output.push(b'\n');
        Ok(())
    })?;
    Ok(output)
}

fn completed_offset(path: &Path) -> Result<u64, String> {
    let mut reader = open(path)?;
    let mut line = Vec::new();
    let mut offset = 0_u64;
    let mut end = None;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|_| "记录读取失败")?;
        if read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            break;
        }
        offset += read as u64;
        let value: Value =
            serde_json::from_slice(&line[..line.len() - 1]).map_err(|_| "会话记录尚未完整写入")?;
        if value["type"] == "event_msg" && value["payload"]["type"] == "task_complete" {
            end = Some(offset);
        }
    }
    end.ok_or_else(|| "会话尚无完成的回复，请等待当前回复结束后重试".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn running_session_uses_only_last_completed_reply() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.join(format!("rollout-{id}.jsonl"));
        let lines = [
            json!({"type":"session_meta","payload":{"id":id,"history_mode":"legacy"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"before"}]}}),
            json!({"type":"token_usage_record","payload":{"thread_id":id}}),
            json!({"type":"world_state","payload":{"full":true,"state":{}}}),
            json!({"type":"event_msg","payload":{"type":"task_complete"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"unfinished"}]}}),
        ];
        fs::write(
            &path,
            lines.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
        let snapshot = materialize(&path, &[], true).unwrap();
        let text = String::from_utf8(snapshot).unwrap();
        assert!(text.contains("before"));
        assert!(!text.contains("unfinished"));
        assert!(text.contains("world_state"));
        assert!(text.contains("token_usage_record"));
        assert!(materialize(&path, &[], false).unwrap().len() > text.len());
    }
    #[test]
    fn streaming_materialize_renumbers_paginated_chain_root_first() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let parent_id = uuid::Uuid::new_v4().to_string();
        let prefix = format!(
            "{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":parent_id,"history_mode":"paginated"}}),
            json!({"type":"response_item","payload":{"text":"inherited"}})
        );
        let parent = root.join(format!("rollout-{parent_id}.jsonl.zst"));
        let full = format!(
            "{prefix}{}\n",
            json!({"type":"response_item","payload":{"text":"later-parent-only"}})
        );
        fs::write(
            &parent,
            zstd::stream::encode_all(full.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let child_id = uuid::Uuid::new_v4().to_string();
        let child = root.join(format!("rollout-{child_id}.jsonl"));
        fs::write(
            &child,
            format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":child_id,"history_mode":"paginated","history_base":{"thread_id":parent_id,"end_ordinal_exclusive":2,"end_byte_offset":prefix.len()}}}),
                json!({"type":"response_item","payload":{"text":"child"}})
            ),
        )
        .unwrap();
        let bytes = materialize(&child, std::slice::from_ref(&parent), false).unwrap();
        let values: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0]["ordinal"], 0);
        assert_eq!(values[0]["payload"]["history_mode"], "paginated");
        assert!(values[0]["payload"]["history_base"].is_null());
        assert_eq!(values[1]["payload"]["text"], "inherited");
        assert_eq!(values[1]["ordinal"], 1);
        assert_eq!(values[2]["payload"]["text"], "child");
        assert_eq!(values[2]["ordinal"], 2);
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("later-parent-only"));
    }

    #[test]
    fn parent_cutoff_must_land_on_a_line_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let parent_id = uuid::Uuid::new_v4().to_string();
        let parent = root.join(format!("rollout-{parent_id}.jsonl"));
        let prefix = format!(
            "{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":parent_id,"history_mode":"paginated"}}),
            json!({"type":"response_item","payload":{"text":"inherited"}})
        );
        fs::write(&parent, &prefix).unwrap();
        let child = root.join("rollout-child.jsonl");
        fs::write(
            &child,
            format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":uuid::Uuid::new_v4().to_string(),"history_mode":"paginated","history_base":{"thread_id":parent_id,"end_ordinal_exclusive":2,"end_byte_offset":prefix.len() - 3}}}),
                json!({"type":"response_item","payload":{"text":"child"}})
            ),
        )
        .unwrap();
        let error = materialize(&child, std::slice::from_ref(&parent), false).unwrap_err();
        assert_eq!(error, "父会话片段缺失或字节边界错误");
    }

    #[test]
    fn compressed_parent_cutoff_ignores_later_parent_work() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let parent_id = uuid::Uuid::new_v4().to_string();
        let parent = root.join(format!("rollout-{parent_id}.jsonl.zst"));
        let prefix = format!(
            "{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":parent_id,"history_mode":"paginated"}}),
            json!({"type":"response_item","payload":{"text":"inherited"}})
        );
        let full = format!(
            "{prefix}{}\n",
            json!({"type":"response_item","payload":{"text":"later-parent-only"}})
        );
        fs::write(
            &parent,
            zstd::stream::encode_all(full.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let child = root.join("rollout-child.jsonl");
        fs::write(&child, format!("{}\n{}\n", json!({"type":"session_meta","payload":{"id":uuid::Uuid::new_v4(),"history_mode":"paginated","history_base":{"thread_id":parent_id,"end_ordinal_exclusive":2,"end_byte_offset":prefix.len()}}}), json!({"type":"response_item","payload":{"text":"child"}}))).unwrap();
        let bytes = materialize(&child, &[parent.clone()], false).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("inherited"));
        assert!(text.contains("child"));
        assert!(!text.contains("later-parent-only"));
        assert!(!text.contains("history_base"));
        fs::remove_file(parent).unwrap();
        assert!(materialize(&child, &[], false).is_err());
    }
}
