import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { usePageActivation } from "./PersistentPage";
import { Alert, Badge, Button, Card, Group, Modal, Select, Stack, Switch, Table, Tabs, Text, ThemeIcon, Tooltip } from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { open } from "@tauri-apps/plugin-dialog";
import {
  IconBrain,
  IconFolder,
  IconInfoCircle,
  IconPlugConnected,
  IconPlus,
  IconRefresh,
  IconRobot,
  IconTrash,
} from "@tabler/icons-react";
import { api } from "../api";
import type { ResourceItem, ResourceKind, ResourceOverview } from "../api";
import PluginManager from "./PluginManager";
import FeatureHelp from "./FeatureHelp";
import {
  AGENT_DEPENDENCY_HELP,
} from "./featureHelpContent";

const STATUS_LABEL: Record<string, { text: string; color: string }> = {
  inherited: { text: "已启用", color: "teal" },
  override: { text: "环境自有版本", color: "blue" },
  excluded: { text: "未启用", color: "gray" },
  missing: { text: "未安装", color: "gray" },
  local: { text: "环境自有版本", color: "blue" },
  unavailable: { text: "缺少依赖", color: "red" },
};

function ResourceManager({ kind }: { kind: ResourceKind }) {
  const [overview, setOverview] = useState<ResourceOverview | null>(null);
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const [deleteItem, setDeleteItem] = useState<ResourceItem | null>(null);
  const [replaceItem, setReplaceItem] = useState<ResourceItem | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const inFlight = useRef(false);
  const refreshQueued = useRef(false);
  const label = kind === "skills" ? "Skill" : "Agent";

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
    if (busy) return false;
    setBusy(key);
    setErr("");
    try {
      notifications.show({ message: await action(), color: "teal" });
      await load(true);
      return true;
    } catch (error) {
      setErr(String(error));
      return false;
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
      const installed = await mutate(`install-${destination}`, () =>
        api.installResourceFromPath(kind, selected, destination === "target" ? target : undefined));
      if (installed) setAddOpen(false);
    } catch (error) {
      setErr(String(error));
    }
  };

  const emptyIcon = kind === "skills" ? <IconBrain size={24} /> : <IconRobot size={24} />;
  const targetLabel = overview?.targets.find((item) => item.target === target)?.label ?? "选择环境";

  return (
    <Stack gap="md" className="extension-manager">
      <Card withBorder radius="lg" className="extension-control-card">
        <div className="resource-manager-heading">
          <div className="resource-manager-copy">
            <Text fw={750}>{label} 管理</Text>
            <Text size="sm" c="dimmed">添加、更新，并选择在哪个 Claude 环境中使用。</Text>
          </div>
          <div className="resource-manager-actions">
            <Select
              size="sm"
              value={target}
              onChange={(value) => setTarget(value ?? "")}
              allowDeselect={false}
              placeholder="选择环境"
              label="查看环境"
              aria-label="查看环境"
              className="resource-target-select"
              data={(overview?.targets ?? []).map((item) => ({ value: item.target, label: item.label }))}
            />
            <Button size="sm" variant="default" leftSection={<IconRefresh size={15} />}
            loading={busy === "sync" || busy === "load"} disabled={!!busy}
            onClick={() => mutate("sync", api.syncExtensionResources)}>
              同步环境
            </Button>
            <Button size="sm" leftSection={<IconPlus size={16} />} disabled={!!busy}
              onClick={() => setAddOpen(true)}>添加 {label}</Button>
          </div>
        </div>
      </Card>

      {err && <div className="extension-inline-error" role="alert">
        <ThemeIcon variant="light" color="red" size={34} radius={11}><IconInfoCircle size={18} /></ThemeIcon>
        <div><Text size="sm" fw={700}>操作没有完成</Text><Text size="xs" c="dimmed">{err}</Text></div>
      </div>}
      {(overview?.lastAutoImportFailures.length ?? 0) > 0 && <Alert color="orange" title="最近自动导入有未完成项">
        {overview?.lastAutoImportFailures.map((failure) => <Text size="xs" key={failure}>{failure}</Text>)}
      </Alert>}
      <Card withBorder padding={0} radius="lg" className="extension-table-card">
        <Table highlightOnHover verticalSpacing="sm">
          <Table.Thead><Table.Tr>
            <Table.Th>{label}</Table.Th><Table.Th w={190}>来源</Table.Th>
            <Table.Th w={230}>{targetLabel} 状态</Table.Th>
            <Table.Th w={250}>操作</Table.Th>
          </Table.Tr></Table.Thead>
          <Table.Tbody>
            {(overview?.items ?? []).map((item) => {
              const status = item.targets.find((entry) => entry.target === target);
              const badge = status ? STATUS_LABEL[status.state] : undefined;
              return <Table.Tr key={item.name}>
                <Table.Td><Text size="sm" fw={600}>{item.name}</Text></Table.Td>
                <Table.Td><Group gap={6} wrap="wrap" className="resource-source-badges">
                  {item.inDefaultClaude
                    ? <Badge color="gray" variant="light">默认 Claude</Badge>
                    : item.inShared
                      ? <Badge color="blue" variant="light">本地添加</Badge>
                      : <Text size="xs" c="dimmed">环境自有</Text>}
                </Group></Table.Td>
                <Table.Td>{item.inShared && status && ["inherited", "excluded", "missing"].includes(status.state)
                  ? <Switch
                      size="sm"
                      className="resource-status-switch"
                      color="#00a675"
                      checked={status.state === "inherited"}
                      label={status.state === "inherited" ? "已启用" : "未启用"}
                      aria-label={`${item.name} 在 ${targetLabel} 中${status.state === "inherited" ? "停用" : "启用"}`}
                      disabled={!!busy || !target}
                      onChange={() => status.state === "inherited"
                        ? mutate(`exclude:${target}:${item.name}`, () => api.setResourceExcluded(kind, target, item.name, true))
                        : mutate(`restore:${target}:${item.name}`, () => api.restoreResourceInheritance(kind, target, item.name))}
                    />
                  : badge
                    ? <Group gap={4}><Tooltip multiline maw={320} label={[status?.reason, ...(status?.issues ?? [])].filter(Boolean).join("；")}><Badge color={badge.color} variant="light">{badge.text}</Badge></Tooltip>
                      {status?.state === "unavailable" && <FeatureHelp content={AGENT_DEPENDENCY_HELP} />}</Group>
                    : <Text c="dimmed">—</Text>}</Table.Td>
                <Table.Td><Group gap={6} className="resource-row-actions">
                  {!item.inShared && item.inDefaultClaude && <Button size="compact-xs" variant="light"
                    loading={busy === `import:${item.name}`} disabled={!!busy}
                    onClick={() => mutate(`import:${item.name}`, () => api.importDefaultResource(kind, item.name))}>添加到所有环境</Button>}
                  {item.inShared && item.inDefaultClaude && <Button size="compact-xs" variant="default" leftSection={<IconRefresh size={13} />}
                    disabled={!!busy} onClick={() => setReplaceItem(item)}>更新</Button>}
                  {item.inShared && <Button size="compact-xs" variant="subtle" color="red" leftSection={<IconTrash size={13} />}
                    disabled={!!busy} onClick={() => setDeleteItem(item)}>移除</Button>}
                  {!item.inShared && !item.inDefaultClaude && <Text size="xs" c="dimmed">无需操作</Text>}
                </Group></Table.Td>
              </Table.Tr>;
            })}
            {!busy && (overview?.items.length ?? 0) === 0 && <Table.Tr><Table.Td colSpan={4}>
              <div className="extension-empty-state">
                <div className="extension-empty-icon">{emptyIcon}</div>
                <Text fw={650}>还没有添加 {label}</Text>
                <Text size="xs" c="dimmed">点击“添加 {label}”，可从默认 Claude 导入或选择本地{kind === "skills" ? "目录" : "文件"}。</Text>
              </div>
            </Table.Td></Table.Tr>}
          </Table.Tbody>
        </Table>
      </Card>
      <Modal opened={addOpen} onClose={() => setAddOpen(false)} title={`添加 ${label}`} centered size="lg">
        <Stack gap="sm">
          <div className="resource-add-option">
            <div className="resource-add-option-heading">
              <ThemeIcon variant="light" size={38} radius={12}><IconPlugConnected size={19} /></ThemeIcon>
              <div><Text fw={700}>导入已有 {label}</Text><Text size="xs" c="dimmed">查找默认 Claude 中尚未添加的 {label}，添加后可在所有环境使用。</Text></div>
            </div>
            <Button size="sm" variant="light" loading={busy === "import-all"} disabled={!!busy}
              onClick={async () => { if (await mutate("import-all", () => api.importDefaultResource(kind))) setAddOpen(false); }}>
              查找并导入
            </Button>
          </div>
          <div className="resource-add-option">
            <div className="resource-add-option-heading">
              <ThemeIcon variant="light" size={38} radius={12}><IconFolder size={19} /></ThemeIcon>
              <div><Text fw={700}>从本地添加</Text><Text size="xs" c="dimmed">{kind === "skills" ? "选择包含 SKILL.md 的目录。" : "选择 Agent Markdown 文件。"}</Text></div>
            </div>
            <Group gap="sm" className="resource-add-actions">
              <Button size="sm" variant="light" loading={busy === "install-shared"} disabled={!!busy}
                onClick={() => void pickAndInstall("shared")}>添加到所有环境</Button>
              <Button size="sm" variant="default" loading={busy === "install-target"} disabled={!!busy || !target}
                onClick={() => void pickAndInstall("target")}>只添加到 {targetLabel}</Button>
            </Group>
          </div>
          <div className="resource-auto-setting">
            <div><Text size="sm" fw={650}>自动发现新增项</Text><Text size="xs" c="dimmed">默认 Claude 新增 {label} 后自动添加，无需重复操作。</Text></div>
            <Switch
              aria-label={`自动发现新增 ${label}`}
              checked={overview?.autoImportEnabled ?? true}
              disabled={!overview || !!busy}
              onChange={(event) => mutate("auto-import", () => api.setResourceAutoImport(event.currentTarget.checked))}
            />
          </div>
        </Stack>
      </Modal>
      <Modal opened={deleteItem !== null} onClose={() => setDeleteItem(null)} title={`移除 ${label}`} centered>
        <Stack>
          <Text size="sm">将从 PathMux 管理的所有环境中移除“{deleteItem?.name}”。环境自己单独安装的同名版本会保留。</Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setDeleteItem(null)}>取消</Button>
            <Button color="red" loading={busy === `delete:${deleteItem?.name}`} onClick={async () => {
              const item = deleteItem;
              if (!item) return;
              if (await mutate(`delete:${item.name}`, () => api.deleteSharedResource(kind, item.name))) setDeleteItem(null);
            }}>确认移除</Button>
          </Group>
        </Stack>
      </Modal>
      <Modal opened={replaceItem !== null} onClose={() => setReplaceItem(null)} title={`更新 ${label}`} centered>
        <Stack>
          <Text size="sm">将使用默认 Claude 中的“{replaceItem?.name}”更新所有由 PathMux 管理的环境；环境自己安装的同名版本不会被覆盖。</Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setReplaceItem(null)}>取消</Button>
            <Button loading={busy === `replace:${replaceItem?.name}`} onClick={async () => {
              const item = replaceItem;
              if (!item) return;
              if (await mutate(`replace:${item.name}`, () => api.importDefaultResource(kind, item.name, true))) setReplaceItem(null);
            }}>确认更新</Button>
          </Group>
        </Stack>
      </Modal>
    </Stack>
  );
}

