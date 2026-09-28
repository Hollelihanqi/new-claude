//! Durable queue; each command performs one item so the UI can pause between copies.
use super::history::{TransferRequest, TransferResult};
use super::*;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub key: String,
    pub result: Option<TransferResult>,
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    #[serde(default)]
    pub created_at: u64,
    pub target_id: String,
    pub requests: Vec<TransferRequest>,
    pub outcomes: Vec<Outcome>,
    pub cancelled: bool,
    #[serde(default)]
    pub last_error: Option<String>,
}
fn directory(root: &Path, r: &Registry, target: &str) -> Result<PathBuf, String> {
    selected(r, target)?;
    let path = storage::profile_dir(root, target)?.join("batches");
    storage::private_dir(&path)?;
    Ok(path)
}
pub fn create(root: &Path, r: &Registry, requests: Vec<TransferRequest>) -> Result<Job, String> {
    if requests.is_empty() || requests.len() > 100 {
        return Err("每批请选择 1–100 条记录".into());
    }
    let target_id = requests[0].target_id.clone();
    let source = &requests[0].source_id;
    let mut keys = std::collections::HashSet::new();
    if requests.iter().any(|v| {
        v.target_id != target_id
            || &v.source_id != source
            || !keys.insert(&v.key)
            || v.fingerprint.is_none()
    }) {
        return Err("批量复制需要相同来源和目标、无重复记录且已完成预览".into());
    }
    let dir = directory(root, r, &target_id)?;
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        target_id,
        requests,
        outcomes: vec![],
        cancelled: false,
        last_error: None,
    };
    storage::write_json(&dir.join(format!("{}.json", job.id)), &job)?;
    Ok(job)
}
pub fn list(root: &Path, r: &Registry, target: &str) -> Result<Vec<Job>, String> {
    let dir = directory(root, r, target)?;
    let mut jobs = vec![];
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.take(1000) {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().is_some_and(|s| s == "json") {
            jobs.push(storage::read_json::<Job>(&entry.path())?);
        }
    }
    jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
    Ok(jobs)
}
pub fn step(
    root: &Path,
    r: &Registry,
    target: &str,
    id: &str,
    cancel: bool,
) -> Result<Job, String> {
    storage::validate_id(id)?;
    let path = directory(root, r, target)?.join(format!("{id}.json"));
    let mut job: Job = storage::read_json(&path)?;
    if job.target_id != target || job.id != id || job.requests.iter().any(|q| q.target_id != target)
    {
        return Err("复制队列归属不符".into());
    }
    if cancel {
        job.cancelled = true;
    }
    if !job.cancelled {
        job.last_error = None;
        if let Some(request) = job.requests.get(job.outcomes.len()).cloned() {
            let key = request.key.clone();
            match history::transfer(root, r, request.clone()) {
                Ok(result) => job.outcomes.push(Outcome {
                    key,
                    result: Some(result),
                    error: None,
                }),
                Err(error) if history::has_pending(root, &request) => job.last_error = Some(error),
                Err(error) => job.outcomes.push(Outcome {
                    key,
                    result: None,
                    error: Some(error),
                }),
            }
        }
    }
    storage::write_json(&path, &job)?;
    Ok(job)
}
