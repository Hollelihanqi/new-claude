import { usePageActivation, usePageActive } from "../PersistentPage";
// MCP 服务管理页：状态摘要 + 筛选工具栏 + 服务定义列表 + 预览/确认/应用流程。
// 不负责配置文件语义；所有写操作统一走 preview → confirm → apply。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Badge,
  Box,
  Button,
  Card,
  Code,
  Group,
  Loader,
  Menu,
  Modal,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Table,
  Tabs,
  Text,
  TextInput,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import {
  IconAlertTriangle,
  IconCheck,
  IconCopy,
  IconDownload,
  IconFolderPlus,
  IconFolderX,
  IconEye,
  IconInfoCircle,
  IconPencil,
  IconPlus,
  IconSearch,
  IconTrash,
} from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../api";
import type {
  McpChangeAction,
  McpChangePreview,
  McpChangeRequest,
  McpConnectionCheck,
  McpConnectionReport,
  McpScope,
  McpService,
  McpState,
  McpSyncPreview,
  McpSyncTargetInfo,
  McpTestResult,
  McpUpdateInfo,
  McpUpdateReport,
} from "../../api";
import McpImportModal from "./McpImportModal";
import StableRefreshButton from "../StableRefreshButton";
import McpServiceDrawer from "./McpServiceDrawer";
import McpSourceIssuesCard, { cleanupOutcomeView } from "./McpSourceIssuesCard";
import McpSummaryGrid from "./McpSummaryGrid";
import McpConnectionBadge from "./McpConnectionBadge";
import FeatureHelp from "../FeatureHelp";
import { MCP_HELP, MCP_SCOPE_HELP } from "../featureHelpContent";
import {
  SCOPE_BADGE_COLOR,
  SCOPE_LABELS,
  TRANSPORT_LABELS,
  buildSyncTargetColumns,
  locatorKey,
  redactConfig,
  serviceContextLabel,
  serviceSearchText,
  syncTargetDisplayLabel,
  type SyncTargetColumn,
} from "./mcpForm";

type DrawerState = { mode: "create" } | { mode: "edit" | "copy"; service: McpService };

const EFFECTIVE_LABEL: Record<string, string> = {
  effective: "生效",
  "partially-shadowed": "部分覆盖",
  shadowed: "被覆盖",
  disabled: "未加载",
};
const EFFECTIVE_COLOR: Record<string, string> = {
  effective: "teal",
  "partially-shadowed": "orange",
  shadowed: "gray",
  disabled: "red",
};

let cachedMcpState: McpState | null = null;
let cachedConnectionReport: McpConnectionReport | null = null;
let cachedConnectionAt = 0;
let cachedUpdateReport: McpUpdateReport | null = null;

function rememberMcpState(nextState: McpState) {
  cachedMcpState = nextState;
  return nextState;
}

