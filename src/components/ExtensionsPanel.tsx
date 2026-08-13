import { useEffect, useState } from "react";
import { Alert, Badge, Card, Group, SimpleGrid, Stack, Text, ThemeIcon, Title } from "@mantine/core";
import { IconBrain, IconCommand, IconPlugConnected, IconRobot, IconTool } from "@tabler/icons-react";
import { api } from "../api";
import type { ExtensionGroup } from "../api";
import StableRefreshButton from "./StableRefreshButton";

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
  const load = () => {
    setBusy(true);
    setErr("");
    api.extensionOverview().then(setGroups).catch((e) => setErr(String(e))).finally(() => setBusy(false));
  };
  useEffect(load, []);

  return (
    <div className="view-scroll">
      <Stack gap="md">
        <Group justify="space-between">
          <div><Title order={3}>扩展中心</Title><Text size="sm" c="dimmed">统一查看主账户共享给所有实例的 Skills、Plugins、Agents 与 Commands。</Text></div>
          <StableRefreshButton busy={busy} label="刷新状态" onClick={load} />
        </Group>
        <Alert color="cyan" variant="light">
          这里不提供第三方市场安装。扩展仍通过 Claude Code 或本地文件管理，本工具负责展示和跨实例共享。
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
      </Stack>
    </div>
  );
}
