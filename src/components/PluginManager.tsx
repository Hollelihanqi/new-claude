import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActionIcon, Alert, Badge, Button, Checkbox, Drawer, Group, Loader, Menu, Modal, Select, Stack, Switch, Table, Tabs, Text, TextInput, ThemeIcon } from "@mantine/core";
import { open } from "@tauri-apps/plugin-dialog";
import { IconAlertTriangle, IconArrowsExchange, IconCheck, IconCircleCheckFilled, IconClock, IconDots, IconFolder, IconInfoCircle, IconLoader2, IconPackage, IconPlus, IconRefresh, IconTrash, IconWorld } from "@tabler/icons-react";
import { api, type PluginRow } from "../api";
import { usePageActivation } from "./PersistentPage";
import { createPluginTask, executePluginTasks, planPluginCopy, pluginIdentity, pluginStateLabel, validatePluginSource, PluginCommandQueue, type PluginTask } from "./pluginTasks";
import "./pluginManager.css";

const ENVIRONMENT_KEY = "pathmux-plugin-environment";
type Panel = "install" | "details" | "copy" | "operation" | null;
type Confirmation = { action: "update" | "uninstall"; row: PluginRow; env: string };
type Operation = { id: number; title: string; tasks: PluginTask[] };

function rememberedEnvironment() {
  try { return window.localStorage.getItem(ENVIRONMENT_KEY) ?? ""; } catch { return ""; }
}

