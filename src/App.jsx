import { useEffect, useState } from "react";
import {
  Box,
  Group,
  Text,
  Badge,
  Alert,
  Container,
  SegmentedControl,
  Popover,
  Select,
  Button,
  Stack,
  Loader,
} from "@mantine/core";
import {
  IconSettings,
  IconChartLine,
  IconBook2,
  IconRoute,
  IconBrandApple,
  IconBrandWindows,
  IconCircleCheck,
  IconAlertTriangle,
  IconCertificate,
  IconListSearch,
} from "@tabler/icons-react";
import { api } from "./api.js";
import ConfigPanel from "./components/ConfigPanel.jsx";
import UsagePanel from "./components/UsagePanel.jsx";
import GuidePanel from "./components/GuidePanel.jsx";

const NAV = [
  { id: "config", label: "实例配置", icon: IconSettings },
  { id: "usage", label: "用量统计", icon: IconChartLine },
  { id: "guide", label: "使用指南", icon: IconBook2 },
];

function NavPill({ value, onChange }) {
  return (
    <div
      style={{
        display: "inline-flex",
        gap: 4,
        padding: 5,
        background: "rgba(0,0,0,0.045)",
        borderRadius: 999,
      }}
    >
      {NAV.map((it) => {
        const Icon = it.icon;
        const active = value === it.id;
        return (
          <button
            key={it.id}
            onClick={() => onChange(it.id)}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 7,
              padding: "9px 18px",
              border: "none",
              cursor: "pointer",
              borderRadius: 999,
              fontSize: 14,
              fontWeight: 600,
              fontFamily: "inherit",
              transition: "all .22s cubic-bezier(.4,0,.2,1)",
              background: active
                ? "var(--mantine-primary-color-filled)"
                : "transparent",
              color: active ? "#fff" : "#6b655f",
              boxShadow: active ? "0 4px 12px rgba(0,0,0,0.15)" : "none",
            }}
          >
            <Icon size={16} />
            {it.label}
          </button>
        );
      })}
    </div>
  );
}

