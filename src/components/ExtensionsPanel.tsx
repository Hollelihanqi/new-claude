import { useCallback, useEffect, useRef, useState } from "react";
import { usePageActivation } from "./PersistentPage";
import { Alert, Badge, Button, Card, Group, Modal, Select, Stack, Switch, Table, Tabs, Text, TextInput, Tooltip } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { open } from "@tauri-apps/plugin-dialog";
import { IconBrain, IconPlugConnected, IconRefresh, IconRobot, IconTrash } from "@tabler/icons-react";
import { api } from "../api";
import type { PluginAction, PluginActionReport, PluginRow, ResourceItem, ResourceKind, ResourceOverview } from "../api";
import FeatureHelp from "./FeatureHelp";
import {
  AGENT_DEPENDENCY_HELP,
  AGENTS_HELP,
  AUTO_IMPORT_HELP,
  PLUGIN_RESULT_HELP,
  PLUGINS_HELP,
  RESOURCE_OVERRIDE_HELP,
  SKILLS_HELP,
} from "./featureHelpContent";

const ALL_ENVS = "__all__";
const STATUS_LABEL: Record<string, { text: string; color: string }> = {
  inherited: { text: "使用共享版本", color: "teal" },
  override: { text: "目标自己的版本", color: "orange" },
  excluded: { text: "已排除", color: "gray" },
  missing: { text: "尚未写入", color: "red" },
  local: { text: "目标独有", color: "blue" },
  unavailable: { text: "缺少依赖", color: "red" },
};