export default function McpPanel() {
  const pageActive = usePageActive();
  const [state, setState] = useState<McpState | null>(() => cachedMcpState);
  const [connectionReport, setConnectionReport] = useState<McpConnectionReport | null>(
    () => cachedConnectionReport
  );
  const [connectionBusy, setConnectionBusy] = useState(false);
  const [updateReport, setUpdateReport] = useState<McpUpdateReport | null>(
    () => cachedUpdateReport
  );
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateToggleBusy, setUpdateToggleBusy] = useState("");
  const [rowUpdateBusy, setRowUpdateBusy] = useState("");
  // 正在恢复哪一行（"环境:条目名"）—— 逐行 loading，不用整页的 busy
  const [restoringRow, setRestoringRow] = useState("");
  // 「一键清理死条目」进行中（RiskConfirm 的确认按钮与卡内按钮共用）
  const [cleaningIssues, setCleaningIssues] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [query, setQuery] = useState("");
  const [scopeFilter, setScopeFilter] = useState<McpScope | "all">("all");
  const [instanceFilter, setInstanceFilter] = useState<string | "all">("all");
  const [projectFilter, setProjectFilter] = useState<string | "all">("all");
  const [drawerState, setDrawerState] = useState<DrawerState | null>(null);
  const [detailService, setDetailService] = useState<McpService | null>(null);
  const [importOpened, setImportOpened] = useState(false);
  const [previewState, setPreviewState] = useState<{
    request: McpChangeRequest;
    preview: McpChangePreview;
  } | null>(null);
  const [applying, setApplying] = useState(false);
  const [syncPreview, setSyncPreview] = useState<McpSyncPreview | null>(null);
  const [syncApplying, setSyncApplying] = useState(false);
  const [syncBusyKey, setSyncBusyKey] = useState("");

  const loadQueue = useRef<Promise<void>>(Promise.resolve());
  const connectionQueue = useRef<Promise<void>>(Promise.resolve());
  const updateQueue = useRef<Promise<unknown>>(Promise.resolve());
  const load = useCallback((quiet = false) => {
    // 刷新必须串行排队，不能在已有请求进行时静默丢弃。尤其是清理完成后的刷新：
    // 若它被页面激活时的旧请求挡掉，旧请求会把清理前状态重新写回界面。
    const task = loadQueue.current.catch(() => undefined).then(async () => {
      if (!quiet) setBusy(true);
      setErr("");
      try {
        const nextState = await api.listMcpServices();
        setState(rememberMcpState(nextState));
      } catch (e) {
        setErr(String(e));
      } finally {
        setBusy(false);
      }
    });
    loadQueue.current = task;
    return task;
  }, []);

  const probeConnections = useCallback(() => {
    const task = connectionQueue.current.catch(() => undefined).then(async () => {
      setConnectionBusy(true);
      try {
        const report = await api.probeMcpConnections();
        cachedConnectionReport = report;
        cachedConnectionAt = Date.now();
        setConnectionReport(report);
      } catch (e) {
        const report = { checks: [], errors: [String(e)] };
        cachedConnectionReport = report;
        cachedConnectionAt = Date.now();
        setConnectionReport(report);
      } finally {
        setConnectionBusy(false);
      }
    });
    connectionQueue.current = task;
    return task;
  }, []);

  const loadUpdateInfo = useCallback(() => {
    const task = updateQueue.current.catch(() => undefined).then(async () => {
      try {
        const report = await api.listMcpUpdateInfo();
        cachedUpdateReport = report;
        setUpdateReport(report);
      } catch (e) {
        notifications.show({
          color: "red",
          title: "无法读取更新设置",
          message: String(e),
        });
      }
    });
    updateQueue.current = task;
    return task;
  }, []);

  const checkUpdates = useCallback((target?: McpService["locator"]) => {
    const task = updateQueue.current.catch(() => undefined).then(async () => {
      if (!target) setUpdateBusy(true);
      try {
        const report = await api.checkMcpUpdates(target);
        const next = target && cachedUpdateReport
          ? {
              entries: cachedUpdateReport.entries.map((entry) =>
                locatorKey(entry.locator) === locatorKey(target)
                  ? report.entries[0] ?? entry
                  : entry
              ),
              errors: report.errors,
            }
          : report;
        cachedUpdateReport = next;
        setUpdateReport(next);
        if (report.errors.length > 0) {
          notifications.show({
            color: "orange",
            title: target ? "当前 MCP 版本检测未完成" : "部分版本检测未完成",
            message: report.errors.join("；"),
          });
        }
        return report;
      } catch (e) {
        notifications.show({ color: "red", title: "版本检测失败", message: String(e) });
        return null;
      } finally {
        if (!target) setUpdateBusy(false);
      }
    });
    updateQueue.current = task;
    return task;
  }, []);

  const detectAll = useCallback(async () => {
    await load();
    await Promise.all([probeConnections(), checkUpdates()]);
  }, [load, probeConnections, checkUpdates]);

  // 「恢复使用共享配置」：**只有用户显式点它**才会撤销覆盖（决策 7.2）。
  const onRestoreSharedEntry = async (env: string, name: string) => {
    // 单独一个状态：`busy` 是整页的布尔量，用它做逐行 loading 会把整页卡住
    setRestoringRow(`${env}:${name}`);
    try {
      const message = await api.restoreSharedMcpEntry(env, name);
      notifications.show({ message, color: "teal" });
      load(true);
    } catch (e) {
      setErr(String(e));
    } finally {
      setRestoringRow("");
    }
  };

  // 「一键清理死条目」：返回 boolean 让 RiskConfirm 决定是否关闭。
  // 未完成（写失败或并发冲突，即 complete=false）→ 橙色通知 + 弹窗保持打开可重试；
  // 无事可做 → 中性灰；全部成功 → 绿色。绝不能把"还有没清掉的"伪装成成功。
  const onCleanupDeadEntries = async () => {
    setCleaningIssues(true);
    try {
      const result = await api.cleanupDeadProjectEntries();
      const view = cleanupOutcomeView(result);
      notifications.show({
        color: view.color,
        title: view.title,
        message: result.message,
      });
      load(true);
      return view.closeDialog;
    } catch (e) {
      notifications.show({ color: "red", title: "清理失败", message: String(e) });
      return false;
    } finally {
      setCleaningIssues(false);
    }
  };

  useEffect(() => {
    void Promise.all([load(), probeConnections(), loadUpdateInfo()]);
  }, [load, probeConnections, loadUpdateInfo]);
  usePageActivation(() => {
    void load(true);
    void loadUpdateInfo();
    if (Date.now() - cachedConnectionAt > 30_000) void probeConnections();
  });

  async function setUpdateCheck(service: McpService, enabled: boolean) {
    const key = locatorKey(service.locator);
    setUpdateToggleBusy(key);
    try {
      const entry = await api.setMcpUpdateCheck(service.locator, enabled);
      const current = cachedUpdateReport ?? { entries: [], errors: [] };
      const exists = current.entries.some(
        (item) => locatorKey(item.locator) === key
      );
      const next = {
        entries: exists
          ? current.entries.map((item) => locatorKey(item.locator) === key ? entry : item)
          : [...current.entries, entry],
        errors: current.errors,
      };
      cachedUpdateReport = next;
      setUpdateReport(next);
    } catch (e) {
      notifications.show({ color: "red", title: "无法修改更新检测", message: String(e) });
    } finally {
      setUpdateToggleBusy("");
    }
  }

  async function updateCurrentService(service: McpService) {
    const key = locatorKey(service.locator);
    setRowUpdateBusy(key);
    try {
      const report = await checkUpdates(service.locator);
      const entry = report?.entries.find(
        (item) => locatorKey(item.locator) === key
      );
      if (!entry || report?.errors.length) return;
      if (entry.updateAvailable && entry.nextConfig) {
        await prepareChange({
          op: "save",
          original: service.locator,
          target: service.locator,
          config: entry.nextConfig,
          overwrite: true,
        });
        return;
      }
      notifications.show({
        color: "teal",
        title: entry.currentVersion ? "当前已是最新版本" : "当前配置自动跟随最新版",
        message: entry.latestVersion
          ? `${entry.packageName ?? service.locator.name} · v${entry.latestVersion}`
          : "未获得可比较的版本信息",
      });
    } catch (e) {
      notifications.show({ color: "red", title: "无法准备更新", message: String(e) });
    } finally {
      setRowUpdateBusy("");
    }
  }

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen("mcp-sync-target-updated", () => load()).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [load]);

  const filtered = useMemo(() => {
    if (!state) return [];
    const q = query.trim().toLowerCase();
    return state.services.filter((s) => {
      if (scopeFilter !== "all" && s.locator.scope !== scopeFilter) return false;
      // User/Project 无环境，不因环境筛选被隐藏；仅 Local 按环境筛
      if (s.locator.scope === "local" && instanceFilter !== "all" && s.locator.instanceId !== instanceFilter)
        return false;
      // User 不因项目筛选被隐藏
      if (s.locator.scope !== "user" && projectFilter !== "all" && s.locator.projectPath !== projectFilter)
        return false;
      if (q && !serviceSearchText(s).includes(q)) return false;
      return true;
    });
  }, [state, query, scopeFilter, instanceFilter, projectFilter]);

  const syncTargetsByKey = useMemo(
    () => {
      const grouped = new Map<string, McpSyncTargetInfo[]>();
      for (const info of state?.syncTargets ?? []) {
        const key = locatorKey(info.locator);
        grouped.set(key, [...(grouped.get(key) ?? []), info]);
      }
      return grouped;
    },
    [state]
  );

  const syncTargetColumns = useMemo<SyncTargetColumn[]>(() => {
    return buildSyncTargetColumns(state?.syncTargets ?? []);
  }, [state]);

  async function prepareChange(action: McpChangeAction) {
    if (!state) return;
    const request: McpChangeRequest = { action, expectedRevisions: state.revisions };
    // 失败时向调用方重新抛出错误；由调用方决定关闭/清空或提示
    const preview = await api.previewMcpChange(request);
    setPreviewState({
      request: { action, expectedRevisions: preview.expectedRevisions },
      preview,
    });
  }

  function prepareChangeSafe(action: McpChangeAction) {
    // 表格内开关/删除等无 await 处理的入口：捕获预览失败并提示
    prepareChange(action).catch((e) =>
      notifications.show({ color: "red", title: "无法预览变更", message: String(e) })
    );
  }

  async function confirmChange() {
    if (!previewState) return;
    setApplying(true);
    try {
      const next = await api.applyMcpChange(previewState.request);
      setState(rememberMcpState(next));
      setPreviewState(null);
      setDrawerState(null);
      setDetailService(null);
      setImportOpened(false);
      void loadUpdateInfo();
      if (next.operationWarnings.length > 0) {
        notifications.show({
          color: "orange",
          title: "主配置已保存，部分环境待同步",
          message: next.operationWarnings.join("；"),
        });
      } else {
        notifications.show({ color: "teal", message: "变更已应用", icon: <IconCheck size={16} /> });
      }
      void probeConnections();
    } catch (e) {
      const msg = String(e);
      if (msg.includes("外部修改") || msg.includes("刷新")) {
        notifications.show({ color: "orange", title: "配置已被外部修改", message: "请刷新后重试" });
        setPreviewState(null);
        load();
      } else {
        notifications.show({ color: "red", title: "应用失败", message: msg });
      }
    } finally {
      setApplying(false);
    }
  }

  async function addProject() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择包含 MCP 配置的项目目录",
      });
      if (typeof selected !== "string") return; // 取消或异常数组都不处理
      const next = await api.registerMcpProject(selected);
      setState(rememberMcpState(next));
      notifications.show({ color: "teal", message: "项目已添加" });
    } catch (e) {
      notifications.show({ color: "red", title: "添加项目失败", message: String(e) });
    }
  }

  async function onTest(name: string, config: Record<string, unknown>): Promise<McpTestResult> {
    return api.testMcpServer({ name, config });
  }

  async function unregisterProject(path: string) {
    try {
      const next = await api.unregisterMcpProject(path);
      setState(rememberMcpState(next));
      notifications.show({ color: "teal", message: "已取消登记（项目文件未删除）" });
    } catch (e) {
      notifications.show({ color: "red", title: "取消登记失败", message: String(e) });
    }
  }

  async function previewTargetSync(
    service: McpService,
    target: McpSyncTargetInfo
  ) {
    const key = `${locatorKey(service.locator)}::${target.targetId}`;
    setSyncBusyKey(key);
    try {
      const preview = await api.previewMcpTargetSync(target.targetId, service.locator);
      setSyncPreview(preview);
    } catch (e) {
      notifications.show({
        color: "red",
        title: "无法生成同步预览",
        message: String(e),
      });
    } finally {
      setSyncBusyKey("");
    }
  }

  async function confirmTargetSync() {
    if (!syncPreview) return;
    const targetLabel = syncTargetDisplayLabel(
      syncPreview.targetId,
      syncPreview.targetLabel
    );
    setSyncApplying(true);
    try {
      const next = await api.applyMcpTargetSync({
        locator: syncPreview.locator,
        targetId: syncPreview.targetId,
        expectedSourceRevision: syncPreview.expectedSourceRevision,
        expectedTargetRevision: syncPreview.expectedTargetRevision,
        expectedRegistryRevision: syncPreview.expectedRegistryRevision,
      });
      setState(rememberMcpState(next));
      setSyncPreview(null);
      notifications.show({
        color: "teal",
        title: `已接入 ${targetLabel}`,
        message: `后续来源配置更新时将自动同步。${syncPreview.restartHint}`,
      });
    } catch (e) {
      notifications.show({
        color: "red",
        title: "同步失败",
        message: String(e),
      });
    } finally {
      setSyncApplying(false);
    }
  }

  async function changeTargetSync(
    service: McpService,
    target: McpSyncTargetInfo,
    enabled: boolean
  ) {
    if (!enabled) {
      await disableTarget(service, target);
      return;
    }
    if (target.status === "incompatible") return;
    await previewTargetSync(service, target);
  }

  async function disableTarget(
    service: McpService,
    target: McpSyncTargetInfo
  ) {
    if (!state) return;
    const key = `${locatorKey(service.locator)}::${target.targetId}`;
    const targetLabel = syncTargetDisplayLabel(target.targetId, target.targetLabel);
    setSyncBusyKey(key);
    try {
      const next = await api.disableMcpTarget({
        locator: service.locator,
        targetId: target.targetId,
        expectedTargetRevision: target.targetRevision,
        expectedRegistryRevision:
          state.syncTargetRevisions[target.targetId] ?? "missing",
      });
      setState(rememberMcpState(next));
      notifications.show({
        color: "teal",
        title: `已关闭 ${targetLabel} 使用`,
        message: "调用和自动同步已停止；两端的 MCP 配置均已保留。",
      });
    } catch (e) {
      notifications.show({
        color: "red",
        title: `无法关闭 ${targetLabel} 使用`,
        message: String(e),
      });
    } finally {
      setSyncBusyKey("");
    }
  }

  const summary = state?.summary;

  return (
    <div className="view-scroll mcp-page">
      <Card withBorder radius="lg" className="mcp-overview-card">
      <Group justify="space-between" align="flex-start" mb="md">
        <div>
          <Group gap={6} align="center">
            <Text fw={700}>服务概览</Text>
            <FeatureHelp content={MCP_HELP} />
          </Group>
          <Text size="sm" c="dimmed">
            管理「所有环境」、「指定环境」和「当前项目」三种作用范围的 MCP 配置：
            写入共享库的会「自动分发」到每个环境，各环境也可单独覆盖。
          </Text>
        </div>
        <StableRefreshButton
          busy={busy || connectionBusy || updateBusy}
          busyLabel="检测中…"
          label="检测"
          onClick={detectAll}
        />
      </Group>

      <McpSummaryGrid
        summary={summary}
        sharedOverrideCount={state?.sharedOverrides?.length}
      />
      </Card>

      {/* 决策 7.2：同名时环境配置优先，但**必须显示冲突**。
          静默保留会让用户以为共享值已经生效，等到发现不一致时无从判断是哪一步的问题。 */}
      {state && (state.sharedOverrides?.length ?? 0) > 0 && (
        <Alert
          color="orange"
          variant="light"
          icon={<IconAlertTriangle size={16} />}
          title={`有 ${state.sharedOverrides.length} 条环境配置覆盖了共享配置`}
        >
          <Stack gap={6}>
            {state.sharedOverrides.map((o) => (
              <Group
                key={`${o.env}:${o.name}`}
                gap="xs"
                justify="space-between"
                wrap="nowrap"
                align="flex-start"
              >
                <Text size="sm">
                  环境 <Code>{o.env}</Code> 的 <Code>{o.name}</Code>：{o.reason}。
                  {o.sharedValue ? (
                    <> 共享值为 <Code>{JSON.stringify(o.sharedValue)}</Code>；</>
                  ) : (
                    <> 共享库里已经没有这一条；</>
                  )}
                  {o.envValue ? (
                    <> 该环境当前是 <Code>{JSON.stringify(o.envValue)}</Code>。</>
                  ) : (
                    <> 该环境已把它删除。</>
                  )}
                </Text>
                {o.sharedValue ? (
                  <Button
                    size="xs"
                    variant="light"
                    style={{ flexShrink: 0 }}
                    loading={restoringRow === `${o.env}:${o.name}`}
                    onClick={() => onRestoreSharedEntry(o.env, o.name)}
                  >
                    恢复使用共享配置
                  </Button>
                ) : null}
              </Group>
            ))}
            <Text size="xs" c="dimmed">
              覆盖不会被自动改写 —— 只有你点「恢复使用共享配置」才会改回共享值，
              共享库后续的更新也会一直绕过这些条目。
            </Text>
          </Stack>
        </Alert>
      )}

      {err && (
        <Alert color="red" icon={<IconAlertTriangle size={16} />} title="加载失败">{err}</Alert>
      )}

      {state && (
        <McpSourceIssuesCard
          issues={state.issues}
          deadEntries={state.deadEntries}
          pageActive={pageActive}
          busy={cleaningIssues}
          onCleanup={onCleanupDeadEntries}
        />
      )}

      {state && state.operationWarnings.length > 0 && (
        <Alert color="orange" icon={<IconAlertTriangle size={16} />} title="同步警告">
          {state.operationWarnings.map((warning, index) => (
            <Text size="xs" key={index}>{warning}</Text>
          ))}
        </Alert>
      )}

      <Card
        withBorder
        padding={0}
        className="mcp-table-card"
        radius="lg"
        data-empty={!busy && filtered.length === 0}
      >
      <div className="mcp-toolbar">
        <TextInput
          leftSection={<IconSearch size={15} />}
          placeholder="搜索名称、command、url…"
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          style={{ flex: "1 1 220px" }}
        />
        <Select
          value={scopeFilter}
          onChange={(v) => setScopeFilter((v as McpScope | "all") || "all")}
          data={[
            { value: "all", label: "全部作用域" },
            { value: "user", label: SCOPE_LABELS.user },
            { value: "local", label: SCOPE_LABELS.local },
            { value: "project", label: SCOPE_LABELS.project },
          ]}
          style={{ flex: "0 0 140px" }}
        />
        <FeatureHelp content={MCP_SCOPE_HELP} />
        <Select
          value={instanceFilter}
          onChange={(v) => setInstanceFilter(v ?? "all")}
          data={[
            { value: "all", label: "全部环境" },
            ...(state?.instances ?? []).map((i) => ({ value: i.id, label: i.label })),
          ]}
          style={{ flex: "0 0 140px" }}
        />
        <Select
          value={projectFilter}
          onChange={(v) => setProjectFilter(v ?? "all")}
          data={[
            { value: "all", label: "全部项目" },
            ...(state?.projects ?? []).map((p) => ({ value: p.path, label: p.label })),
          ]}
          style={{ flex: "0 0 160px" }}
        />
        <Button variant="default" leftSection={<IconFolderPlus size={15} />} onClick={addProject}>
          添加项目
        </Button>
        <Menu position="bottom-end">
          <Menu.Target>
            <Button variant="subtle" leftSection={<IconFolderX size={15} />}>取消登记</Button>
          </Menu.Target>
          <Menu.Dropdown>
            {(state?.projects ?? []).filter((p) => !p.discovered).length === 0 ? (
              <Menu.Item disabled>无手工登记项目</Menu.Item>
            ) : (
              (state?.projects ?? [])
                .filter((p) => !p.discovered)
                .map((p) => (
                  <Menu.Item key={p.path} onClick={() => unregisterProject(p.path)}>
                    {p.label}
                    <Text size="xs" c="dimmed" span ml={6}>取消登记</Text>
                  </Menu.Item>
                ))
            )}
          </Menu.Dropdown>
        </Menu>
        <Button variant="default" onClick={() => setImportOpened(true)}>导入 JSON</Button>
        <Button leftSection={<IconPlus size={15} />} onClick={() => setDrawerState({ mode: "create" })}>
          添加 MCP
        </Button>
      </div>

        <Box className="mcp-table-scroll">
          {busy && !state ? (
            <Group justify="center" p="xl"><Loader /></Group>
          ) : (
            <Table striped highlightOnHover>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th className="mcp-name-cell">服务名称</Table.Th>
                  <Table.Th className="mcp-version-column">版本</Table.Th>
                  <Table.Th className="mcp-scope-column">使用范围</Table.Th>
                  <Table.Th className="mcp-enabled-column">加载配置</Table.Th>
                  <Table.Th className="mcp-connection-column">连接检测</Table.Th>
                  <Table.Th className="mcp-update-check-column">更新检测</Table.Th>
                  {syncTargetColumns.map((target) => (
                    <Table.Th className="mcp-target-column" key={target.targetId}>
                      {target.label}
                    </Table.Th>
                  ))}
                  <Table.Th className="mcp-actions-column">操作</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {filtered.map((s) => (
                  <ServiceRow
                    key={locatorKey(s.locator)}
                    service={s}
                    onEdit={() => setDrawerState({ mode: "edit", service: s })}
                    onCopy={() => setDrawerState({ mode: "copy", service: s })}
                    onDelete={() => prepareChangeSafe({ op: "delete", target: s.locator })}
                    onEnabledChange={(enabled) =>
                      prepareChangeSafe({
                        op: "setEnabled",
                        target: s.locator,
                        enabled,
                      })
                    }
                    connectionChecks={(connectionReport?.checks ?? []).filter(
                      (check) =>
                        check.name === s.locator.name &&
                        (instanceFilter === "all" || check.environment === instanceFilter)
                    )}
                    connectionBusy={connectionBusy}
                    connectionErrors={connectionReport?.errors ?? []}
                    updateInfo={(updateReport?.entries ?? []).find(
                      (entry) => locatorKey(entry.locator) === locatorKey(s.locator)
                    )}
                    updateBusy={
                      updateBusy || rowUpdateBusy === locatorKey(s.locator)
                    }
                    updateToggleBusy={updateToggleBusy === locatorKey(s.locator)}
                    onUpdateCheckChange={(enabled) => void setUpdateCheck(s, enabled)}
                    onUpdate={() => void updateCurrentService(s)}
                    syncTargets={syncTargetsByKey.get(locatorKey(s.locator)) ?? []}
                    syncTargetColumns={syncTargetColumns}
                    syncBusyKey={syncBusyKey}
                    onTargetSyncChange={(target, enabled) =>
                      void changeTargetSync(s, target, enabled)
                    }
                    onDetails={() => setDetailService(s)}
                  />
                ))}
                {filtered.length === 0 && (
                  <Table.Tr>
                    <Table.Td colSpan={7 + syncTargetColumns.length}>
                      <div className="mcp-empty-state">
                        <div className="extension-empty-icon"><IconPlus size={24} /></div>
                        <Text fw={650}>{query || scopeFilter !== "all" || instanceFilter !== "all" || projectFilter !== "all" ? "没有符合筛选条件的 MCP 服务" : "还没有 MCP 服务"}</Text>
                        <Text size="xs" c="dimmed">可以新建服务，也可以从现有 JSON 配置导入。</Text>
                        <Group gap="xs" mt={4}>
                          <Button size="xs" variant="default" onClick={() => setImportOpened(true)}>导入 JSON</Button>
                          <Button size="xs" leftSection={<IconPlus size={14} />} onClick={() => setDrawerState({ mode: "create" })}>添加 MCP</Button>
                        </Group>
                      </div>
                    </Table.Td>
                  </Table.Tr>
                )}
              </Table.Tbody>
            </Table>
          )}
        </Box>
      </Card>

      {state && drawerState && (
        <McpServiceDrawer
          opened={pageActive}
          mode={drawerState.mode}
          service={drawerState.mode === "create" ? undefined : drawerState.service}
          state={state}
          onClose={() => setDrawerState(null)}
          onSave={prepareChange}
          onTest={onTest}
        />
      )}

      {state && (
        <McpImportModal
          opened={pageActive && (importOpened)}
          state={state}
          onClose={() => setImportOpened(false)}
          onSave={prepareChange}
        />
      )}

      <Modal
        opened={pageActive && (detailService !== null)}
        onClose={() => setDetailService(null)}
        size="xl"
        title={detailService?.locator.name}
        centered
        classNames={{ body: "mcp-detail-modal-body" }}
      >
        {detailService && (
          <Tabs defaultValue="overview">
            <Tabs.List>
              <Tabs.Tab value="overview">概览</Tabs.Tab>
              <Tabs.Tab value="config">配置</Tabs.Tab>
            </Tabs.List>

            <Tabs.Panel value="overview" pt="md">
              <Stack gap="md">
                <Group gap={8}>
                  <Badge
                    color={SCOPE_BADGE_COLOR[detailService.locator.scope]}
                    variant="light"
                  >
                    {SCOPE_LABELS[detailService.locator.scope]}
                  </Badge>
                  <Badge variant="light">{TRANSPORT_LABELS[detailService.transport]}</Badge>
                  <Badge
                    color={EFFECTIVE_COLOR[detailService.effectiveState]}
                    variant="light"
                  >
                    {EFFECTIVE_LABEL[detailService.effectiveState]}
                  </Badge>
                </Group>

                {detailService.warnings.length > 0 && (
                  <Alert color="orange" icon={<IconAlertTriangle size={16} />} title="配置提醒">
                    {detailService.warnings.map((warning, index) => (
                      <Text size="xs" key={index}>{warning}</Text>
                    ))}
                  </Alert>
                )}

                <SimpleGrid cols={{ base: 1, sm: 2 }}>
                  <DetailCard title="使用位置">
                    <DetailField
                      label="范围"
                      value={SCOPE_LABELS[detailService.locator.scope]}
                    />
                    {detailService.locator.instanceId && (
                      <DetailField label="环境" value={detailService.locator.instanceId} />
                    )}
                    {detailService.locator.projectPath && (
                      <DetailField label="项目路径" value={detailService.locator.projectPath} mono />
                    )}
                  </DetailCard>
                  <DetailCard title="当前状态">
                    <DetailField
                      label="状态"
                      value={EFFECTIVE_LABEL[detailService.effectiveState]}
                    />
                    <DetailField
                      label="连接方式"
                      value={TRANSPORT_LABELS[detailService.transport]}
                    />
                    <Group justify="space-between" wrap="nowrap">
                      <Text size="sm">允许 Claude 加载</Text>
                      <Switch
                        size="sm"
                        checked={detailService.enabled}
                        label={detailService.enabled ? "允许加载" : "不加载"}
                        labelPosition="left"
                        onChange={(event) =>
                          prepareChangeSafe({
                            op: "setEnabled",
                            target: detailService.locator,
                            enabled: event.currentTarget.checked,
                          })
                        }
                      />
                    </Group>
                  </DetailCard>
                </SimpleGrid>

                <Group justify="flex-end" className="mcp-detail-actions">
                  <Group gap={8}>
                    <Button
                      variant="default"
                      leftSection={<IconCopy size={14} />}
                      onClick={() => {
                        setDetailService(null);
                        setDrawerState({ mode: "copy", service: detailService });
                      }}
                    >
                      复制
                    </Button>
                    <Button
                      leftSection={<IconPencil size={14} />}
                      onClick={() => {
                        setDetailService(null);
                        setDrawerState({ mode: "edit", service: detailService });
                      }}
                    >
                      编辑配置
                    </Button>
                  </Group>
                </Group>
              </Stack>
            </Tabs.Panel>

            <Tabs.Panel value="config" pt="md">
              <Stack gap="md">
                <Alert color="blue" icon={<IconInfoCircle size={16} />}>
                  敏感字段已脱敏；完整路径、命令、参数和连接配置仅在此处展示。
                </Alert>
                <ConfigBlock
                  label="完整配置（已脱敏）"
                  value={redactConfig(detailService.config, detailService.sensitivePaths)}
                />
              </Stack>
            </Tabs.Panel>
          </Tabs>
        )}
      </Modal>

      <Modal
        opened={pageActive && (previewState !== null)}
        onClose={() => (applying ? undefined : setPreviewState(null))}
        size="lg"
        title={previewState?.preview.actionLabel}
        centered
      >
        {previewState && (
          <Stack gap="sm">
            <Box>
              <Text size="sm" fw={600}>受影响来源</Text>
              {previewState.preview.affectedSources.map((src) => (
                <Text component="div" size="xs" c="dimmed" key={src.sourceId}>
                  <Badge color={SCOPE_BADGE_COLOR[src.scope]} size="xs" mr={6}>
                    {SCOPE_LABELS[src.scope]}
                  </Badge>
                  {src.sourceId} · {src.path}
                </Text>
              ))}
              {previewState.preview.affectedInstances.length > 0 && (
                <Text size="xs" c="dimmed" mt={4}>
                  受影响环境：{previewState.preview.affectedInstances.join("、")}
                </Text>
              )}
            </Box>
            {previewState.preview.userSyncNote && (
              <Alert color="blue" icon={<IconCheck size={16} />}>
                {previewState.preview.userSyncNote}
              </Alert>
            )}
            {previewState.preview.batchItems.length > 0 ? (
              <Stack gap="sm">
                <Text size="sm" fw={600}>各服务变更（按来源分组，已脱敏）</Text>
                {(() => {
                  const groups = new Map<string, typeof previewState.preview.batchItems>();
                  for (const it of previewState.preview.batchItems) {
                    const arr = groups.get(it.sourceId) ?? [];
                    arr.push(it);
                    groups.set(it.sourceId, arr);
                  }
                  return Array.from(groups.entries()).map(([sid, items], gi) => (
                    <Box key={gi}>
                      <Text size="xs" c="dimmed" fw={600} mb={4}>{sid}</Text>
                      <Stack gap={6}>
                        {items.map((it, i) => (
                          <Box key={i}>
                            <Text component="div" size="xs" fw={600} mb={2}>
                              <Badge color={SCOPE_BADGE_COLOR[it.scope]} size="xs" mr={6}>
                                {SCOPE_LABELS[it.scope]}
                              </Badge>
                              {it.name}
                            </Text>
                            {it.redactedBefore !== undefined && (
                              <ConfigBlock label="变更前" value={it.redactedBefore} />
                            )}
                            {it.redactedAfter !== undefined && (
                              <ConfigBlock label="变更后" value={it.redactedAfter} />
                            )}
                          </Box>
                        ))}
                      </Stack>
                    </Box>
                  ));
                })()}
              </Stack>
            ) : (
              <>
                {previewState.preview.redactedBefore !== undefined && (
                  <ConfigBlock label="变更前（已脱敏）" value={previewState.preview.redactedBefore} />
                )}
                {previewState.preview.redactedAfter !== undefined && (
                  <ConfigBlock label="变更后（已脱敏）" value={previewState.preview.redactedAfter} />
                )}
              </>
            )}
            {previewState.preview.warnings.length > 0 && (
              <Alert color="orange" icon={<IconAlertTriangle size={16} />}>
                {previewState.preview.warnings.map((w, i) => (
                  <Text size="xs" key={i}>{w}</Text>
                ))}
              </Alert>
            )}
            <Group justify="flex-end">
              <Button variant="subtle" onClick={() => setPreviewState(null)} disabled={applying}>
                取消
              </Button>
              <Button loading={applying} onClick={confirmChange}>确认执行</Button>
            </Group>
          </Stack>
        )}
      </Modal>

      <Modal
        opened={pageActive && (syncPreview !== null)}
        onClose={() => {
          if (syncApplying) return;
          setSyncPreview(null);
        }}
        size="lg"
        title={syncPreview?.actionLabel}
        centered
      >
        {syncPreview && (
          <Stack gap="sm">
            <Alert color="blue" icon={<IconCheck size={16} />}>
              {syncPreview.preservedFieldsNote}
            </Alert>
            <Box>
              <Text size="sm" fw={600}>目标配置</Text>
              <Text size="xs" c="dimmed">{syncPreview.targetPath}</Text>
            </Box>
            {syncPreview.redactedBefore !== undefined ? (
              <ConfigBlock
                label={`${syncTargetDisplayLabel(
                  syncPreview.targetId,
                  syncPreview.targetLabel
                )} 当前配置（已脱敏）`}
                value={syncPreview.redactedBefore}
              />
            ) : (
              <Text size="sm" c="dimmed">
                {syncTargetDisplayLabel(
                  syncPreview.targetId,
                  syncPreview.targetLabel
                )} 中尚无同名 MCP。
              </Text>
            )}
            <ConfigBlock
              label="同步后配置（已脱敏）"
              value={syncPreview.redactedAfter}
            />
            {syncPreview.warnings.length > 0 && (
              <Alert color="orange" icon={<IconAlertTriangle size={16} />}>
                {syncPreview.warnings.map((warning, index) => (
                  <Text size="xs" key={index}>{warning}</Text>
                ))}
              </Alert>
            )}
            <Text size="xs" c="dimmed">{syncPreview.restartHint}</Text>
            <Group justify="flex-end">
              <Button
                variant="subtle"
                onClick={() => {
                  setSyncPreview(null);
                }}
                disabled={syncApplying}
              >
                取消
              </Button>
              <Button loading={syncApplying} onClick={confirmTargetSync}>
                确认同步
              </Button>
            </Group>
          </Stack>
        )}
      </Modal>
    </div>
  );
}

