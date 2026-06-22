import { useEffect, useState } from "react";
import {
  AppShell,
  Group,
  Title,
  Tabs,
  Badge,
  Text,
  Container,
  Alert,
} from "@mantine/core";
import {
  IconSettings,
  IconBook2,
  IconBrandApple,
  IconBrandWindows,
  IconAlertTriangle,
  IconCircleCheck,
} from "@tabler/icons-react";
import { api } from "./api.js";
import ConfigPanel from "./components/ConfigPanel.jsx";
import GuidePanel from "./components/GuidePanel.jsx";

export default function App() {
  const [env, setEnv] = useState(null);
  const [err, setErr] = useState("");

  const refreshEnv = () => {
    api
      .environment()
      .then(setEnv)
      .catch((e) => setErr(String(e)));
  };

  useEffect(refreshEnv, []);

  const platformLabel =
    env?.platform === "macos"
      ? "macOS"
      : env?.platform === "windows"
      ? "Windows"
      : env?.platform || "未知";

  const PlatformIcon =
    env?.platform === "macos" ? IconBrandApple : IconBrandWindows;

  return (
    <AppShell header={{ height: 64 }} padding="md">
      <AppShell.Header>
        <Group h="100%" px="md" justify="space-between">
          <Group gap="xs">
            <Title order={3}>Claude Code 配置工具</Title>
            <Text c="dimmed" size="sm">
              账户 / 公司路由，一处管好
            </Text>
          </Group>
          <Group gap="xs">
            {env && (
              <Badge
                variant="light"
                leftSection={<PlatformIcon size={14} />}
                color="gray"
              >
                {platformLabel}
              </Badge>
            )}
            {env &&
              (env.claude_found ? (
                <Badge
                  variant="light"
                  color="teal"
                  leftSection={<IconCircleCheck size={14} />}
                >
                  已检测到 claude
                </Badge>
              ) : (
                <Badge
                  variant="light"
                  color="orange"
                  leftSection={<IconAlertTriangle size={14} />}
                >
                  未检测到 claude
                </Badge>
              ))}
          </Group>
        </Group>
      </AppShell.Header>

      <AppShell.Main>
        <Container size="lg" px={0}>
          {err && (
            <Alert
              color="red"
              icon={<IconAlertTriangle size={16} />}
              mb="md"
              title="后端通信失败"
            >
              {err}
              <Text size="xs" mt={4} c="dimmed">
                如果你在浏览器里直接打开（而非桌面应用），这是正常的——
                系统操作只在打包后的桌面应用里可用。
              </Text>
            </Alert>
          )}

          {env && !env.claude_found && (
            <Alert
              color="orange"
              icon={<IconAlertTriangle size={16} />}
              mb="md"
              title="还没装 Claude Code"
            >
              这个工具是用来配置 Claude Code 的。请先安装它、确认终端里能运行{" "}
              <code>claude</code>，再来这里配置。
            </Alert>
          )}

          <Tabs defaultValue="config" variant="outline">
            <Tabs.List>
              <Tabs.Tab value="config" leftSection={<IconSettings size={16} />}>
                实例配置
              </Tabs.Tab>
              <Tabs.Tab value="guide" leftSection={<IconBook2 size={16} />}>
                使用指南
              </Tabs.Tab>
            </Tabs.List>

            <Tabs.Panel value="config" pt="md">
              <ConfigPanel env={env} onChanged={refreshEnv} />
            </Tabs.Panel>
            <Tabs.Panel value="guide" pt="md">
              <GuidePanel env={env} />
            </Tabs.Panel>
          </Tabs>
        </Container>
      </AppShell.Main>
    </AppShell>
  );
}