export default function PluginManager() {
  const [rows, setRows] = useState<PluginRow[]>([]);
  const [envs, setEnvs] = useState<string[]>([]);
  const [target, setTarget] = useState(rememberedEnvironment);
  const [loading, setLoading] = useState(true);
  const [readError, setReadError] = useState("");
  const [formError, setFormError] = useState("");
  const [panel, setPanel] = useState<Panel>(null);
  const [running, setRunning] = useState(false);
  const [operation, setOperation] = useState<Operation | null>(null);
  const [records, setRecords] = useState<Operation[]>([]);
  const [centerId, setCenterId] = useState<number | null>(null);
  const [viewedId, setViewedId] = useState<number | null>(null);
  const [pendingKeys, setPendingKeys] = useState<string[]>([]);
  const commandQueue = useRef(new PluginCommandQueue());
  const operationId = useRef(0);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [detailName, setDetailName] = useState("");
  const [installEnv, setInstallEnv] = useState("");
  const [installMode, setInstallMode] = useState("remote");
  const [source, setSource] = useState("");
  const [localPackage, setLocalPackage] = useState("");
  const [enableAfterInstall, setEnableAfterInstall] = useState(true);
  const [copySource, setCopySource] = useState("");
  const [copyPlugin, setCopyPlugin] = useState("");
  const [copyTargets, setCopyTargets] = useState<string[]>([]);
  const mutating = useRef(false);
  const generation = useRef(0);
  const mounted = useRef(true);

  const acceptTargets = useCallback((names: string[]) => {
    const editable = [...new Set(names)].filter((name) => name && name !== "__main__" && name !== "__all__");
    setEnvs(editable);
    setTarget((current) => editable.includes(current) ? current : editable[0] ?? "");
  }, []);

  const load = useCallback(async () => {
    if (mutating.current) return;
    const request = ++generation.current;
    setLoading(true);
    try {
      const [nextRows, nextEnvs] = await Promise.all([api.pluginsOverview(), api.pluginTargets()]);
      if (mounted.current && request === generation.current) {
        setRows(nextRows); acceptTargets(nextEnvs); setReadError("");
      }
    } catch (error) {
      if (mounted.current && request === generation.current) setReadError(String(error));
    } finally {
      if (mounted.current && request === generation.current) setLoading(false);
    }
  }, [acceptTargets]);

  useEffect(() => {
    mounted.current = true;
    void load();
    return () => { mounted.current = false; ++generation.current; };
  }, [load]);
  usePageActivation(() => void load());
  useEffect(() => {
    if (target && envs.includes(target)) {
      try { window.localStorage.setItem(ENVIRONMENT_KEY, target); } catch { /* Selection still works without persistent storage. */ }
    }
  }, [target, envs]);

  const canChange = envs.includes(target) && !readError;
  const installed = rows.filter((row) => row.envs.some((entry) => entry.env === target && entry.installed));
  const available = rows.filter((row) => !row.envs.some((entry) => entry.env === target && entry.installed));
  const detail = rows.find((row) => row.name === detailName);
  const sourceRow = rows.find((row) => row.name === copyPlugin);
  const copyRows = rows.filter((row) => row.envs.some((entry) => entry.env === copySource && entry.installed));
  const otherEnvs = envs.filter((env) => env !== copySource);
  const copyPlan = useMemo(() => sourceRow ? planPluginCopy(sourceRow, copySource, copyTargets) : [], [sourceRow, copySource, copyTargets]);

  const begin = async (tasks: PluginTask[], title: string, feedback: "drawer" | "inline" | "center" = "drawer", retryId?: number): Promise<void> => {
    if (tasks.length === 0) return;
    if (tasks.some((task) => !envs.includes(task.env))) { setFormError("目标环境已不存在，请刷新后重新选择"); return; }
    const id = retryId ?? ++operationId.current;
    const record = (next: PluginTask[], active = true) => {
      if (!mounted.current) return;
      const nextOperation = { id, title, tasks: next };
      setRecords((current) => [...current.filter((entry) => entry.id !== id), nextOperation].sort((a, b) => a.id - b.id));
      if (active) setOperation(nextOperation);
    };
    const keys = tasks.map((task) => `${task.env}:${task.plugin}`);
    return commandQueue.current.enqueue(keys, async () => {
      if (!mounted.current) return;
      mutating.current = true;
      ++generation.current;
      setRunning(true); setLoading(false); setFormError(""); setReadError("");
      record(tasks);
      try {
        const result = await executePluginTasks(tasks, api, (next) => {
          record(next);
        });
        if (!mounted.current) return;
        if (result.rows) setRows(result.rows);
        setReadError(result.readError);
        record(result.tasks);
        try {
          const nextEnvs = await api.pluginTargets();
          if (mounted.current) acceptTargets(nextEnvs);
        } catch (error) {
          if (mounted.current) setReadError(`操作结果已保留，环境列表刷新失败：${String(error)}`);
        }
      } finally {
        mutating.current = false;
        if (mounted.current) setRunning(false);
      }
    }, (keys) => {
      if (!mounted.current) return;
      setPendingKeys(keys);
      if (keys.some((key) => tasks.some((task) => key === `${task.env}:${task.plugin}`))) {
        record(tasks, false);
        if (feedback === "center") { setCenterId(id); setConfirmation(null); setPanel(null); }
        if (feedback === "drawer") { setViewedId(id); setPanel("operation"); }
      }
    }).catch((error) => {
      if (mounted.current) setFormError(error instanceof Error ? error.message : String(error));
    });
  };

  const openInstaller = (row?: PluginRow, env = target) => {
    if (!canChange || !envs.includes(env)) return;
    setInstallEnv(env); setInstallMode("remote"); setSource(row?.name ?? ""); setLocalPackage("");
    setEnableAfterInstall(true); setFormError(""); setPanel("install");
  };

  const install = () => {
    const input = (installMode === "local" ? localPackage : source).trim();
    const error = installMode === "remote" && rows.some((row) => row.name === input)
      ? null : validatePluginSource(installMode, input);
    if (error) { setFormError(error); return; }
    const packageInput = installMode === "local" || /^https:\/\//i.test(input);
    void begin([createPluginTask(installEnv, packageInput ? "待识别的插件包" : input,
      [packageInput ? { source: input } : { action: "install" }, { action: enableAfterInstall ? "enable" : "disable" }],
      { installed: true, enabled: enableAfterInstall })], `安装到 ${installEnv}`);
  };

  const pickPackage = async () => {
    try {
      const selected = await open({ title: "选择 Claude Code 插件包", multiple: false, filters: [{ name: "Claude Code 插件包", extensions: ["zip"] }] });
      if (typeof selected === "string") { setLocalPackage(selected); setFormError(""); }
    } catch (error) { setFormError(String(error)); }
  };

  const showCopy = (row?: PluginRow) => {
    if (!canChange) return;
    const sourceEnv = row?.envs.some((entry) => entry.env === target && entry.installed) ? target
      : rows.some((item) => item.envs.some((entry) => entry.env === target && entry.installed)) ? target
        : envs.find((env) => rows.some((item) => item.envs.some((entry) => entry.env === env && entry.installed))) ?? target;
    setCopySource(sourceEnv);
    setCopyPlugin(row?.name ?? rows.find((item) => item.envs.some((entry) => entry.env === sourceEnv && entry.installed))?.name ?? "");
    setCopyTargets(sourceEnv === target ? [] : [target]); setFormError(""); setPanel("copy");
  };

  const toggle = (row: PluginRow) => {
    if (!canChange) return;
    const enabled = row.envs.find((entry) => entry.env === target)?.value === true;
    void begin([createPluginTask(target, row.name, [{ action: enabled ? "disable" : "enable" }], { installed: true, enabled: !enabled })],
      `在 ${target} ${enabled ? "停用" : "启用"} ${pluginIdentity(row.name).name}`, "inline");
  };

  const actionPending = (row: PluginRow, actions: string[]) => records.some((record) => record.tasks.some((task) =>
    task.env === target && task.plugin === row.name && (task.status === "waiting" || task.status === "running")
    && task.steps[0] && "action" in task.steps[0] && actions.includes(task.steps[0].action)));

  const update = (row: PluginRow) => {
    if (!canChange) return;
    const enabled = row.envs.find((entry) => entry.env === target)?.value === true;
    void begin([createPluginTask(target, row.name, [{ action: "update" }, { action: enabled ? "enable" : "disable" }],
      { installed: true, enabled })], `在 ${target} 更新 ${pluginIdentity(row.name).name}`, "inline");
  };

  const confirm = () => {
    if (!confirmation || !canChange) return;
    const { row, env, action } = confirmation;
    const enabled = row.envs.find((entry) => entry.env === env)?.value === true;
    void begin([createPluginTask(env, row.name,
      action === "uninstall" ? [{ action }] : [{ action }, { action: enabled ? "enable" : "disable" }],
      action === "uninstall" ? { installed: false } : { installed: true, enabled })],
    `${action === "uninstall" ? "从" : "在"} ${env} ${action === "uninstall" ? "卸载" : "更新"} ${pluginIdentity(row.name).name}`, "center");
  };

  const renderRows = (items: PluginRow[], isInstalled: boolean) => <div className="plugin-environment-table-scroll" role="region" aria-label={isInstalled ? "当前环境已安装插件" : "当前环境可添加插件"} tabIndex={0}>
    <Table className="plugin-environment-table" verticalSpacing="md" highlightOnHover>
      <Table.Thead><Table.Tr>
        <Table.Th>插件</Table.Th><Table.Th>版本</Table.Th><Table.Th>来源</Table.Th>
        <Table.Th className="plugin-description-column">说明</Table.Th><Table.Th>状态</Table.Th><Table.Th>操作</Table.Th>
      </Table.Tr></Table.Thead>
      <Table.Tbody>{items.map((row) => {
        const identity = pluginIdentity(row.name);
        const state = row.envs.find((entry) => entry.env === target);
        const pending = pendingKeys.includes(`${target}:${row.name}`);
        const toggling = actionPending(row, ["enable", "disable"]);
        const updating = actionPending(row, ["update"]);
        return <Table.Tr key={row.name} aria-busy={!!pending}>
          <Table.Td><Group gap={12} wrap="nowrap"><ThemeIcon variant="light" radius="md" size={40}><IconPackage size={22} /></ThemeIcon>
            <button className="plugin-name-button" onClick={() => { setDetailName(row.name); setPanel("details"); }}>{identity.name}</button>
          </Group></Table.Td>
          <Table.Td><Text size="sm" c="dimmed">{(isInstalled ? state?.version : row.defaultVersion) ? `v${isInstalled ? state?.version : row.defaultVersion}` : "未记录"}</Text></Table.Td>
          <Table.Td><Text size="sm" c="dimmed" className="plugin-source-name">{identity.source}</Text></Table.Td>
          <Table.Td className="plugin-description-column"><Text size="sm" c="dimmed">{row.defaultInstalled ? "默认 Claude 中已发现" : "已在本机发现的 Claude Code 插件"}</Text></Table.Td>
          <Table.Td>{isInstalled ? <Group gap={10} wrap="nowrap" className="plugin-status-control">
            <Text size="sm" c={state?.value ? "teal.6" : "dimmed"} fw={600}>{state?.value ? "已启用" : "已停用"}</Text>
            <Switch size="md" color="#00a675" withThumbIndicator={false} checked={state?.value === true} disabled={!canChange || toggling}
              aria-label={`${row.name} 在环境 ${target} ${state?.value ? "停用" : "启用"}`} aria-busy={toggling}
              thumbIcon={toggling ? <IconLoader2 size={13} className="plugin-switch-spinner" /> : undefined}
              onChange={() => toggle(row)} />
          </Group> : <Text size="sm" c="dimmed">未安装到 {target}</Text>}</Table.Td>
          <Table.Td><Group gap={8} wrap="nowrap">
            {isInstalled ? <Button size="sm" variant="default" leftSection={<IconRefresh size={15} className={updating ? "plugin-refresh-spinner" : undefined} />} disabled={!canChange || updating} aria-busy={updating}
              aria-label={`更新 ${row.name}`} onClick={() => update(row)}>更新</Button>
              : <Button size="sm" variant="light" disabled={!canChange || pending} onClick={() => openInstaller(row)}>安装到 {target}</Button>}
            <Menu position="bottom-end" withinPortal><Menu.Target><ActionIcon size={34} variant="subtle" color="gray" disabled={!canChange} aria-label={`${identity.name} 更多操作`}><IconDots size={19} /></ActionIcon></Menu.Target>
              <Menu.Dropdown>
                <Menu.Item leftSection={<IconInfoCircle size={15} />} onClick={() => { setDetailName(row.name); setPanel("details"); }}>插件详情</Menu.Item>
                {isInstalled && <Menu.Item leftSection={<IconArrowsExchange size={15} />} onClick={() => showCopy(row)}>复制配置到其他环境</Menu.Item>}
                {isInstalled && <Menu.Item color="red" leftSection={<IconTrash size={15} />} onClick={() => setConfirmation({ action: "uninstall", row, env: target })}>从 {target} 卸载</Menu.Item>}
              </Menu.Dropdown>
            </Menu>
          </Group></Table.Td>
        </Table.Tr>;
      })}</Table.Tbody>
    </Table>
  </div>;

  const renderResult = (operation: Operation, close: () => void, centered = false) => {
    const running = operation.tasks.some((task) => task.status === "waiting" || task.status === "running");
    const unresolved = operation.tasks.filter((task) => task.status === "failed" || task.status === "unverified");
    return <Stack gap="lg" className={centered ? "plugin-center-result" : "plugin-flow-layout"}>
        <div className="plugin-operation-heading">{running ? <Loader size={36} /> : unresolved.length ? <ThemeIcon size={46} radius="xl" color="orange" variant="light"><IconAlertTriangle size={26} /></ThemeIcon> : <ThemeIcon size={46} radius="xl" color="teal" variant="light"><IconCheck size={27} /></ThemeIcon>}
          <Text fw={750} size="lg">{operation.title}</Text><Text size="sm" c="dimmed">{running ? "按目标环境逐项执行，以实际返回结果为准。" : `${operation.tasks.filter((task) => task.status === "succeeded").length}/${operation.tasks.length} 个环境已完成并核对`}</Text>
        </div>
        <Stack gap="sm">{operation.tasks.map((task) => {
          const actual = rows.find((row) => row.name === task.plugin)?.envs.find((entry) => entry.env === task.env);
          return <div className={`plugin-task-result ${task.status}`} key={task.env}>
            <Group justify="space-between" wrap="nowrap"><Text fw={700}>{task.env}</Text>
              <Group gap={6}>{task.status === "running" ? <Loader size={15} /> : task.status === "succeeded" ? <IconCircleCheckFilled size={18} color="#00a675" /> : task.status === "waiting" ? <IconClock size={18} /> : <IconAlertTriangle size={18} />}
                <Text size="sm">{({ waiting: "等待执行", running: "执行中", succeeded: task.steps.length ? "已完成" : "无需修改", failed: "失败", unverified: "待核对" })[task.status]}</Text></Group>
            </Group>
            <Text size="sm" fw={600} mt={8}>{pluginIdentity(task.plugin).name}</Text><Text size="sm" c="dimmed" mt={5} className="plugin-task-detail">{task.detail}</Text>
            {!running && !readError && <Text size="sm" mt={8}>当前：{actual?.installed ? actual.value ? "已启用" : "已停用" : "未安装"}{actual?.installed && actual.version ? ` · v${actual.version}` : ""}</Text>}
            {task.reloadHint && <Text size="xs" c="dimmed" mt={8} className="plugin-task-detail">{task.reloadHint}</Text>}
            {[...new Set(task.warnings)].map((warning) => <Alert color="orange" mt="sm" key={warning}>{warning}</Alert>)}
          </div>;
        })}</Stack>
        {readError && <Alert color="orange">执行记录已保留。{readError}</Alert>}
        <Text size="sm" c="dimmed">仅处理上述目标环境。已完成的环境和步骤不会因重试而重复执行。</Text>
        <div className="plugin-flow-actions">
          <Button variant="default" onClick={close}>{running ? "关闭后在后台继续" : "完成"}</Button>
          {!running && unresolved.length > 0 && <Button onClick={() => void begin(operation.tasks, operation.title, centered ? "center" : "drawer", operation.id)}>{unresolved.some((task) => task.status === "failed") ? `重试 ${unresolved.filter((task) => task.status === "failed").length} 个失败环境` : "重新核对实际状态"}</Button>}
          {!running && unresolved.length === 0 && operation.tasks.length === 1 && !operation.tasks[0].expected.installed && <Button onClick={() => openInstaller(rows.find((row) => row.name === operation.tasks[0].plugin) ?? { name: operation.tasks[0].plugin, envs: [] }, operation.tasks[0].env)}>重新安装</Button>}
        </div>
      </Stack>;
  };
  const centerOperation = records.find((entry) => entry.id === centerId);
  const viewedOperation = records.find((entry) => entry.id === viewedId) ?? operation;
  const failures = records.filter((entry) => !entry.tasks.some((task) => task.status === "waiting" || task.status === "running")
    && entry.tasks.some((task) => task.status === "failed" || task.status === "unverified" || task.warnings.length > 0));

  return <Stack gap="md" className="extension-manager plugin-environment-manager">
    <div className="plugin-environment-toolbar">
      <div className="plugin-env-tabs" role="tablist" aria-label="选择插件环境">
        {envs.map((env) => <button type="button" role="tab" key={env} className="plugin-env-tab" aria-selected={target === env}
          onClick={() => { setTarget(env); setFormError(""); }}>{env}</button>)}
        {envs.length === 0 && <Text size="sm" c="dimmed">{loading ? "正在读取环境…" : "暂无受管理环境"}</Text>}
      </div>
      <Button variant="default" leftSection={<IconArrowsExchange size={17} />} disabled={!canChange || envs.length < 2} onClick={() => showCopy()}>跨环境管理</Button>
    </div>
    <div className="plugin-scope-banner"><ThemeIcon size={40} radius="md" variant="light"><IconPackage size={22} /></ThemeIcon>
      <Text fw={750}>当前管理：{target || "未选择环境"}</Text>
      {(readError || failures.length > 0) && <><span className="plugin-scope-divider" />
        <div className="plugin-scope-feedback" role="status" aria-live="polite">
          <span className="plugin-feedback-icon"><IconAlertTriangle size={18} /></span>
          <Text size="sm" truncate title={readError || undefined}>{readError ? "读取失败，请重新刷新" : `${failures.length} 项操作需要处理`}</Text>
          {(failures.length > 0 || operation) && <Button className="plugin-result-entry" variant="subtle" size="compact-sm" onClick={() => { setViewedId((failures[failures.length - 1] ?? operation)?.id ?? null); setPanel("operation"); }}>查看结果</Button>}
        </div>
      </>}
    </div>
    <section className="plugin-environment-content" aria-label="当前环境插件">
      <div className="plugin-environment-heading">
        <Group gap={10}><Text fw={750}>已安装</Text><Badge variant="light">{installed.length}</Badge></Group>
        <Group gap={12}>
          <ActionIcon size={36} variant="subtle" aria-label="刷新插件状态" aria-busy={loading} aria-disabled={loading || running} onClick={() => { if (!loading && !running) void load(); }}><IconRefresh size={18} className={loading ? "plugin-refresh-spinner" : undefined} /></ActionIcon>
          <Button leftSection={<IconPlus size={17} />} disabled={!canChange} onClick={() => openInstaller()}>安装插件</Button>
        </Group>
      </div>
      {loading && envs.length === 0 ? <div className="extension-empty-state" role="status"><Loader size="sm" /><Text size="sm" c="dimmed">正在读取插件状态…</Text></div>
        : !loading && !readError && !target ? <div className="extension-empty-state"><IconPackage size={34} /><Text fw={650}>还没有受管理环境</Text><Text size="sm" c="dimmed">先创建一个环境。</Text></div>
        : target && <>
          {installed.length > 0 ? renderRows(installed, true) : <div className="plugin-installed-empty"><Text size="sm" c="dimmed">{target} 尚未安装插件</Text><Button size="compact-sm" variant="subtle" onClick={() => showCopy()} disabled={!canChange || envs.length < 2}>从其他环境复制</Button></div>}
          {available.length > 0 && <><Group gap={10} className="plugin-section-title"><Text fw={750}>可添加</Text><Badge variant="light">{available.length}</Badge></Group>{renderRows(available, false)}</>}
        </>}
    </section>

    <Drawer opened={panel !== null} onClose={() => setPanel(null)} position="right" size={480}
      closeButtonProps={{ "aria-label": "关闭插件面板" }}
      title={panel === "install" ? `安装到 ${installEnv}` : panel === "details" ? "插件详情" : panel === "copy" ? "复制插件配置" : running ? "正在执行" : "执行结果"}
      overlayProps={{ backgroundOpacity: 0.32, blur: 3 }}
      classNames={{ content: "plugin-flow-drawer", header: "plugin-flow-drawer-header", body: "plugin-flow-drawer-body" }}>
      {formError && <Alert color="red" mb="md">{formError}</Alert>}
      {panel === "install" && <Stack gap="lg" className="plugin-flow-layout">
        <Alert variant="light" icon={<IconInfoCircle size={18} />}>仅安装到 {installEnv}，其他环境保持不变。</Alert>
        <Tabs value={installMode} onChange={(value) => { setInstallMode(value ?? "remote"); setFormError(""); }}>
          <Tabs.List grow><Tabs.Tab value="remote" leftSection={<IconWorld size={16} />}>远程地址</Tabs.Tab><Tabs.Tab value="local" leftSection={<IconFolder size={16} />}>本地插件包</Tabs.Tab></Tabs.List>
          <Tabs.Panel value="remote" pt="lg"><TextInput label="插件来源" description="支持 plugin@marketplace 或 HTTPS 插件 ZIP 地址" placeholder="plugin@marketplace" value={source} aria-label="远程插件来源" onChange={(event) => setSource(event.currentTarget.value)} />
            {rows.length > 0 && <div className="plugin-known-sources"><Text size="xs" c="dimmed">已发现的插件</Text><Group gap={6}>{rows.slice(0, 8).map((row) => <Button key={row.name} size="compact-xs" variant={source === row.name ? "light" : "subtle"} onClick={() => setSource(row.name)}>{row.name}</Button>)}</Group></div>}
          </Tabs.Panel>
          <Tabs.Panel value="local" pt="lg"><Button variant="default" leftSection={<IconFolder size={18} />} fullWidth onClick={() => void pickPackage()}>选择本地插件包</Button><Text size="sm" c="dimmed" mt="sm" className="plugin-package-path">{localPackage || "支持 .zip 格式"}</Text></Tabs.Panel>
        </Tabs>
        {source && installMode === "remote" && <div className="plugin-flow-identity"><ThemeIcon size={44} radius="md" variant="light"><IconPackage size={24} /></ThemeIcon><div><Text fw={700}>{pluginIdentity(source).name}</Text><Text size="sm" c="dimmed">{pluginIdentity(source).source}</Text></div></div>}
        <div className="plugin-detail-line"><Text size="sm" c="dimmed">目标环境</Text><Badge variant="light" tt="none">{installEnv}</Badge></div>
        <Text size="sm" c="dimmed">安装完成后读取实际版本。</Text>
        <Checkbox checked={enableAfterInstall} onChange={(event) => setEnableAfterInstall(event.currentTarget.checked)} label="安装后启用插件" description="新会话生效；已有会话的加载方式以执行结果提示为准。" />
        <div className="plugin-flow-actions"><Button variant="default" onClick={() => setPanel(null)}>取消</Button><Button disabled={!canChange} onClick={install}>安装到 {installEnv}</Button></div>
      </Stack>}
      {panel === "details" && detail && <Stack gap="lg" className="plugin-flow-layout">
        <div className="plugin-flow-identity"><ThemeIcon size={44} radius="md" variant="light"><IconPackage size={24} /></ThemeIcon><div><Text fw={750}>{pluginIdentity(detail.name).name}</Text><Text size="sm" c="dimmed">{pluginIdentity(detail.name).source}</Text></div></div>
        <div className="plugin-detail-line"><Text c="dimmed" size="sm">当前环境</Text><Text fw={650}>{target}</Text></div>
        <div className="plugin-detail-line"><Text c="dimmed" size="sm">实际状态</Text><Text>{pluginStateLabel(detail, target)}</Text></div>
        <div className="plugin-detail-line"><Text c="dimmed" size="sm">当前版本</Text><Text>{detail.envs.find((item) => item.env === target)?.version ?? "未记录"}</Text></div>
        <Text size="sm" c="dimmed">{detail.defaultInstalled ? "在默认 Claude 中已发现此插件；默认 Claude 仅供查看。" : "由本机环境发现的 Claude Code 插件。"}</Text>
        {detail.envs.some((entry) => entry.env === target && entry.installed) ? <>
          <Group justify="space-between"><Text size="sm">在 {target} 中启用</Text><Switch aria-label={`${detail.name} 详情中启用`} color="#00a675" withThumbIndicator={false} checked={detail.envs.find((item) => item.env === target)?.value === true} disabled={!canChange} onChange={() => toggle(detail)} /></Group>
          <Alert icon={<IconInfoCircle size={18} />}>新会话使用当前配置。已有会话可重新加载插件，具体提示在执行结果中显示。</Alert>
          <Button variant="default" leftSection={<IconRefresh size={16} className={actionPending(detail, ["update"]) ? "plugin-refresh-spinner" : undefined} />} onClick={() => update(detail)} disabled={!canChange || actionPending(detail, ["update"])} aria-busy={actionPending(detail, ["update"])}>检查并更新</Button>
          <Button variant="default" leftSection={<IconArrowsExchange size={16} />} onClick={() => showCopy(detail)} disabled={!canChange || envs.length < 2}>复制配置到其他环境</Button>
          <div className="plugin-flow-actions"><Button color="red" variant="subtle" leftSection={<IconTrash size={16} />} onClick={() => { setPanel(null); setConfirmation({ action: "uninstall", row: detail, env: target }); }} disabled={!canChange}>从 {target} 卸载</Button></div>
        </> : <Button onClick={() => openInstaller(detail)} disabled={!canChange}>安装到 {target}</Button>}
      </Stack>}
      {panel === "copy" && <Stack gap="lg" className="plugin-flow-layout">
        <Text size="sm" c="dimmed">选择一个来源环境，把插件的安装与启停配置一次性应用到所选环境。</Text>
        <Select label="来源环境" value={copySource} allowDeselect={false} data={envs} onChange={(value) => {
          const next = value ?? ""; setCopySource(next); setCopyTargets((current) => current.filter((env) => env !== next));
          setCopyPlugin(rows.find((row) => row.envs.some((entry) => entry.env === next && entry.installed))?.name ?? "");
        }} />
        <Select label="来源插件" placeholder="此环境尚未安装插件" value={copyPlugin || null} allowDeselect={false} searchable data={copyRows.map((row) => ({ value: row.name, label: pluginIdentity(row.name).name }))} onChange={(value) => setCopyPlugin(value ?? "")} />
        {sourceRow && <div className="plugin-detail-line"><Text size="sm" c="dimmed">来源状态</Text><Text size="sm" fw={650}>{pluginStateLabel(sourceRow, copySource)}</Text></div>}
        <Stack gap="sm"><Text fw={700}>目标环境</Text>
          <Checkbox label="选择所有其他环境" checked={otherEnvs.length > 0 && otherEnvs.every((env) => copyTargets.includes(env))} indeterminate={copyTargets.length > 0 && !otherEnvs.every((env) => copyTargets.includes(env))} disabled={!sourceRow} onChange={(event) => setCopyTargets(event.currentTarget.checked ? [...otherEnvs] : [])} />
          {otherEnvs.map((env) => <div className="plugin-copy-target" key={env}><Checkbox label={env} checked={copyTargets.includes(env)} disabled={!sourceRow} onChange={(event) => {
            const checked = event.currentTarget.checked; setCopyTargets((current) => checked ? [...current, env] : current.filter((item) => item !== env));
          }} /><Text size="sm" c="dimmed">{sourceRow ? pluginStateLabel(sourceRow, env) : "—"}</Text></div>)}
        </Stack>
        <div className="plugin-copy-preview"><Text fw={700} mb="sm">变更预览</Text>
          {copyPlan.length === 0 ? <Text size="sm" c="dimmed">选择目标环境后显示将发生的变化。</Text> : copyPlan.map((task) => <div className="plugin-copy-preview-row" key={task.env}><Text size="sm" fw={650}>{task.env}</Text><Text size="sm">{task.steps.length === 0 ? "无需修改" : `${sourceRow ? pluginStateLabel(sourceRow, task.env) : "未安装"} → ${task.expected.enabled ? "已启用" : "已安装并停用"}`}</Text></div>)}
        </div>
        <Text size="sm" c="dimmed">现有安装版本保持不变；新安装版本以官方来源为准。未选中的环境不受影响。</Text>
        <Alert icon={<IconInfoCircle size={18} />}>这是一次性复制，不会建立持续同步关系。</Alert>
        <div className="plugin-flow-actions"><Button variant="default" onClick={() => setPanel(null)}>取消</Button><Button disabled={!canChange || copyPlan.length === 0} onClick={() => void begin(copyPlan, `复制 ${pluginIdentity(copyPlugin).name} 的配置`)}>应用到 {copyPlan.length} 个环境</Button></div>
      </Stack>}
      {panel === "operation" && <Stack gap="md">
        {records.length > 1 && <Select label="执行记录" value={String(viewedOperation?.id ?? "")} data={[...records].reverse().map((entry) => ({ value: String(entry.id), label: entry.title }))} onChange={(value) => setViewedId(Number(value))} />}
        {viewedOperation && renderResult(viewedOperation, () => setPanel(null))}
      </Stack>}
    </Drawer>
    <Modal opened={confirmation !== null || centerOperation !== undefined} onClose={() => { setConfirmation(null); setCenterId(null); }} centered size="md" title={centerOperation ? centerOperation.title : confirmation?.action === "uninstall" ? `从 ${confirmation.env} 卸载插件？` : `更新 ${confirmation?.env ?? ""} 中的插件`}
      closeButtonProps={{ "aria-label": "取消当前操作" }}
      classNames={{ content: "plugin-remove-modal-content", header: "plugin-remove-modal-header", body: "plugin-remove-modal-body" }}>
      {centerOperation && renderResult(centerOperation, () => setCenterId(null), true)}
      {confirmation && <Stack gap="md">
        <div className="plugin-flow-identity"><ThemeIcon size={40} variant="light" radius="md"><IconPackage size={22} /></ThemeIcon><div><Text fw={750}>{pluginIdentity(confirmation.row.name).name}</Text><Text size="sm" c="dimmed">{pluginIdentity(confirmation.row.name).source}</Text></div></div>
        <Text size="sm">仅影响 {confirmation.env}，其他环境保持不变。</Text>
        {confirmation.action === "update" ? <><Text size="sm">当前版本：{confirmation.row.envs.find((entry) => entry.env === confirmation.env)?.version ?? "未记录"}</Text><Text size="sm">将由官方来源检查并更新可用版本，保留当前启停状态。完成后读取实际版本。</Text><Text size="sm" c="dimmed">新版本需要按执行结果中的提示重新加载或重启会话。</Text></>
          : <Text size="sm" c="dimmed">仅移除此环境中的插件。已运行的会话不会被强制退出；卸载后可重新安装。</Text>}
        <Group justify="flex-end"><Button variant="default" onClick={() => setConfirmation(null)}>取消</Button><Button color={confirmation.action === "uninstall" ? "red" : undefined} disabled={!canChange} onClick={confirm}>{confirmation.action === "uninstall" ? `从 ${confirmation.env} 卸载` : "确认更新"}</Button></Group>
      </Stack>}
    </Modal>
  </Stack>;
}