function ModelDetect({ profiles }) {
  const [opened, setOpened] = useState(false);
  const [sel, setSel] = useState(null);
  const [list, setList] = useState([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const routerProfiles = (profiles || []).filter((p) => p.type === "router");
  const opts = routerProfiles.map((p) => ({ value: p.name, label: p.name }));

  const run = (name) => {
    setSel(name);
    setList([]);
    setErr("");
    if (!name) return;
    setBusy(true);
    api
      .detectModelsFor(name)
      .then(setList)
      .catch((e) => setErr(String(e)))
      .finally(() => setBusy(false));
  };

  return (
    <Popover opened={opened} onChange={setOpened} position="bottom-end" width={300} shadow="md">
      <Popover.Target>
        <Button
          size="xs"
          variant="light"
          leftSection={<IconListSearch size={14} />}
          onClick={() => setOpened((o) => !o)}
        >
          模型检测
        </Button>
      </Popover.Target>
      <Popover.Dropdown>
        <Stack gap="xs">
          <Text size="sm" fw={600}>
            查看网关可用模型
          </Text>
          <Select
            placeholder={opts.length ? "选择实例" : "暂无自定义路由实例"}
            data={opts}
            value={sel}
            onChange={run}
            disabled={!opts.length}
            comboboxProps={{ withinPortal: true }}
          />
          {busy && (
            <Group gap={6}>
              <Loader size="xs" />
              <Text size="xs" c="dimmed">
                检测中…
              </Text>
            </Group>
          )}
          {err && (
            <Text size="xs" c="red">
              {err}
            </Text>
          )}
          {list.length > 0 && (
            <>
              <Text size="xs" c="dimmed">
                共 {list.length} 个可用模型：
              </Text>
              <Group gap={5}>
                {list.map((m) => (
                  <Badge
                    key={m}
                    variant="light"
                    style={{ textTransform: "none", cursor: "pointer" }}
                    onClick={() => {
                      try {
                        navigator.clipboard?.writeText(m);
                      } catch (_) {}
                    }}
                  >
                    {m}
                  </Badge>
                ))}
              </Group>
            </>
          )}
        </Stack>
      </Popover.Dropdown>
    </Popover>
  );
}

function StatusPills({ env }) {
  if (!env) return null;
  const PlatformIcon =
    env.platform === "macos" ? IconBrandApple : IconBrandWindows;
  const platformLabel =
    env.platform === "macos" ? "macOS" : env.platform === "windows" ? "Windows" : "未知";
  const pill = { radius: 999, variant: "light", size: "lg" };
  return (
    <Group gap={8}>
      <Badge {...pill} color="gray" leftSection={<PlatformIcon size={13} />}>
        {platformLabel}
      </Badge>
      <Badge
        {...pill}
        color={env.claude_found ? "teal" : "orange"}
        leftSection={env.claude_found ? <IconCircleCheck size={13} /> : <IconAlertTriangle size={13} />}
      >
        {env.claude_found ? "claude 就绪" : "未装 claude"}
      </Badge>
      <Badge
        {...pill}
        color={env.cert_imported ? "teal" : "gray"}
        leftSection={<IconCertificate size={13} />}
      >
        {env.cert_imported ? "证书已导入" : "未导入证书"}
      </Badge>
    </Group>
  );
}

export default function App({ scheme, setScheme }) {
  const [env, setEnv] = useState(null);
  const [err, setErr] = useState("");
  const [view, setView] = useState("config");
  const [profiles, setProfiles] = useState([]);

  const refreshEnv = () => {
    api.environment().then(setEnv).catch((e) => setErr(String(e)));
    api.listProfiles().then(setProfiles).catch(() => {});
  };
  useEffect(refreshEnv, []);

  useEffect(() => {
    document.body.className = "theme-" + scheme;
  }, [scheme]);

  return (
    <Box style={{ minHeight: "100vh" }}>
      {/* 顶栏 */}
      <Box
        style={{
          position: "sticky",
          top: 0,
          zIndex: 10,
          background: "#ffffff",
          borderBottom: "1px solid rgba(0,0,0,0.07)",
        }}
      >
        <Container fluid px="xl" py="sm">
          <Group justify="space-between" wrap="nowrap">
            <Group gap="sm" wrap="nowrap">
              <Box
                style={{
                  width: 40,
                  height: 40,
                  borderRadius: 13,
                  background: "var(--mantine-primary-color-filled)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  flexShrink: 0,
                }}
              >
                <IconRoute size={22} color="#fff" />
              </Box>
              <div>
                <Text fw={700} size="md" lh={1.1}>
                  Claude 路由管理
                </Text>
                <Text size="xs" c="dimmed">
                  多账户 / 公司网关，一处配好
                </Text>
              </div>
            </Group>
            <Group gap="sm" wrap="nowrap">
              <ModelDetect profiles={profiles} />
              <SegmentedControl
                size="xs"
                value={scheme}
                onChange={setScheme}
                data={[
                  { label: "A 橘橙", value: "a" },
                  { label: "B 孔雀蓝", value: "b" },
                ]}
              />
              <StatusPills env={env} />
            </Group>
          </Group>
        </Container>
      </Box>

      {/* 导航 */}
      <Container fluid px="xl" pt="lg" pb="xs" className="glass-content">
        <NavPill value={view} onChange={setView} />
      </Container>

      {/* 内容 */}
      <Container fluid px="xl" pb="xl" className="glass-content">
        {err && (
          <Alert color="red" icon={<IconAlertTriangle size={16} />} mb="md" title="后端通信失败">
            {err}
          </Alert>
        )}
        {env && !env.claude_found && view === "config" && (
          <Alert color="orange" icon={<IconAlertTriangle size={16} />} mb="md" title="还没装 Claude Code">
            请先安装 Claude Code、确认终端能运行 <code>claude</code>，再来配置。
          </Alert>
        )}

        {view === "config" && <ConfigPanel env={env} onChanged={refreshEnv} />}
        {view === "usage" && <UsagePanel scheme={scheme} />}
        {view === "guide" && <GuidePanel env={env} />}
      </Container>
    </Box>
  );
}
