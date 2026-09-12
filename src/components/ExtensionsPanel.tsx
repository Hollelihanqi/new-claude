import { useEffect, useRef, useState } from "react";
import { usePageActivation } from "./PersistentPage";
import {
  Alert,
  Badge,
  Button,
  Card,
  Group,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Table,
  Text,
  ThemeIcon,
  Title,
  Tooltip,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { IconBrain, IconCommand, IconPlugConnected, IconRobot, IconTool } from "@tabler/icons-react";
import { api } from "../api";
import type { ExtensionGroup, PluginRow } from "../api";
import StableRefreshButton from "./StableRefreshButton";

/** 「所有环境」的哨兵值 —— 与 MCP 那边同一套语义：写共享库，然后单向分发。 */
const ALL_ENVS = "__all__";

/** 插件共享设置：默认编辑「所有环境」，也可以切到某个环境看它的继承/覆盖。 */
function PluginSharing() {
  const [rows, setRows] = useState<PluginRow[]>([]);
  const [target, setTarget] = useState<string>(ALL_ENVS);
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const mutating = useRef(false);
  const loadGeneration = useRef(0);

  const load = () => {
    const generation = ++loadGeneration.current;
    return api
      .pluginsOverview()
      .then((next) => { if (generation === loadGeneration.current) { setRows(next); setErr(""); } })
      .catch((e) => { if (generation === loadGeneration.current) setErr(String(e)); });
  };
  useEffect(() => { void load(); }, []);
  usePageActivation(() => { if (!mutating.current) void load(); });

  const envNames = rows[0]?.envs.map((e) => e.env) ?? [];
  const isAll = target === ALL_ENVS;

  const apply = async (fn: () => Promise<string>, key: string) => {
    if (mutating.current) return;
    mutating.current = true;
    ++loadGeneration.current;
    setBusy(key);
    setErr("");
    try {
      notifications.show({ message: await fn(), color: "teal" });
      await load();
    } catch (e) {
      setErr(String(e));
    } finally {
      mutating.current = false;
      setBusy("");
    }
  };

  return (
    <Card withBorder padding="lg" radius="lg">
      <Group justify="space-between" align="flex-start" wrap="nowrap">
        <div>
          <Text fw={700}>插件共享设置</Text>
          <Text size="xs" c="dimmed">
            共享插件设置统一分发；各环境可独立覆盖。默认 Claude 只展示、不会被修改。
          </Text>
        </div>
        <Select
          size="xs"
          w={220}
          value={target}
          disabled={!!busy}
          onChange={(v) => v && setTarget(v)}
          allowDeselect={false}
          data={[
            { value: ALL_ENVS, label: "所有环境（共享设置）" },
            ...envNames.map((e) => ({ value: e, label: `环境 ${e}（独立设置）` })),
          ]}
        />
      </Group>

      {err && (
        <Alert color="red" mt="sm">
          {err}
        </Alert>
      )}

      <Table mt="sm" verticalSpacing="xs" highlightOnHover>
        <Table.Thead>
          <Table.Tr>
            <Table.Th>插件</Table.Th>
            <Table.Th w={110}>默认 Claude</Table.Th>
            <Table.Th w={150}>{isAll ? "共享（所有环境）" : `环境 ${target}`}</Table.Th>
            <Table.Th w={130} />
          </Table.Tr>
        </Table.Thead>
        <Table.Tbody>
          {rows.map((row) => {
            const envState = row.envs.find((e) => e.env === target);
            return (
              <Table.Tr key={row.name}>
                <Table.Td>
                  <Text size="sm">{row.name}</Text>
                </Table.Td>
                <Table.Td>
                  {/* 默认 Claude 只展示：约束 4（应用对它只读） */}
                  {row.defaultClaude === undefined || row.defaultClaude === null ? (
                    <Text size="xs" c="dimmed">
                      —
                    </Text>
                  ) : (
                    <Badge size="xs" variant="light" color="gray">
                      {row.defaultClaude ? "启用" : "停用"}
                    </Badge>
                  )}
                </Table.Td>
                <Table.Td>
                  {isAll ? (
                    <Switch
                      size="sm"
                      checked={row.shared === true}
                      disabled={!!busy}
                      onChange={(e) =>
                        apply(
                          () => api.setSharedPlugin(row.name, e.currentTarget.checked),
                          row.name
                        )
                      }
                    />
                  ) : (
                    <Group gap="xs">
                      <Switch
                        size="sm"
                        checked={envState?.value === true}
                        disabled={!!busy}
                        onChange={(e) =>
                          apply(
                            () => api.setEnvPlugin(target, row.name, e.currentTarget.checked),
                            `${target}:${row.name}`
                          )
                        }
                      />
                      {envState && !envState.inherited && (
                        <Tooltip label={envState.reason || "该环境有自己的设置"}>
                          <Badge size="xs" variant="light" color="orange">
                            已覆盖
                          </Badge>
                        </Tooltip>
                      )}
                    </Group>
                  )}
                </Table.Td>
                <Table.Td>
                  {!isAll && envState && !envState.inherited && row.shared !== null && (
                    <Button
                      size="compact-xs"
                      variant="light"
                      loading={busy === `restore:${target}:${row.name}`}
                      disabled={!!busy}
                      onClick={() =>
                        apply(
                          () => api.restorePluginInheritance(target, row.name),
                          `restore:${target}:${row.name}`
                        )
                      }
                    >
                      恢复继承
                    </Button>
                  )}
                </Table.Td>
              </Table.Tr>
            );
          })}
        </Table.Tbody>
      </Table>
      {rows.length === 0 && (
        <Text size="xs" c="dimmed" mt="sm">
          还没有已知的插件。插件由 Claude Code 安装后会自动出现在这里。
        </Text>
      )}
    </Card>
  );
}

const ICONS = {
  skills: IconBrain,
  plugins: IconPlugConnected,
  agents: IconRobot,
  commands: IconCommand,
};

export default function ExtensionsPanel() {
  const [groups, setGroups] = useState<ExtensionGroup[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const inFlight = useRef(false);
  const load = (quiet = false) => {
    if (inFlight.current) return;
    inFlight.current = true;
    if (!quiet) setBusy(true);
    setErr("");
    api.extensionOverview().then(setGroups).catch((e) => setErr(String(e))).finally(() => { inFlight.current = false; setBusy(false); });
  };
  useEffect(load, []);
  usePageActivation(() => load(true));

  return (
    <div className="view-scroll">
      <Stack gap="md">
        <Group justify="space-between">
          <div>
            <Title order={3}>扩展中心</Title>
            <Text size="sm" c="dimmed">
              查看所有环境共享的 Skills、Plugins、Agents 与 Commands，并管理插件的共享启用状态。
            </Text>
          </div>
          <StableRefreshButton busy={busy} label="刷新状态" onClick={load} />
        </Group>
        <Alert color="cyan" variant="light">
          这里不提供第三方市场安装。扩展仍通过 Claude Code 或本地文件管理，本工具负责展示和跨环境共享。
        </Alert>
        {err && <Alert color="red">{err}</Alert>}
        <SimpleGrid cols={{ base: 1, sm: 2, lg: 3 }}>
          {groups.map((group) => {
            const Icon = ICONS[group.kind] || IconTool;
            return (
              <Card key={group.kind} withBorder padding="lg" radius="lg" className="extension-card">
                <Group justify="space-between" align="flex-start">
                  <ThemeIcon size={42} radius="md" variant="light"><Icon size={21} /></ThemeIcon>
                  <Badge size="lg" variant="light">{group.items.length}</Badge>
                </Group>
                <Text fw={700} mt="md">{group.label}</Text>
                <Text size="xs" c="dimmed" lineClamp={1}>{group.path}</Text>
                <div className="extension-items">
                  {group.items.length ? group.items.slice(0, 8).map((item) => <span key={item}>{item}</span>) : <Text size="xs" c="dimmed">暂未发现已配置项目</Text>}
                  {group.items.length > 8 && <Text size="xs" c="dimmed">另有 {group.items.length - 8} 项</Text>}
                </div>
              </Card>
            );
          })}
        </SimpleGrid>
        <PluginSharing />
      </Stack>
    </div>
  );
}