function ResourceManager({ kind }: { kind: ResourceKind }) {
  const [overview, setOverview] = useState<ResourceOverview | null>(null);
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const [deleteItem, setDeleteItem] = useState<ResourceItem | null>(null);
  const [replaceItem, setReplaceItem] = useState<ResourceItem | null>(null);
  const inFlight = useRef(false);
  const refreshQueued = useRef(false);
  const help = kind === "skills" ? SKILLS_HELP : AGENTS_HELP;
  const label = kind === "skills" ? "Skill" : "Agent";
  const lastAutoImport = overview?.lastAutoImportAt
    ? new Date(overview.lastAutoImportAt * 1000).toLocaleString()
    : "尚未检查";

  const load = useCallback(async (quiet = false) => {
    if (inFlight.current) {
      refreshQueued.current = true;
      return;
    }
    inFlight.current = true;
    if (!quiet) setBusy("load");
    try {
      const next = await api.resourceOverview(kind);
      setOverview(next);
      setTarget((current) => current && next.targets.some((item) => item.target === current)
        ? current
        : (next.targets[0]?.target ?? ""));
      setErr("");
    } catch (error) {
      setErr(String(error));
    } finally {
      inFlight.current = false;
      if (!quiet) setBusy("");
      if (refreshQueued.current) {
        refreshQueued.current = false;
        void load(true);
      }
    }
  }, [kind]);

  useEffect(() => { void load(); }, [load]);
  usePageActivation(() => void load(true));

  const mutate = async (key: string, action: () => Promise<string>) => {
    if (busy) return;
    setBusy(key);
    setErr("");
    try {
      notifications.show({ message: await action(), color: "teal" });
      await load(true);
    } catch (error) {
      setErr(String(error));
    } finally {
      setBusy("");
    }
  };

  const pickAndInstall = async (destination: "shared" | "target") => {
    try {
      const selected = await open({
        title: kind === "skills" ? "选择包含 SKILL.md 的 Skill 目录" : "选择 Agent Markdown 文件",
        directory: kind === "skills",
        multiple: false,
        ...(kind === "agents" ? { filters: [{ name: "Agent", extensions: ["md"] }] } : {}),
      });
      if (typeof selected !== "string") return;
      await mutate(`install-${destination}`, () =>
        api.installResourceFromPath(kind, selected, destination === "target" ? target : undefined));
    } catch (error) {
      setErr(String(error));
    }
  };

  const emptyIcon = kind === "skills" ? <IconBrain size={24} /> : <IconRobot size={24} />;

  return (
    <Stack gap="md" className="extension-manager">
      <Card withBorder radius="lg" className="extension-control-card">
      <Group justify="space-between" align="flex-start" wrap="wrap" gap="md">
        <div>
          <Group gap={6}>
            <Text fw={700}>共享 {label}</Text>
            <FeatureHelp content={help} />
          </Group>
          <Text size="xs" c="dimmed">默认 Claude 只用于发现可导入项；共享更新不会覆盖目标自己的同名版本。</Text>
        </div>
        <Group gap="xs" className="extension-toolbar">
          <Group gap={4}>
            <Switch
              size="xs"
              label="自动导入新增项"
              checked={overview?.autoImportEnabled ?? true}
              disabled={!overview || !!busy}
              onChange={(event) => mutate("auto-import", () => api.setResourceAutoImport(event.currentTarget.checked))}
            />
            <FeatureHelp content={AUTO_IMPORT_HELP} />
          </Group>
          <Select
            size="xs"
            w={220}
            value={target}
            onChange={(value) => setTarget(value ?? "")}
            allowDeselect={false}
            placeholder="选择查看目标"
            data={(overview?.targets ?? []).map((item) => ({ value: item.target, label: item.label }))}
          />
          <FeatureHelp content={RESOURCE_OVERRIDE_HELP} />
          <Button size="xs" variant="default" loading={busy === "import-all"} disabled={!!busy}
            onClick={() => mutate("import-all", () => api.importDefaultResource(kind))}>
            从默认 Claude 导入新增项
          </Button>
          <Button size="xs" variant="default" loading={busy === "install-shared"} disabled={!!busy}
            onClick={() => void pickAndInstall("shared")}>安装到共享库</Button>
          <Button size="xs" variant="default" loading={busy === "install-target"} disabled={!!busy || !target}
            onClick={() => void pickAndInstall("target")}>安装到当前目标</Button>
          <Button size="xs" variant="light" leftSection={<IconRefresh size={14} />}
            loading={busy === "sync" || busy === "load"} disabled={!!busy}
            onClick={() => mutate("sync", api.syncExtensionResources)}>
            更新所有目标
          </Button>
        </Group>
      </Group>

      {overview && <Text size="xs" c="dimmed" mt="md" className="extension-check-summary">
        最近自动检查：{lastAutoImport} · 新增 {overview.lastAutoImportAdded} 项 · 跳过 {overview.lastAutoImportSkipped} 项
      </Text>}
      </Card>

      {err && <Alert color="red">{err}</Alert>}
      {(overview?.lastAutoImportFailures.length ?? 0) > 0 && <Alert color="orange" title="最近自动导入有未完成项">
        {overview?.lastAutoImportFailures.map((failure) => <Text size="xs" key={failure}>{failure}</Text>)}
      </Alert>}
      <Card withBorder padding={0} radius="lg" className="extension-table-card">
        <Table highlightOnHover verticalSpacing="sm">
          <Table.Thead><Table.Tr>
            <Table.Th>{label}</Table.Th><Table.Th w={115}>共享库</Table.Th><Table.Th w={125}>默认 Claude</Table.Th>
            <Table.Th w={180}>{overview?.targets.find((item) => item.target === target)?.label ?? "目标状态"}</Table.Th>
            <Table.Th w={250}>操作</Table.Th>
          </Table.Tr></Table.Thead>
          <Table.Tbody>
            {(overview?.items ?? []).map((item) => {
              const status = item.targets.find((entry) => entry.target === target);
              const badge = status ? STATUS_LABEL[status.state] : undefined;
              return <Table.Tr key={item.name}>
                <Table.Td><Text size="sm" fw={600}>{item.name}</Text></Table.Td>
                <Table.Td>{item.inShared ? <Badge color="teal" variant="light">已共享</Badge> : <Text c="dimmed">—</Text>}</Table.Td>
                <Table.Td>{item.inDefaultClaude ? <Badge color="gray" variant="light">已发现</Badge> : <Text c="dimmed">—</Text>}</Table.Td>
                <Table.Td>{badge ? <Group gap={4}><Tooltip multiline maw={320} label={[status?.reason, ...(status?.issues ?? [])].filter(Boolean).join("；")}><Badge color={badge.color} variant="light">{badge.text}</Badge></Tooltip>
                  {status?.state === "unavailable" && <FeatureHelp content={AGENT_DEPENDENCY_HELP} />}</Group> : <Text c="dimmed">—</Text>}</Table.Td>
                <Table.Td><Group gap={6}>
                  {!item.inShared && item.inDefaultClaude && <Button size="compact-xs" variant="light"
                    loading={busy === `import:${item.name}`} disabled={!!busy}
                    onClick={() => mutate(`import:${item.name}`, () => api.importDefaultResource(kind, item.name))}>导入共享库</Button>}
                  {item.inShared && item.inDefaultClaude && <Button size="compact-xs" variant="default"
                    disabled={!!busy} onClick={() => setReplaceItem(item)}>用默认版本更新</Button>}
                  {item.inShared && target && status?.state === "inherited" && <Button size="compact-xs" variant="default"
                    loading={busy === `exclude:${target}:${item.name}`} disabled={!!busy}
                    onClick={() => mutate(`exclude:${target}:${item.name}`, () => api.setResourceExcluded(kind, target, item.name, true))}>在此目标排除</Button>}
                  {item.inShared && target && ["override", "excluded", "missing"].includes(status?.state ?? "") && <Button size="compact-xs" variant="light"
                    loading={busy === `restore:${target}:${item.name}`} disabled={!!busy}
                    onClick={() => mutate(`restore:${target}:${item.name}`, () => api.restoreResourceInheritance(kind, target, item.name))}>恢复共享版本</Button>}
                  {item.inShared && <Button size="compact-xs" variant="subtle" color="red" leftSection={<IconTrash size={13} />}
                    disabled={!!busy} onClick={() => setDeleteItem(item)}>删除共享项</Button>}
                </Group></Table.Td>
              </Table.Tr>;
            })}
            {!busy && (overview?.items.length ?? 0) === 0 && <Table.Tr><Table.Td colSpan={5}>
              <div className="extension-empty-state">
                <div className="extension-empty-icon">{emptyIcon}</div>
                <Text fw={650}>暂未发现 {label}</Text>
                <Text size="xs" c="dimmed">可从默认 Claude 导入，或选择本地{kind === "skills" ? "目录" : "文件"}安装到共享库。</Text>
              </div>
            </Table.Td></Table.Tr>}
          </Table.Tbody>
        </Table>
      </Card>
      {overview && <Text size="xs" c="dimmed" px={4}>共享库：{overview.sharedPath}</Text>}

      <Modal opened={deleteItem !== null} onClose={() => setDeleteItem(null)} title={`删除共享 ${label}`} centered>
        <Stack>
          <Text size="sm">将从共享库删除“{deleteItem?.name}”。仍使用共享版本的目标会删除对应副本；目标自己的版本会保留。</Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setDeleteItem(null)}>取消</Button>
            <Button color="red" loading={busy === `delete:${deleteItem?.name}`} onClick={async () => {
              const item = deleteItem;
              if (!item) return;
              await mutate(`delete:${item.name}`, () => api.deleteSharedResource(kind, item.name));
              setDeleteItem(null);
            }}>确认删除</Button>
          </Group>
        </Stack>
      </Modal>
      <Modal opened={replaceItem !== null} onClose={() => setReplaceItem(null)} title={`更新共享 ${label}`} centered>
        <Stack>
          <Text size="sm">将用默认 Claude 中的“{replaceItem?.name}”整体替换共享版本。仍在继承的目标会收到新版本；目标自己的同名版本不会被覆盖。</Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setReplaceItem(null)}>取消</Button>
            <Button loading={busy === `replace:${replaceItem?.name}`} onClick={async () => {
              const item = replaceItem;
              if (!item) return;
              await mutate(`replace:${item.name}`, () => api.importDefaultResource(kind, item.name, true));
              setReplaceItem(null);
            }}>确认更新</Button>
          </Group>
        </Stack>
      </Modal>
    </Stack>
  );
}

