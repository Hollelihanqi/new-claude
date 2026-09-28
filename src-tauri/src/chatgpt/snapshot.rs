//! Materialize the immutable JSONL prefix chain described by SessionMeta.history_base.
//! Database indexes are derived data: they and credentials never cross profiles.
use super::*;
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
pub const LIMIT: u64 = 32 * 1024 * 1024;

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

pub fn materialize(path: &Path, candidates: &[PathBuf]) -> Result<Vec<u8>, String> {
    let mut seen = HashSet::new();
    let mut segments: Vec<Vec<Value>> = vec![];
    let mut next = path.to_path_buf();
    let mut cutoff: Option<(u64, u64)> = None;
    let mut leaf = None;
    let mut total = 0usize;
    loop {
        if seen.len() >= 128 || !seen.insert(next.clone()) {
            return Err("会话父链存在循环或超过 128 层".into());
        }
        let mut bytes = read(&next)?;
        let meta = header(&bytes)?;
        if leaf.is_none() {
            leaf = Some(meta.clone());
        }
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
        if let Some((offset, ordinal)) = cutoff {
            let offset = usize::try_from(offset).map_err(|_| "父会话边界溢出")?;
            if offset == 0 || offset > bytes.len() || bytes[offset - 1] != b'\n' {
                return Err("父会话片段缺失或字节边界错误".into());
            }
            bytes.truncate(offset);
            let count = bytes.iter().copied().filter(|b| *b == b'\n').count() as u64;
            if inherited.checked_add(count) != Some(ordinal) {
                return Err("父会话字节与序号边界不一致".into());
            }
        }
        if !bytes.ends_with(b"\n") {
            return Err("记录未完整写入，请关闭来源后重试".into());
        }
        total = total.checked_add(bytes.len()).ok_or("记录大小溢出")?;
        if total as u64 > LIMIT {
            return Err("合并后的历史超过 32 MB 上限".into());
        }
        let mut lines = Vec::new();
        for line in bytes
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .skip(1)
        {
            let value: Value = serde_json::from_slice(line).map_err(|_| "历史包含损坏的记录")?;
            if !matches!(
                value["type"].as_str(),
                Some("response_item" | "event_msg" | "turn_context" | "compacted")
            ) {
                return Err("历史包含尚未适配的记录类型".into());
            }
            lines.push(value);
        }
        segments.push(lines);
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
        cutoff = Some((
            base["end_byte_offset"]
                .as_u64()
                .ok_or("父记录字节边界缺失")?,
            inherited,
        ));
    }
    let mut meta = leaf.ok_or("记录为空")?;
    let payload = meta["payload"].as_object_mut().ok_or("元数据无效")?;
    payload.insert("history_mode".into(), Value::String("legacy".into()));
    for key in [
        "history_base",
        "forked_from_id",
        "forked_from_ordinal_exclusive",
        "parent_thread_id",
        "subagent_history_start_ordinal",
        "creator_user_id",
        "creator_account_id",
    ] {
        payload.remove(key);
    }
    let mut output = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    output.push(b'\n');
    for segment in segments.into_iter().rev() {
        for line in segment {
            serde_json::to_writer(&mut output, &line).map_err(|e| e.to_string())?;
            output.push(b'\n');
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
        let bytes = materialize(&child, &[parent.clone()]).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("inherited"));
        assert!(text.contains("child"));
        assert!(!text.contains("later-parent-only"));
        assert!(!text.contains("history_base"));
        fs::remove_file(parent).unwrap();
        assert!(materialize(&child, &[]).is_err());
    }
}
