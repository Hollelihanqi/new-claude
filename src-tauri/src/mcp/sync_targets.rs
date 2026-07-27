use super::codex_sync;
use super::storage::McpPaths;
use super::{McpLocator, McpState, McpSyncApplyRequest, McpSyncPreview, McpTargetDisableRequest};

/// 同步目标适配器边界。
///
/// 通用 MCP 管理流程只认识 target_id 和这组操作；目标配置格式、路径、
/// 兼容性及写入策略全部封装在具体适配器中。新增客户端时实现并注册新适配器，
/// 不需要修改前端列表结构或复制 Tauri 命令。
trait McpSyncTargetAdapter: Sync {
    fn id(&self) -> &'static str;
    fn attach_state(&self, paths: &McpPaths, state: McpState) -> McpState;
    fn preview(
        &self,
        paths: &McpPaths,
        state: &McpState,
        locator: &McpLocator,
    ) -> Result<McpSyncPreview, String>;
    fn apply(
        &self,
        paths: &McpPaths,
        state: &McpState,
        request: &McpSyncApplyRequest,
    ) -> Result<(), String>;
    fn disable(
        &self,
        paths: &McpPaths,
        state: &McpState,
        request: &McpTargetDisableRequest,
    ) -> Result<(), String>;
    fn reconcile_source_enabled(
        &self,
        paths: &McpPaths,
        state: &McpState,
        locator: &McpLocator,
    ) -> Result<(), String>;
    fn start_monitor(&self, app: tauri::AppHandle);
}

struct CodexSyncTargetAdapter;

impl McpSyncTargetAdapter for CodexSyncTargetAdapter {
    fn id(&self) -> &'static str {
        codex_sync::TARGET_ID
    }

    fn attach_state(&self, paths: &McpPaths, state: McpState) -> McpState {
        codex_sync::attach_sync_state(paths, state)
    }

    fn preview(
        &self,
        paths: &McpPaths,
        state: &McpState,
        locator: &McpLocator,
    ) -> Result<McpSyncPreview, String> {
        codex_sync::preview_sync(paths, state, locator)
    }

    fn apply(
        &self,
        paths: &McpPaths,
        state: &McpState,
        request: &McpSyncApplyRequest,
    ) -> Result<(), String> {
        codex_sync::apply_manual_sync(paths, state, request)
    }

    fn disable(
        &self,
        paths: &McpPaths,
        state: &McpState,
        request: &McpTargetDisableRequest,
    ) -> Result<(), String> {
        codex_sync::disable_target(paths, state, request)
    }

    fn reconcile_source_enabled(
        &self,
        paths: &McpPaths,
        state: &McpState,
        locator: &McpLocator,
    ) -> Result<(), String> {
        codex_sync::reconcile_source_enabled(paths, state, locator)
    }

    fn start_monitor(&self, app: tauri::AppHandle) {
        codex_sync::start_monitor(app);
    }
}

static CODEX_TARGET: CodexSyncTargetAdapter = CodexSyncTargetAdapter;

fn adapters() -> [&'static dyn McpSyncTargetAdapter; 1] {
    [&CODEX_TARGET]
}

fn adapter(target_id: &str) -> Result<&'static dyn McpSyncTargetAdapter, String> {
    adapters()
        .into_iter()
        .find(|candidate| candidate.id() == target_id)
        .ok_or_else(|| format!("未知的 MCP 同步目标：{target_id}"))
}

pub(crate) fn attach_sync_state(paths: &McpPaths, mut state: McpState) -> McpState {
    for target in adapters() {
        state = target.attach_state(paths, state);
    }
    state
}

pub(crate) fn preview_sync(
    target_id: &str,
    paths: &McpPaths,
    state: &McpState,
    locator: &McpLocator,
) -> Result<McpSyncPreview, String> {
    adapter(target_id)?.preview(paths, state, locator)
}

pub(crate) fn apply_sync(
    paths: &McpPaths,
    state: &McpState,
    request: &McpSyncApplyRequest,
) -> Result<(), String> {
    adapter(&request.target_id)?.apply(paths, state, request)
}

pub(crate) fn disable_target(
    paths: &McpPaths,
    state: &McpState,
    request: &McpTargetDisableRequest,
) -> Result<(), String> {
    adapter(&request.target_id)?.disable(paths, state, request)
}

pub(crate) fn reconcile_source_enabled(
    paths: &McpPaths,
    state: &McpState,
    locator: &McpLocator,
) -> Result<(), String> {
    for target in adapters() {
        target.reconcile_source_enabled(paths, state, locator)?;
    }
    Ok(())
}

pub(crate) fn start_monitors(app: tauri::AppHandle) {
    for target in adapters() {
        target.start_monitor(app.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_dispatches_registered_target_and_rejects_unknown_target() {
        assert_eq!(adapter("codex").unwrap().id(), "codex");
        assert!(adapter("future-client").is_err());
    }
}