function PluginSharing() {
  const [rows, setRows] = useState<PluginRow[]>([]);
  const [envNames, setEnvNames] = useState<string[]>([]);
  const [target, setTarget] = useState(ALL_ENVS);
  const [pluginName, setPluginName] = useState("");
  const [defaultPlugin, setDefaultPlugin] = useState<string | null>(null);
  const [lastReport, setLastReport] = useState<PluginActionReport | null>(null);
  const [removeName, setRemoveName] = useState("");
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const mutating = useRef(false);
  const loadGeneration = useRef(0);
  const load = () => {
    const generation = ++loadGeneration.current;
    return Promise.all([api.pluginsOverview(), api.pluginTargets()]).then(([next, targets]) => {
      if (generation === loadGeneration.current) {
        setRows(next);
        setEnvNames(targets);
        setTarget((current) => current === ALL_ENVS || targets.includes(current) ? current : ALL_ENVS);
        setErr("");
      }
    }).catch((error) => { if (generation === loadGeneration.current) setErr(String(error)); });
  };
  useEffect(() => { void load(); }, []);
  usePageActivation(() => { if (!mutating.current) void load(); });
  const isAll = target === ALL_ENVS;
  const defaultPlugins = rows.filter((row) => row.defaultInstalled).map((row) => row.name);
  const apply = async (action: () => Promise<string>, key: string) => {
    if (mutating.current) return;
    mutating.current = true; ++loadGeneration.current; setBusy(key); setErr("");
    try { notifications.show({ message: await action(), color: "teal" }); await load(); }
    catch (error) { setErr(String(error)); }
    finally { mutating.current = false; setBusy(""); }
  };

  const manage = async (action: PluginAction, name: string) => {
    const plugin = name.trim();
    if (!plugin || busy || mutating.current) return;
    mutating.current = true;
    ++loadGeneration.current;
    const selected = isAll ? envNames : [target];
    setBusy(`${action}:${plugin}`);
    setErr("");
    setLastReport(null);
    try {
      const report = await api.managePlugin(action, plugin, selected, isAll);
      setLastReport(report);
      const succeeded = report.results.filter((item) => item.ok).length;
      notifications.show({
        message: `${plugin}：${succeeded}/${report.results.length} 个环境操作成功`,
        color: succeeded === report.results.length ? "teal" : "orange",
      });
      await load();
    } catch (error) {
      setErr(String(error));
    } finally {
      mutating.current = false;
      setBusy("");
    }
  };

  return <Stack gap="md" className="extension-manager">
    <Card withBorder radius="lg" className="extension-control-card">
    <Group justify="space-between" align="flex-start" wrap="wrap" gap="md">
      <div><Group gap={6}><Text fw={700}>插件管理</Text><FeatureHelp content={PLUGINS_HELP} /></Group>
        <Text size="xs" c="dimmed">插件在每个环境独立安装；“所有环境”会逐个调用官方命令，默认 Claude 只读。</Text></div>
      <Group gap="xs" className="extension-toolbar">
        <Select size="xs" w={220} value={target} disabled={!!busy} onChange={(value) => value && setTarget(value)} allowDeselect={false}
          data={[{ value: ALL_ENVS, label: "所有环境" }, ...envNames.map((env) => ({ value: env, label: `环境 ${env}` }))]} />
        <Select size="xs" w={235} searchable clearable value={defaultPlugin} onChange={setDefaultPlugin}
          placeholder="默认 Claude 已安装的插件"
          data={defaultPlugins.map((name) => ({ value: name, label: name }))} />
        <Button size="xs" variant="default" disabled={!defaultPlugin || envNames.length === 0 || !!busy}
          loading={!!defaultPlugin && busy === `install:${defaultPlugin}`}
          onClick={() => defaultPlugin && void manage("install", defaultPlugin)}>安装默认 Claude 的插件</Button>
        <TextInput size="xs" w={230} value={pluginName} onChange={(event) => setPluginName(event.currentTarget.value)}
          placeholder="plugin@marketplace" aria-label="插件名称" />
        <Button size="xs" disabled={!pluginName.trim() || envNames.length === 0 || !!busy}
          loading={busy === `install:${pluginName.trim()}`} onClick={() => void manage("install", pluginName)}>安装插件</Button>
      </Group>
    </Group>
    </Card>
    <Alert color="blue" variant="light">安装、更新、启用、停用和卸载均由 Claude Code 官方命令逐环境执行；列表分别展示真实安装状态与启停策略。</Alert>
    {err && <Alert color="red">{err}</Alert>}
    {lastReport && <Alert color={lastReport.results.every((item) => item.ok) ? "teal" : "orange"} title={<Group gap={4}>插件操作结果<FeatureHelp content={PLUGIN_RESULT_HELP} /></Group>}>
      {lastReport.results.map((item) => <Text size="xs" key={item.env}>{item.env}：{item.ok ? "成功" : "失败"} · {item.detail}</Text>)}
      {lastReport.policyWarning && <Text size="xs" c="orange.9" mt={6}>{lastReport.policyWarning}</Text>}
      <Text size="xs" mt={6}>{lastReport.reloadHint}</Text>
    </Alert>}
    <Card withBorder padding={0} radius="lg" className="extension-table-card"><Table verticalSpacing="xs" highlightOnHover>
      <Table.Thead><Table.Tr><Table.Th>插件</Table.Th><Table.Th w={140}>默认 Claude</Table.Th><Table.Th w={190}>{isAll ? "所有环境" : `环境 ${target}`}</Table.Th><Table.Th w={270}>操作</Table.Th></Table.Tr></Table.Thead>
      <Table.Tbody>{rows.map((row) => {
        const envState = row.envs.find((entry) => entry.env === target);
        return <Table.Tr key={row.name}>
          <Table.Td><Text size="sm">{row.name}</Text></Table.Td>
          <Table.Td><Stack gap={3}>{row.defaultInstalled ? <Badge size="xs" variant="light" color="gray">已安装{row.defaultVersion ? ` · ${row.defaultVersion}` : ""}</Badge> : <Text size="xs" c="dimmed">未安装</Text>}
            {row.defaultClaude != null && <Text size="xs" c="dimmed">{row.defaultClaude ? "已启用" : "已停用"}</Text>}</Stack></Table.Td>
          <Table.Td>{isAll ? <Stack gap={4}><Text size="xs">已安装 {row.envs.filter((item) => item.installed).length}/{envNames.length}</Text>
            {row.envs.some((item) => item.storageIndependent === false) && <Badge size="xs" color="orange">存在旧共享目录</Badge>}</Stack> :
            <Group gap="xs"><Switch size="sm" checked={envState?.value === true} disabled={!!busy || !envState?.installed}
              aria-label={`${row.name} ${envState?.value ? "停用" : "启用"}`}
              onChange={(event) => void manage(event.currentTarget.checked ? "enable" : "disable", row.name)} />
              <Badge size="xs" variant="light" color={envState?.installed ? "teal" : "gray"}>{envState?.installed ? `已安装${envState.version ? ` · ${envState.version}` : ""}` : "未安装"}</Badge>
              {envState?.excluded && <Tooltip label="不跟随所有环境的插件操作"><Badge size="xs" variant="light" color="gray">已排除共享策略</Badge></Tooltip>}
              {envState && !envState.inherited && <Tooltip label={envState.reason || "该环境有自己的设置"}><Badge size="xs" variant="light" color="orange">已覆盖</Badge></Tooltip>}</Group>}</Table.Td>
          <Table.Td><Group gap={5}>
            {(!isAll && !envState?.installed || isAll && row.envs.some((item) => !item.installed)) && <Button size="compact-xs" variant="light" disabled={!!busy} loading={busy === `install:${row.name}`} onClick={() => void manage("install", row.name)}>安装</Button>}
            <Button size="compact-xs" variant="default" disabled={!!busy || (isAll ? row.envs.every((item) => !item.installed) : !envState?.installed)} loading={busy === `update:${row.name}`} onClick={() => void manage("update", row.name)}>更新</Button>
            {isAll && <Button size="compact-xs" variant="light" disabled={!!busy || row.envs.every((item) => !item.installed)}
              loading={busy === `${row.shared === true ? "disable" : "enable"}:${row.name}`}
              onClick={() => void manage(row.shared === true ? "disable" : "enable", row.name)}>{row.shared === true ? "全部停用" : "全部启用"}</Button>}
            <Button size="compact-xs" variant="subtle" color="red" disabled={!!busy || (isAll ? row.envs.every((item) => !item.installed) : !envState?.installed)} onClick={() => setRemoveName(row.name)}>卸载</Button>
            {!isAll && envState?.excluded && <Button size="compact-xs" variant="light" loading={busy === `include:${target}:${row.name}`} disabled={!!busy}
              onClick={() => apply(() => api.setPluginExcluded(target, row.name, false), `include:${target}:${row.name}`)}>恢复共享策略</Button>}
            {!isAll && envState && !envState.excluded && row.shared != null && <Button size="compact-xs" variant="default" loading={busy === `exclude-plugin:${target}:${row.name}`} disabled={!!busy}
              onClick={() => apply(() => api.setPluginExcluded(target, row.name, true), `exclude-plugin:${target}:${row.name}`)}>排除共享策略</Button>}
            {!isAll && envState && !envState.excluded && !envState.inherited && row.shared != null && <Button size="compact-xs" variant="light" loading={busy === `restore:${target}:${row.name}`} disabled={!!busy}
              onClick={() => apply(() => api.restorePluginInheritance(target, row.name), `restore:${target}:${row.name}`)}>恢复策略</Button>}
          </Group></Table.Td>
        </Table.Tr>;
      })}</Table.Tbody>
      {rows.length === 0 && <Table.Tbody><Table.Tr><Table.Td colSpan={4}>
        <div className="extension-empty-state">
          <div className="extension-empty-icon"><IconPlugConnected size={24} /></div>
          <Text fw={650}>还没有已知插件</Text>
          <Text size="xs" c="dimmed">选择默认 Claude 中的插件，或输入 plugin@marketplace 开始安装。</Text>
        </div>
      </Table.Td></Table.Tr></Table.Tbody>}
    </Table></Card>
    <Modal opened={!!removeName} onClose={() => setRemoveName("")} title="卸载插件" centered>
      <Stack><Text size="sm">将从{isAll ? "所有环境" : `环境 ${target}`}卸载“{removeName}”。每个环境会分别执行 Claude Code 官方卸载命令。</Text>
        <Group justify="flex-end"><Button variant="default" onClick={() => setRemoveName("")}>取消</Button>
          <Button color="red" onClick={async () => { const name = removeName; await manage("uninstall", name); setRemoveName(""); }}>确认卸载</Button></Group>
      </Stack>
    </Modal>
  </Stack>;
}

