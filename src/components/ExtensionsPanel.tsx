import { useCallback, useEffect, useRef, useState } from "react";
import { usePageActivation } from "./PersistentPage";
import { Alert, Badge, Button, Card, Drawer, Group, Modal, Select, Stack, Switch, Table, Tabs, Text, TextInput, Tooltip } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { open } from "@tauri-apps/plugin-dialog";
import {
  IconBrain,
  IconCircle,
  IconCircleCheckFilled,
  IconFolder,
  IconLoader2,
  IconPackage,
  IconPlugConnected,
  IconPlus,
  IconRefresh,
  IconRobot,
  IconTrash,
  IconWorld,
} from "@tabler/icons-react";
import { api } from "../api";
import type { PluginAction, PluginRow, ResourceItem, ResourceKind, ResourceOverview } from "../api";
import FeatureHelp from "./FeatureHelp";
import {
  AGENT_DEPENDENCY_HELP,
  AGENTS_HELP,
  AUTO_IMPORT_HELP,
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
  const [installOpen, setInstallOpen] = useState(false);
  const [installMode, setInstallMode] = useState<string | null>("remote");
  const [installSource, setInstallSource] = useState("");
  const [localPackage, setLocalPackage] = useState("");
  const [installEnvs, setInstallEnvs] = useState<string[]>([]);
  const [installAllEnvs, setInstallAllEnvs] = useState(true);
  const [pendingEnvKey, setPendingEnvKey] = useState("");
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
  const scopeLabel = isAll ? "所有环境" : `环境 ${target}`;
  const defaultPlugins = rows.filter((row) => row.defaultInstalled).map((row) => row.name);
  const apply = async (action: () => Promise<string>, key: string) => {
    if (mutating.current) return;
    mutating.current = true; ++loadGeneration.current; setBusy(key); setErr("");
    try { notifications.show({ message: await action(), color: "teal" }); await load(); }
    catch (error) { setErr(String(error)); }
    finally { mutating.current = false; setBusy(""); }
  };

  const manage = async (
    action: PluginAction,
    name: string,
    selected = isAll ? envNames : [target],
    sharedScope = isAll,
  ) => {
    const plugin = name.trim();
    if (!plugin || busy || mutating.current) return false;
    mutating.current = true;
    ++loadGeneration.current;
    setBusy(`${action}:${plugin}`);
    setErr("");
    try {
      const report = await api.managePlugin(action, plugin, selected, sharedScope);
      const succeeded = report.results.filter((item) => item.ok).length;
      const failures = report.results.filter((item) => !item.ok);
      notifications.show({
        message: `${plugin}：${succeeded}/${report.results.length} 个环境操作成功${report.policyWarning ? ` · ${report.policyWarning}` : ""}`,
        color: succeeded === report.results.length ? "teal" : "orange",
        className: "plugin-action-notification",
      });
      await load();
      if (failures.length) setErr(failures.map((item) => `${item.env}：${item.detail}`).join("；"));
      return true;
    } catch (error) {
      setErr(String(error));
      return false;
    } finally {
      mutating.current = false;
      setBusy("");
    }
  };

  const openInstaller = () => {
    setInstallEnvs([...envNames]);
    setInstallAllEnvs(true);
    setInstallOpen(true);
    setErr("");
  };

  const toggleInstallEnv = (env: string) => {
    if (installAllEnvs) {
      setInstallAllEnvs(false);
      setInstallEnvs([env]);
      return;
    }
    setInstallEnvs((current) => {
      if (!current.includes(env)) return [...current, env];
      return current.length === 1 ? current : current.filter((item) => item !== env);
    });
  };

  const pickLocalPackage = async () => {
    try {
      const selected = await open({
        title: "选择 Claude Code 插件包",
        multiple: false,
        filters: [{ name: "Claude Code 插件包", extensions: ["zip"] }],
      });
      if (typeof selected === "string") setLocalPackage(selected);
    } catch (error) {
      setErr(String(error));
    }
  };

  const installPackage = async () => {
    const source = installMode === "local" ? localPackage : installSource.trim();
    if (!source || installEnvs.length === 0 || busy || mutating.current) return;
    const sharedScope = installAllEnvs;
    if (installMode === "remote" && !/^https:\/\//i.test(source) && !source.includes("@")) {
      setErr("请输入 plugin@marketplace，或 HTTPS 插件 ZIP 地址");
      return;
    }
    if (installMode === "remote" && !/^https:\/\//i.test(source)) {
      if (await manage("install", source, installEnvs, sharedScope)) setInstallOpen(false);
      return;
    }
    mutating.current = true;
    ++loadGeneration.current;
    setBusy("install-package");
    setErr("");
    try {
      const report = await api.installPluginPackage(source, installEnvs, sharedScope);
      const succeeded = report.results.filter((item) => item.ok).length;
      const failures = report.results.filter((item) => !item.ok);
      notifications.show({
        message: `${report.plugin}：${succeeded}/${report.results.length} 个环境安装成功${report.policyWarning ? ` · ${report.policyWarning}` : ""}`,
        color: succeeded === report.results.length ? "teal" : "orange",
        className: "plugin-action-notification",
      });
      await load();
      if (failures.length) setErr(failures.map((item) => `${item.env}：${item.detail}`).join("；"));
      setInstallOpen(false);
    } catch (error) {
      setErr(String(error));
    } finally {
      mutating.current = false;
      setBusy("");
    }
  };

  const envAction = async (row: PluginRow, env: string) => {
    if (mutating.current) return;
    const state = row.envs.find((item) => item.env === env);
    const action: PluginAction = !state?.installed ? "install" : state.value ? "disable" : "enable";
    setPendingEnvKey(`${row.name}:${env}`);
    try { await manage(action, row.name, [env], false); }
    finally { setPendingEnvKey(""); }
  };

  return <Stack gap="md" className="extension-manager plugin-manager">
    <div className="plugin-toolbar" aria-label="插件环境筛选">
      <div className="plugin-env-tabs" role="tablist" aria-label="选择插件环境">
        {[{ value: ALL_ENVS, label: "全部环境" }, ...envNames.map((env) => ({ value: env, label: env }))].map((item) =>
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={target === item.value}
            className="plugin-env-tab"
            disabled={!!busy}
            onClick={() => setTarget(item.value)}
          >{item.label}</button>
        )}
      </div>
      <Button className="plugin-install-button" leftSection={<IconPlus size={17} />} onClick={openInstaller} disabled={!envNames.length || !!busy}>
        安装插件
      </Button>
    </div>
    {err && <Alert color="red">{err}</Alert>}
    <section className="plugin-list-card" aria-label="已发现的插件">
      <div className="plugin-list-heading">
        <div>
          <Group gap={8}><Text fw={700}>已发现的插件</Text><Badge size="sm" variant="light">{rows.length}</Badge></Group>
        </div>
      </div>
      <div className="plugin-list">{rows.map((row) => {
        const envState = row.envs.find((entry) => entry.env === target);
        const installedCount = row.envs.filter((item) => item.installed).length;
        const splitAt = row.name.lastIndexOf("@");
        const displayName = splitAt > 0 ? row.name.slice(0, splitAt) : row.name;
        const sourceName = splitAt > 0 ? row.name.slice(splitAt + 1) : "自定义来源";
        const canManage = isAll ? installedCount > 0 : envState?.installed === true;
        const version = isAll
          ? row.envs.find((item) => item.installed && item.version)?.version ?? row.defaultVersion
          : envState?.version ?? row.defaultVersion;
        const rowBusy = busy.endsWith(`:${row.name}`);
        return <article className="plugin-card" key={row.name} aria-busy={rowBusy}>
          <div className="plugin-card-body">
            <div className="plugin-identity">
              <div className="plugin-identity-icon"><IconPackage size={21} /></div>
              <div className="plugin-identity-copy">
                <Text fw={700}>{displayName}</Text>
                <Text size="xs" c="dimmed">{version ? `v${version}` : `来自 ${sourceName}`}</Text>
              </div>
            </div>

            <Text className="plugin-card-description" size="sm" c="dimmed">
              {row.defaultInstalled
                ? `默认 Claude 中已发现此插件，来源为 ${sourceName}。`
                : `来自 ${sourceName} 的 Claude Code 插件。`}
            </Text>

            <div className="plugin-env-controls">
              {envNames.map((env) => {
                const state = row.envs.find((item) => item.env === env);
                const mode = !state?.installed ? "missing" : state.value ? "enabled" : "disabled";
                const pending = pendingEnvKey === `${row.name}:${env}`;
                const actionLabel = mode === "missing" ? "安装" : mode === "enabled" ? "停用" : "启用";
                if (mode === "missing") return <button
                    type="button"
                    key={env}
                    className="plugin-env-toggle missing"
                    aria-label={`${row.name} 在环境 ${env} ${actionLabel}`}
                    title={`${actionLabel} · ${env}`}
                    aria-disabled={!!busy}
                    onClick={() => void envAction(row, env)}
                  ><IconPlus size={15} /><span>{env}</span></button>;
                return <Switch
                  key={env}
                  className="plugin-env-switch"
                  size="md"
                  checked={mode === "enabled"}
                  thumbIcon={pending ? <IconLoader2 className="plugin-switch-spinner" size={13} /> : undefined}
                  aria-busy={pending}
                  aria-disabled={!!busy}
                  label={env}
                  labelPosition="left"
                  color="#00a675"
                  withThumbIndicator={false}
                  aria-label={`${row.name} 在环境 ${env} ${actionLabel}`}
                  title={`${actionLabel} · ${env}`}
                  onChange={() => void envAction(row, env)}
                />;
              })}
            </div>

            <div className="plugin-card-actions">
              <Button size="sm" variant="default" leftSection={<IconRefresh size={15} />} disabled={!canManage}
                aria-disabled={!!busy || !canManage}
                loading={busy === `update:${row.name}`} onClick={() => void manage("update", row.name)}>更新</Button>
              <Button size="sm" variant="subtle" color="red" leftSection={<IconTrash size={15} />} disabled={!canManage}
                aria-disabled={!!busy || !canManage}
                onClick={() => { if (!busy) setRemoveName(row.name); }}>卸载</Button>
            </div>
          </div>

          {!isAll && envState && row.shared != null && <div className="plugin-policy-row">
            <div>
              <Text size="xs" fw={650}>与“所有环境”批量设置的关系</Text>
              <Text size="xs" c="dimmed">
                {envState.excluded
                  ? "当前环境已设为单独管理，之后的批量启停不会影响它。"
                  : envState.inherited
                    ? "当前环境会跟随“所有环境”的批量启停。"
                    : `当前状态与批量设置不同${envState.reason ? `：${envState.reason}` : "。"}`}
              </Text>
            </div>
            <Group gap={6} wrap="wrap">
              {envState.excluded && <Button size="compact-sm" variant="light" loading={busy === `include:${target}:${row.name}`} aria-disabled={!!busy}
                onClick={() => apply(() => api.setPluginExcluded(target, row.name, false), `include:${target}:${row.name}`)}>重新跟随批量设置</Button>}
              {!envState.excluded && !envState.inherited && <Button size="compact-sm" variant="light" loading={busy === `restore:${target}:${row.name}`} aria-disabled={!!busy}
                onClick={() => apply(() => api.restorePluginInheritance(target, row.name), `restore:${target}:${row.name}`)}>恢复为批量设置</Button>}
              {!envState.excluded && <Button size="compact-sm" variant="default" loading={busy === `exclude-plugin:${target}:${row.name}`} aria-disabled={!!busy}
                onClick={() => apply(() => api.setPluginExcluded(target, row.name, true), `exclude-plugin:${target}:${row.name}`)}>改为单独管理</Button>}
            </Group>
          </div>}
        </article>;
      })}
      {rows.length === 0 &&
        <div className="extension-empty-state">
          <div className="extension-empty-icon"><IconPlugConnected size={24} /></div>
          <Text fw={650}>还没有已知插件</Text>
          <Text size="xs" c="dimmed">可通过远程地址或本地插件包完成安装。</Text>
        </div>
      }</div>
    </section>
    <Drawer
      opened={installOpen}
      onClose={() => setInstallOpen(false)}
      position="right"
      size={468}
      title={<div><Text fw={750} size="xl">安装插件</Text><Text size="sm" c="dimmed">从远程地址或本地插件包安装到所选环境。</Text></div>}
      overlayProps={{ backgroundOpacity: 0.38, blur: 5 }}
      classNames={{ root: "plugin-install-drawer", content: "plugin-install-drawer-content", header: "plugin-install-drawer-header", body: "plugin-install-drawer-body" }}
    >
      <div className="plugin-install-drawer-layout">
        <Tabs value={installMode} onChange={setInstallMode} className="plugin-install-tabs">
          <Tabs.List grow>
            <Tabs.Tab value="remote" leftSection={<IconWorld size={16} />}>远程地址</Tabs.Tab>
            <Tabs.Tab value="local" leftSection={<IconFolder size={16} />}>本地插件包</Tabs.Tab>
          </Tabs.List>
          <Tabs.Panel value="remote" pt="lg">
            <TextInput
              label="插件来源"
              description="支持 plugin@marketplace 或 HTTPS 插件 ZIP 地址"
              value={installSource}
              onChange={(event) => setInstallSource(event.currentTarget.value)}
              placeholder="plugin@marketplace"
              aria-label="远程插件来源"
              size="md"
            />
            {defaultPlugins.length > 0 && <div className="plugin-source-suggestions">
              {defaultPlugins.slice(0, 4).map((name) => <button key={name} type="button" onClick={() => setInstallSource(name)}>{name}</button>)}
            </div>}
          </Tabs.Panel>
          <Tabs.Panel value="local" pt="lg">
            <button type="button" className={`plugin-package-picker ${localPackage ? "selected" : ""}`} onClick={() => void pickLocalPackage()}>
              <IconFolder size={24} />
              <span><strong>{localPackage ? "已选择插件包" : "选择本地插件包"}</strong><small>{localPackage || "支持 .zip 格式"}</small></span>
            </button>
          </Tabs.Panel>
        </Tabs>

        <div className="plugin-install-env-section">
          <Text fw={700}>安装到哪些环境</Text>
          <Text size="sm" c="dimmed">默认选择全部环境。</Text>
          <div className="plugin-install-envs">
            <button type="button" className={installAllEnvs ? "selected" : ""} aria-pressed={installAllEnvs}
              onClick={() => { setInstallAllEnvs(true); setInstallEnvs([...envNames]); }}>
              {installAllEnvs ? <IconCircleCheckFilled size={18} /> : <IconCircle size={18} />}全部环境
            </button>
            {envNames.map((env) => {
              const selected = !installAllEnvs && installEnvs.includes(env);
              const included = installAllEnvs || selected;
              return <button type="button" key={env} className={selected ? "selected" : included ? "included" : ""} aria-pressed={included}
                onClick={() => toggleInstallEnv(env)}>
                {included ? <IconCircleCheckFilled size={18} /> : <IconCircle size={18} />}{env}
              </button>;
            })}
          </div>
        </div>

        <div className="plugin-install-summary">
          <IconPackage size={21} />
          <div><Text fw={700}>将安装到 {installEnvs.length} 个环境</Text><Text size="xs" c="dimmed">{installEnvs.join("、") || "尚未选择环境"}</Text></div>
        </div>

        <div className="plugin-install-drawer-actions">
          <Button variant="default" size="md" onClick={() => setInstallOpen(false)}>取消</Button>
          <Button size="md" loading={busy === "install-package" || busy.startsWith("install:")}
            disabled={!installEnvs.length || !(installMode === "local" ? localPackage : installSource.trim())}
            onClick={() => void installPackage()}>开始安装</Button>
        </div>
      </div>
    </Drawer>
    <Modal opened={!!removeName} onClose={() => setRemoveName("")} title="卸载插件" centered
      classNames={{ content: "plugin-remove-modal-content", header: "plugin-remove-modal-header", body: "plugin-remove-modal-body", title: "plugin-remove-modal-title" }}>
      <Stack><Text size="sm">将从{scopeLabel}卸载“{removeName}”。{isAll ? "系统会逐个环境执行卸载，其他插件不受影响。" : `只会影响 ${target}，其他环境保持不变。`}</Text>
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