export default function ExtensionsPanel({ navigationHost }: { navigationHost?: HTMLElement | null }) {
  const navigation = <Tabs.List className="extensions-tab-list">
    <Tabs.Tab value="skills" leftSection={<IconBrain size={15} />}>Skills</Tabs.Tab>
    <Tabs.Tab value="plugins" leftSection={<IconPlugConnected size={15} />}>Plugins</Tabs.Tab>
    <Tabs.Tab value="agents" leftSection={<IconRobot size={15} />}>Agents</Tabs.Tab>
  </Tabs.List>;
  return <div className="view-scroll extensions-scroll"><Tabs defaultValue="skills" keepMounted className="extensions-tabs">
    {navigationHost ? createPortal(navigation, navigationHost) : <Card withBorder radius="lg" className="extensions-nav-card">
      <Group justify="space-between" align="center" wrap="wrap" gap="md">
        <Text fw={700}>扩展类型</Text>
        {navigation}
      </Group>
    </Card>}
    <Tabs.Panel value="skills"><ResourceManager kind="skills" /></Tabs.Panel>
    <Tabs.Panel value="plugins"><PluginManager /></Tabs.Panel>
    <Tabs.Panel value="agents"><ResourceManager kind="agents" /></Tabs.Panel>
  </Tabs></div>;
}