function ServiceRow({
  service,
  onEdit,
  onCopy,
  onDelete,
  onEnabledChange,
  connectionChecks,
  connectionBusy,
  connectionErrors,
  updateInfo,
  updateBusy,
  updateToggleBusy,
  onUpdateCheckChange,
  onUpdate,
  syncTargets,
  syncTargetColumns,
  syncBusyKey,
  onTargetSyncChange,
  onDetails,
}: {
  service: McpService;
  onEdit: () => void;
  onCopy: () => void;
  onDelete: () => void;
  onEnabledChange: (enabled: boolean) => void;
  connectionChecks: McpConnectionCheck[];
  connectionBusy: boolean;
  connectionErrors: string[];
  updateInfo?: McpUpdateInfo;
  updateBusy: boolean;
  updateToggleBusy: boolean;
  onUpdateCheckChange: (enabled: boolean) => void;
  onUpdate: () => void;
  syncTargets: McpSyncTargetInfo[];
  syncTargetColumns: SyncTargetColumn[];
  syncBusyKey: string;
  onTargetSyncChange: (target: McpSyncTargetInfo, enabled: boolean) => void;
  onDetails: () => void;
}) {
  const loc = service.locator;
  const context = serviceContextLabel(service);
  return (
    <Table.Tr className="mcp-service-row" onDoubleClick={onDetails}>
      <Table.Td className="mcp-name-cell">
        <Group gap={7} wrap="nowrap">
          <Text fw={600}>{loc.name}</Text>
          {service.warnings.length > 0 && (
            <Tooltip label={service.warnings.join("；")} multiline>
              <Badge color="orange" variant="light" size="xs">
                {service.warnings.length} 个提醒
              </Badge>
            </Tooltip>
          )}
        </Group>
      </Table.Td>
      <Table.Td className="mcp-version-column">
        <VersionCell info={updateInfo} busy={updateBusy && updateInfo?.checkEnabled === true} />
      </Table.Td>
      <Table.Td className="mcp-scope-column">
        <Stack gap={2}>
          <Badge
            color={SCOPE_BADGE_COLOR[loc.scope]}
            variant="light"
            style={{ alignSelf: "flex-start" }}
          >
            {SCOPE_LABELS[loc.scope]}
          </Badge>
          {context && <Text size="xs" c="dimmed">{context}</Text>}
        </Stack>
      </Table.Td>
      <Table.Td className="mcp-enabled-column">
        <Tooltip
          label={
            service.enabled
              ? "新的 Claude Code 会话会加载这项配置。关闭只会保留定义并停止后续会话加载，不会关闭 MCP 服务器；已经运行的会话可能要重启后才生效。"
              : "新的 Claude Code 会话不会加载这项配置，但 MCP 服务器本身及其他客户端可能仍在运行。点击可恢复加载。"
          }
          multiline
          maw={300}
        >
          <Switch
            size="sm"
            checked={service.enabled}
            aria-label={service.enabled ? "停止加载 MCP 配置" : "允许加载 MCP 配置"}
            onChange={(event) => onEnabledChange(event.currentTarget.checked)}
          />
        </Tooltip>
      </Table.Td>
      <Table.Td className="mcp-connection-column">
        <McpConnectionBadge
          enabled={service.enabled}
          supported={loc.scope === "user"}
          checks={connectionChecks}
          busy={connectionBusy}
          errors={connectionErrors}
        />
      </Table.Td>
      <Table.Td className="mcp-update-check-column">
        <Tooltip
          label={updateInfo?.reason ?? "正在识别更新来源"}
          multiline
          maw={300}
        >
          <span className="mcp-update-switch-wrap">
            <Switch
              size="sm"
              checked={updateInfo?.checkEnabled ?? false}
              disabled={!updateInfo?.supported || updateBusy || updateToggleBusy}
              aria-label={
                updateInfo?.supported
                  ? updateInfo.checkEnabled
                    ? "关闭该 MCP 的更新检测"
                    : "开启该 MCP 的更新检测"
                  : "该 MCP 不支持远程更新检测"
              }
              onChange={(event) => onUpdateCheckChange(event.currentTarget.checked)}
            />
          </span>
        </Tooltip>
      </Table.Td>
      {syncTargetColumns.map((column) => {
        const target = syncTargets.find((item) => item.targetId === column.targetId);
        const busy = target
          ? syncBusyKey === `${locatorKey(service.locator)}::${target.targetId}`
          : false;
        return (
          <Table.Td className="mcp-target-column" key={column.targetId}>
            <TargetSyncSwitch
              target={target}
              busy={busy}
              serviceEnabled={service.enabled}
              onChange={(enabled) => target && onTargetSyncChange(target, enabled)}
            />
          </Table.Td>
        );
      })}
      <Table.Td className="mcp-actions-column">
        <Group gap={2} wrap="nowrap">
          <Tooltip
            label={
              !updateInfo?.supported
                ? updateInfo?.reason ?? "无法识别更新来源"
                : !updateInfo.checkEnabled
                  ? "请先开启更新检测"
                  : updateInfo.updateAvailable
                    ? `更新到 v${updateInfo.latestVersion}`
                    : updateInfo.latestVersion
                      ? "当前已是最新版本"
                      : "点击顶部“检测”获取最新版本"
            }
          >
            <span>
              <Button
                variant="subtle"
                size="compact-sm"
                leftSection={<IconDownload size={14} />}
                disabled={
                  updateBusy
                  || !updateInfo?.supported
                  || !updateInfo.checkEnabled
                }
                loading={updateBusy}
                onClick={onUpdate}
              >
                更新
              </Button>
            </span>
          </Tooltip>
          <Button
            variant="subtle"
            size="compact-sm"
            leftSection={<IconEye size={14} />}
            onClick={onDetails}
          >
            详情
          </Button>
          <Button
            variant="subtle"
            size="compact-sm"
            leftSection={<IconPencil size={14} />}
            onClick={onEdit}
          >
            编辑
          </Button>
          <Button
            variant="subtle"
            size="compact-sm"
            leftSection={<IconCopy size={14} />}
            onClick={onCopy}
          >
            复制
          </Button>
          <Button
            color="red"
            variant="subtle"
            size="compact-sm"
            leftSection={<IconTrash size={14} />}
            onClick={onDelete}
          >
            删除
          </Button>
        </Group>
      </Table.Td>
    </Table.Tr>
  );
}

