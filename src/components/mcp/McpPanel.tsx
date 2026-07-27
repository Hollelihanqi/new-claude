// MCP 服务管理页：状态摘要 + 筛选工具栏 + 服务定义列表 + 预览/确认/应用流程。
// 不负责配置文件语义；所有写操作统一走 preview → confirm → apply。
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Alert,
  Badge,
  Box,
  Button,
  Card,
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
  Title,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import {
  IconAlertTriangle,
  IconCheck,
  IconCopy,
  IconFolderPlus,
  IconFolderX,
  IconEye,
  IconInfoCircle,
  IconPencil,
  IconPlus,
  IconRefresh,
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
  McpScope,
  McpService,
  McpState,
  McpSyncPreview,
  McpSyncTargetInfo,
  McpTestResult,
} from "../../api";
import McpImportModal from "./McpImportModal";
import McpServiceDrawer from "./McpServiceDrawer";
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
  disabled: "已停用",
};
const EFFECTIVE_COLOR: Record<string, string> = {
  effective: "teal",
  "partially-shadowed": "orange",
  shadowed: "gray",
  disabled: "red",
};

export default function McpPanel() {
  const [state, setState] = useState<McpState | null>(null);
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

  const load = useCallback(() => {
    setBusy(true);
    setErr("");
    api
      .listMcpServices()
      .then(setState)
      .catch((e) => setErr(String(e)))
      .finally(() => setBusy(false));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

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
      // User/Project 无实例，不因实例筛选被隐藏；仅 Local 按实例筛
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
      setState(next);
      setPreviewState(null);
      setDrawerState(null);
      setDetailService(null);
      setImportOpened(false);
      if (next.operationWarnings.length > 0) {
        notifications.show({
          color: "orange",
          title: "主配置已保存，部分实例待同步",
          message: next.operationWarnings.join("；"),
        });
      } else {
        notifications.show({ color: "teal", message: "变更已应用", icon: <IconCheck size={16} /> });
      }
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
      setState(next);
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
      setState(next);
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
      setState(next);
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
      setState(next);
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
    <div className="mcp-page">
      <Group justify="space-between" align="flex-start">
        <div>
          <Title order={3}>MCP 服务</Title>
          <Text size="sm" c="dimmed">管理用户级、项目本地和项目共享的 MCP 配置。</Text>
        </div>
        <Button variant="light" leftSection={<IconRefresh size={15} />} loading={busy} onClick={load}>
          刷新
        </Button>
      </Group>

      {summary && (
        <SimpleGrid cols={{ base: 2, md: 4 }} className="mcp-summary-grid">
          <SummaryCard label="全部定义" value={summary.total} />
          <SummaryCard label="已启用" value={summary.enabled} color="teal" />
          <SummaryCard label="存在警告" value={summary.warnings} color="orange" />
          <SummaryCard label="被覆盖" value={summary.shadowed} color="gray" />
        </SimpleGrid>
      )}

      {err && (
        <Alert color="red" icon={<IconAlertTriangle size={16} />} title="加载失败">{err}</Alert>
      )}

      {state && state.issues.length > 0 && (
        <Alert color="orange" icon={<IconAlertTriangle size={16} />} title="配置来源问题">
          <Stack gap={2}>
            {state.issues.map((iss, i) => (
              <Text size="xs" key={i}>
                <Text span fw={600}>{iss.sourceId}</Text>
                {iss.path ? ` · ${iss.path}` : ""}：{iss.detail}
              </Text>
            ))}
          </Stack>
        </Alert>
      )}

      {state && state.operationWarnings.length > 0 && (
        <Alert color="orange" icon={<IconAlertTriangle size={16} />} title="同步警告">
          {state.operationWarnings.map((warning, index) => (
            <Text size="xs" key={index}>{warning}</Text>
          ))}
        </Alert>
      )}

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
        <Select
          value={instanceFilter}
          onChange={(v) => setInstanceFilter(v ?? "all")}
          data={[
            { value: "all", label: "全部实例" },
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

      <Card withBorder padding={0} className="mcp-table-card" radius="md">
        <Box className="mcp-table-scroll">
          {busy && !state ? (
            <Group justify="center" p="xl"><Loader /></Group>
          ) : (
            <Table striped highlightOnHover>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th className="mcp-name-cell">服务名称</Table.Th>
                  <Table.Th>使用范围</Table.Th>
                  <Table.Th>状态</Table.Th>
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
                    <Table.Td colSpan={4 + syncTargetColumns.length}>
                      <Text c="dimmed" ta="center" py="lg">没有匹配的 MCP 服务</Text>
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
          opened
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
          opened={importOpened}
          state={state}
          onClose={() => setImportOpened(false)}
          onSave={prepareChange}
        />
      )}

      <Modal
        opened={detailService !== null}
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
                      <DetailField label="实例" value={detailService.locator.instanceId} />
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
                      <Text size="sm">启用服务</Text>
                      <Switch
                        size="sm"
                        checked={detailService.enabled}
                        label={detailService.enabled ? "已启用" : "已停用"}
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
        opened={previewState !== null}
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
                <Text size="xs" c="dimmed" key={src.sourceId}>
                  <Badge color={SCOPE_BADGE_COLOR[src.scope]} size="xs" mr={6}>
                    {SCOPE_LABELS[src.scope]}
                  </Badge>
                  {src.sourceId} · {src.path}
                </Text>
              ))}
              {previewState.preview.affectedInstances.length > 0 && (
                <Text size="xs" c="dimmed" mt={4}>
                  受影响实例：{previewState.preview.affectedInstances.join("、")}
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
                            <Text size="xs" fw={600} mb={2}>
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
        opened={syncPreview !== null}
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
      <Table.Td>
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
      <Table.Td>
        <Tooltip
          label={
            service.enabled
              ? "MCP 已开启，可以被已启用的目标端调用。点击关闭后只停用调用，不删除配置。"
              : "MCP 已关闭，所有目标端都不能调用。点击可重新开启。"
          }
          multiline
          maw={300}
        >
          <Switch
            size="sm"
            checked={service.enabled}
            aria-label={service.enabled ? "关闭 MCP 服务" : "开启 MCP 服务"}
            onChange={(event) => onEnabledChange(event.currentTarget.checked)}
          />
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
      ? `请先开启 MCP 状态。当前 ${targetLabel} 使用选择会保留，但该 MCP 不能被调用。`
      : incompatible
        ? `当前服务无法接入 ${targetLabel}：${target.detail}`
        : target.connected
          ? `${targetLabel} 使用开关已开启。关闭后将停止调用和自动同步，但保留 MCP 配置。`
          : `${targetLabel} 使用开关已关闭。打开后会同步最新配置并允许调用。`;
  return (
    <Tooltip label={hint} multiline maw={320}>
      <Switch
        size="sm"
        checked={target.connected}
        disabled={busy || !serviceEnabled || (incompatible && !target.connected)}
        aria-label={
          target.connected
            ? `关闭 ${targetLabel} 使用`
            : `开启 ${targetLabel} 使用并自动同步`
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

function SummaryCard({ label, value, color }: { label: string; value: number; color?: string }) {
  return (
    <Card withBorder padding="md" radius="md">
      <Text size="xs" c="dimmed">{label}</Text>
      <Text fw={700} size="xl" c={color}>{value}</Text>
    </Card>
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
