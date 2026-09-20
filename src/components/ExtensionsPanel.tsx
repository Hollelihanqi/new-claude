import { useCallback, useEffect, useRef, useState } from "react";
import { usePageActivation } from "./PersistentPage";
import { Alert, Badge, Button, Card, Group, Modal, Select, Stack, Switch, Table, Tabs, Text, TextInput, Tooltip } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { open } from "@tauri-apps/plugin-dialog";
import {
  IconBrain,
  IconChevronDown,
  IconDownload,
  IconInfoCircle,
  IconPackage,
  IconPlayerPause,
  IconPlayerPlay,
  IconPlugConnected,
  IconRefresh,
  IconRobot,
  IconTrash,
} from "@tabler/icons-react";
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
  const scopeLabel = isAll ? "所有环境" : `环境 ${target}`;
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

  return <Stack gap="md" className="extension-manager plugin-manager">
    <Card withBorder radius="lg" className="extension-control-card plugin-control-card">
      <div className="plugin-control-heading">
        <div className="plugin-control-icon"><IconPlugConnected size={20} /></div>
        <div>
          <Group gap={6}><Text fw={700}>插件管理</Text><FeatureHelp content={PLUGINS_HELP} /></Group>
          <Text size="xs" c="dimmed">先选择要管理的环境，再安装或调整插件。默认 Claude 仅作为插件来源，不会被修改。</Text>
        </div>
      </div>

      <div className="plugin-scope-panel">
        <div className="plugin-scope-copy">
          <Text size="xs" fw={700}>当前操作范围</Text>
          <Text size="sm" fw={650}>{scopeLabel}</Text>
          <Text size="xs" c="dimmed">
            {isAll
              ? `安装、更新或启停会依次作用于 ${envNames.length} 个环境。`
              : `接下来的操作只会影响 ${target}，其他环境保持不变。`}
          </Text>
        </div>
        <Select
          size="sm"
          label="选择管理范围"
          aria-label="选择插件管理范围"
          value={target}
          disabled={!!busy}
          onChange={(value) => value && setTarget(value)}
          allowDeselect={false}
          data={[{ value: ALL_ENVS, label: "所有环境（批量管理）" }, ...envNames.map((env) => ({ value: env, label: `仅环境 ${env}` }))]}
        />
      </div>

      <details className="plugin-install-disclosure">
        <summary>
          <span>
            <IconDownload size={18} />
            <span>
              <Text size="sm" fw={650}>安装新插件</Text>
              <Text size="xs" c="dimmed">从默认 Claude 复制，或使用 Marketplace 插件标识。</Text>
            </span>
          </span>
          <IconChevronDown size={18} className="plugin-install-chevron" />
        </summary>
      <div className="plugin-install-grid">
        <section className="plugin-install-option" aria-label="从默认 Claude 复制插件">
          <div className="plugin-install-option-heading">
            <IconDownload size={18} />
            <div>
              <Text size="sm" fw={650}>从默认 Claude 复制</Text>
              <Text size="xs" c="dimmed">选择已经安装过的插件，省去手动输入。</Text>
            </div>
          </div>
          <div className="plugin-install-controls">
            <Select
              size="sm"
              searchable
              clearable
              value={defaultPlugin}
              onChange={setDefaultPlugin}
              placeholder={defaultPlugins.length ? "选择一个插件" : "默认 Claude 暂无插件"}
              aria-label="选择默认 Claude 中的插件"
              data={defaultPlugins.map((name) => ({ value: name, label: name }))}
            />
            <Button
              size="sm"
              variant="default"
              leftSection={<IconDownload size={15} />}
              disabled={!defaultPlugin || envNames.length === 0 || !!busy}
              loading={!!defaultPlugin && busy === `install:${defaultPlugin}`}
              onClick={() => defaultPlugin && void manage("install", defaultPlugin)}
            >安装到{scopeLabel}</Button>
          </div>
        </section>

        <section className="plugin-install-option" aria-label="从 Marketplace 安装插件">
          <div className="plugin-install-option-heading">
            <IconPackage size={18} />
            <div>
              <Text size="sm" fw={650}>从 Marketplace 安装</Text>
              <Text size="xs" c="dimmed">粘贴完整插件标识，例如 plugin@marketplace。</Text>
            </div>
          </div>
          <div className="plugin-install-controls">
            <TextInput
              size="sm"
              value={pluginName}
              onChange={(event) => setPluginName(event.currentTarget.value)}
              placeholder="plugin@marketplace"
              aria-label="Marketplace 插件标识"
            />
            <Button
              size="sm"
              leftSection={<IconDownload size={15} />}
              disabled={!pluginName.trim() || envNames.length === 0 || !!busy}
              loading={busy === `install:${pluginName.trim()}`}
              onClick={() => void manage("install", pluginName)}
            >安装到{scopeLabel}</Button>
          </div>
        </section>
      </div>
      </details>
    </Card>

    <div className="plugin-reading-guide">
      <IconInfoCircle size={18} />
      <Text size="xs"><strong>怎么看：</strong>“默认 Claude”只显示可复制的来源；“{scopeLabel}”才是当前正在管理的真实状态。每次操作都会逐个环境执行并显示结果。</Text>
    </div>
    {err && <Alert color="red">{err}</Alert>}
    {lastReport && <Alert color={lastReport.results.every((item) => item.ok) ? "teal" : "orange"} title={<Group gap={4}>插件操作结果<FeatureHelp content={PLUGIN_RESULT_HELP} /></Group>}>
      {lastReport.results.map((item) => <Text size="xs" key={item.env}>{item.env}：{item.ok ? "成功" : "失败"} · {item.detail}</Text>)}
      {lastReport.policyWarning && <Text size="xs" c="orange.9" mt={6}>{lastReport.policyWarning}</Text>}
      <Text size="xs" mt={6}>{lastReport.reloadHint}</Text>
    </Alert>}
    <Card withBorder padding={0} radius="lg" className="plugin-list-card">
      <div className="plugin-list-heading">
        <div>
          <Group gap={8}><Text fw={700}>已发现的插件</Text><Badge size="sm" variant="light">{rows.length}</Badge></Group>
          <Text size="xs" c="dimmed">每张卡片依次显示插件来源、安装状态和当前可执行的操作。</Text>
        </div>
        <Text size="xs" c="dimmed">正在查看：<strong>{scopeLabel}</strong></Text>
      </div>
      <div className="plugin-list">{rows.map((row) => {
        const envState = row.envs.find((entry) => entry.env === target);
        const installedCount = row.envs.filter((item) => item.installed).length;
        const enabledCount = row.envs.filter((item) => item.installed && item.value === true).length;
        const allInstalledEnabled = installedCount > 0
          && row.envs.filter((item) => item.installed).every((item) => item.value === true);
        const splitAt = row.name.lastIndexOf("@");
        const displayName = splitAt > 0 ? row.name.slice(0, splitAt) : row.name;
        const sourceName = splitAt > 0 ? row.name.slice(splitAt + 1) : "自定义来源";
        const canManage = isAll ? installedCount > 0 : envState?.installed === true;
        const needsInstall = isAll ? installedCount < envNames.length : !envState?.installed;
        return <article className="plugin-card" key={row.name}>
          <div className="plugin-card-summary">
            <div className="plugin-identity">
              <div className="plugin-identity-icon"><IconPlugConnected size={19} /></div>
              <div>
                <Text size="sm" fw={700}>{displayName}</Text>
                <Text size="xs" c="dimmed">来源：{sourceName}</Text>
                <code title={row.name}>{row.name}</code>
              </div>
            </div>

            <div className="plugin-status-block">
              <Text className="plugin-status-label">默认 Claude（仅参考）</Text>
              <Group gap={6} wrap="wrap">
                <Badge size="sm" variant="light" color={row.defaultInstalled ? "gray" : "dark"}>
                  {row.defaultInstalled ? `已安装${row.defaultVersion ? ` · ${row.defaultVersion}` : ""}` : "未安装"}
                </Badge>
                {row.defaultClaude != null && <Badge size="sm" variant="dot" color={row.defaultClaude ? "teal" : "gray"}>{row.defaultClaude ? "已启用" : "已停用"}</Badge>}
              </Group>
              <Text size="xs" c="dimmed">这里的状态不会被本页操作修改。</Text>
            </div>

            <div className="plugin-status-block">
              <Text className="plugin-status-label">{scopeLabel}的真实状态</Text>
              {isAll ? <>
                <Group gap={6} wrap="wrap">
                  <Badge size="sm" variant="light" color={installedCount === envNames.length ? "teal" : "blue"}>{installedCount}/{envNames.length} 已安装</Badge>
                  <Badge size="sm" variant="light" color={enabledCount === installedCount && installedCount > 0 ? "teal" : "gray"}>{enabledCount}/{installedCount} 已启用</Badge>
                </Group>
                <Text size="xs" c="dimmed">批量操作会按环境逐一执行。</Text>
                {row.envs.some((item) => item.storageIndependent === false) && <Badge size="xs" color="orange" variant="light">部分环境仍使用旧目录</Badge>}
              </> : <>
                <Group gap={6} wrap="wrap">
                  <Badge size="sm" variant="light" color={envState?.installed ? "teal" : "gray"}>{envState?.installed ? `已安装${envState.version ? ` · ${envState.version}` : ""}` : "未安装"}</Badge>
                  {envState?.installed && <Badge size="sm" variant="dot" color={envState.value ? "teal" : "gray"}>{envState.value ? "已启用" : "已停用"}</Badge>}
                  {envState?.excluded && <Badge size="sm" variant="light" color="gray">单独管理</Badge>}
                  {envState && !envState.inherited && !envState.excluded && <Badge size="sm" variant="light" color="orange">与批量设置不同</Badge>}
                </Group>
                <Text size="xs" c="dimmed">只反映环境 {target}，不会混入其他环境。</Text>
              </>}
            </div>
          </div>

          <div className="plugin-card-actions">
            <div className="plugin-action-main">
              <Text className="plugin-action-label">{isAll ? "批量操作" : `环境 ${target} 的操作`}</Text>
              <Group gap={7} wrap="wrap">
                {needsInstall && <Button size="sm" variant="light" leftSection={<IconDownload size={15} />} disabled={!!busy}
                  loading={busy === `install:${row.name}`} onClick={() => void manage("install", row.name)}>安装缺少的环境</Button>}
                <Button size="sm" variant="default" leftSection={<IconRefresh size={15} />} disabled={!!busy || !canManage}
                  loading={busy === `update:${row.name}`} onClick={() => void manage("update", row.name)}>检查并更新</Button>
                {isAll ? <Button size="sm" variant="light" leftSection={allInstalledEnabled ? <IconPlayerPause size={15} /> : <IconPlayerPlay size={15} />}
                  disabled={!!busy || !canManage} loading={busy === `${allInstalledEnabled ? "disable" : "enable"}:${row.name}`}
                  onClick={() => void manage(allInstalledEnabled ? "disable" : "enable", row.name)}>{allInstalledEnabled ? "在全部环境停用" : "在全部环境启用"}</Button> :
                  <Switch size="md" label={envState?.value ? "此环境已启用" : "此环境已停用"} checked={envState?.value === true}
                    disabled={!!busy || !envState?.installed} aria-label={`${row.name} ${envState?.value ? "停用" : "启用"}`}
                    onChange={(event) => void manage(event.currentTarget.checked ? "enable" : "disable", row.name)} />}
              </Group>
            </div>
            <div className="plugin-danger-action">
              <Text className="plugin-action-label">危险操作</Text>
              <Button size="sm" variant="subtle" color="red" leftSection={<IconTrash size={15} />} disabled={!!busy || !canManage}
                onClick={() => setRemoveName(row.name)}>从{scopeLabel}卸载</Button>
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
              {envState.excluded && <Button size="compact-sm" variant="light" loading={busy === `include:${target}:${row.name}`} disabled={!!busy}
                onClick={() => apply(() => api.setPluginExcluded(target, row.name, false), `include:${target}:${row.name}`)}>重新跟随批量设置</Button>}
              {!envState.excluded && !envState.inherited && <Button size="compact-sm" variant="light" loading={busy === `restore:${target}:${row.name}`} disabled={!!busy}
                onClick={() => apply(() => api.restorePluginInheritance(target, row.name), `restore:${target}:${row.name}`)}>恢复为批量设置</Button>}
              {!envState.excluded && <Button size="compact-sm" variant="default" loading={busy === `exclude-plugin:${target}:${row.name}`} disabled={!!busy}
                onClick={() => apply(() => api.setPluginExcluded(target, row.name, true), `exclude-plugin:${target}:${row.name}`)}>改为单独管理</Button>}
            </Group>
          </div>}
        </article>;
      })}
      {rows.length === 0 &&
        <div className="extension-empty-state">
          <div className="extension-empty-icon"><IconPlugConnected size={24} /></div>
          <Text fw={650}>还没有已知插件</Text>
          <Text size="xs" c="dimmed">可以从默认 Claude 复制，或输入 plugin@marketplace 安装到{scopeLabel}。</Text>
        </div>
      }</div>
    </Card>
    <Modal opened={!!removeName} onClose={() => setRemoveName("")} title="卸载插件" centered>
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