function VersionCell({ info, busy }: { info?: McpUpdateInfo; busy: boolean }) {
  if (!info) return <Text size="xs" c="dimmed">—</Text>;
  if (!info.supported) {
    return (
      <Tooltip label={info.reason} multiline maw={280}>
        <Text size="xs" c="dimmed" className="mcp-version-unsupported">—</Text>
      </Tooltip>
    );
  }
  return (
    <Stack gap={4} className="mcp-version-stack">
      <Group gap={6} wrap="nowrap">
        <Text size="sm" fw={600} className="mcp-version-current">
          {info.currentVersion ? `v${info.currentVersion}` : "自动"}
        </Text>
        {busy && <Loader size={12} />}
      </Group>
      {info.updateAvailable && info.latestVersion ? (
        <Badge color="red" variant="filled" size="xs" className="mcp-latest-version-badge">
          最新 v{info.latestVersion}
        </Badge>
      ) : !busy && !info.currentVersion && info.latestVersion ? (
        <Text size="xs" c="dimmed">最新 v{info.latestVersion}</Text>
      ) : null}
    </Stack>
  );
}

function TargetSyncSwitch({
  target,
  busy,
  serviceEnabled,
  onChange,
}: {
  target?: McpSyncTargetInfo;
  busy: boolean;
  serviceEnabled: boolean;
  onChange: (enabled: boolean) => void;
}) {
  if (!target) return <Text size="xs" c="dimmed">—</Text>;
  const incompatible = target.status === "incompatible";
  const targetLabel = syncTargetDisplayLabel(target.targetId, target.targetLabel);
  const hint = busy
    ? `正在处理 ${targetLabel} 配置，请稍候。`
    : !serviceEnabled
      ? `请先允许 Claude 加载这项 MCP 配置。当前 ${targetLabel} 使用选择会保留。`
      : incompatible
        ? `当前服务无法接入 ${targetLabel}：${target.detail}`
        : target.connected
          ? `${targetLabel} 会加载并自动同步这项配置。关闭后不再向新的会话提供配置，但不会关闭 MCP 服务器。`
          : `${targetLabel} 当前不加载这项配置。打开后会同步最新配置；已经运行的会话可能需要重启。`;
  return (
    <Tooltip label={hint} multiline maw={320}>
      <Switch
        size="sm"
        checked={target.connected}
        disabled={busy || !serviceEnabled || (incompatible && !target.connected)}
        aria-label={
          target.connected
            ? `停止 ${targetLabel} 加载 MCP 配置`
            : `允许 ${targetLabel} 加载并自动同步 MCP 配置`
        }
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
    </Tooltip>
  );
}

function DetailCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Card withBorder radius="md" padding="md">
      <Text fw={700} mb="sm">{title}</Text>
      <Stack gap={8}>{children}</Stack>
    </Card>
  );
}

function DetailField({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <Box>
      <Text size="xs" c="dimmed">{label}</Text>
      <Text size="sm" className={mono ? "mcp-detail-path" : undefined}>{value || "—"}</Text>
    </Box>
  );
}

function ConfigBlock({ label, value }: { label: string; value: Record<string, unknown> }) {
  return (
    <Box>
      <Text size="sm" fw={600}>{label}</Text>
      <pre className="mcp-config-preview">{JSON.stringify(value, null, 2)}</pre>
    </Box>
  );
}