export default function ExtensionsPanel() {
  return <div className="view-scroll extensions-scroll"><Tabs defaultValue="skills" keepMounted className="extensions-tabs">
    <Card withBorder radius="lg" className="extensions-nav-card">
      <Group justify="space-between" align="center" wrap="wrap" gap="md">
        <div>
          <Text fw={700}>扩展类型</Text>
          <Text size="xs" c="dimmed">Skills、Plugins 与 Agents 分开管理，Commands 已退出独立管理。</Text>
        </div>
        <Tabs.List className="extensions-tab-list">
          <Tabs.Tab value="skills" leftSection={<IconBrain size={15} />}>Skills</Tabs.Tab>
          <Tabs.Tab value="plugins" leftSection={<IconPlugConnected size={15} />}>Plugins</Tabs.Tab>
          <Tabs.Tab value="agents" leftSection={<IconRobot size={15} />}>Agents</Tabs.Tab>
        </Tabs.List>
      </Group>
    </Card>
    <Tabs.Panel value="skills"><ResourceManager kind="skills" /></Tabs.Panel>
    <Tabs.Panel value="plugins"><PluginSharing /></Tabs.Panel>
    <Tabs.Panel value="agents"><ResourceManager kind="agents" /></Tabs.Panel>
  </Tabs></div>;
}
