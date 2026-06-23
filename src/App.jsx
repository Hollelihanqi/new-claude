import { useEffect, useState } from "react";
import { Box, Group, Text, Badge, Alert, Container } from "@mantine/core";
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
        background: "rgba(255,102,0,0.07)",
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
                ? "linear-gradient(135deg,#ff6600,#ff8a3d)"
                : "transparent",
              color: active ? "#fff" : "#8a7d72",
              boxShadow: active ? "0 6px 16px rgba(255,102,0,0.32)" : "none",
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

function StatusPills({ env }) {
  if (!env) return null;
  const PlatformIcon =
    env.platform === "macos" ? IconBrandApple : IconBrandWindows;
  const platformLabel =
    env.platform === "macos" ? "macOS" : env.platform === "windows" ? "Windows" : "未知";
  const pill = {
    radius: 999,
    variant: "light",
    size: "lg",
  };
  return (
    <Group gap={8}>
      <Badge {...pill} color="gray" leftSection={<PlatformIcon size={13} />}>
        {platformLabel}
      </Badge>
      <Badge
        {...pill}
        color={env.claude_found ? "teal" : "orange"}
        leftSection={
          env.claude_found ? (
            <IconCircleCheck size={13} />
          ) : (
            <IconAlertTriangle size={13} />
          )
        }
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

export default function App() {
  const [env, setEnv] = useState(null);
  const [err, setErr] = useState("");
  const [view, setView] = useState("config");

  const refreshEnv = () => {
    api
      .environment()
      .then(setEnv)
      .catch((e) => setErr(String(e)));
  };
  useEffect(refreshEnv, []);

  return (
    <Box className="glass-bg" style={{ minHeight: "100vh" }}>
      {/* 顶部栏 */}
      <Box
        style={{
          position: "sticky",
          top: 0,
          zIndex: 10,
          background: "rgba(255,255,255,0.55)",
          backdropFilter: "blur(18px) saturate(140%)",
          borderBottom: "1px solid rgba(255,102,0,0.12)",
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
                  background: "linear-gradient(135deg,#ff6600,#ff8a3d)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  boxShadow: "0 6px 16px rgba(255,102,0,0.32)",
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
            <StatusPills env={env} />
          </Group>
        </Container>
      </Box>

      {/* 导航胶囊 */}
      <Container fluid px="xl" pt="lg" pb="xs" className="glass-content">
        <NavPill value={view} onChange={setView} />
      </Container>

      {/* 内容 */}
      <Container fluid px="xl" pb="xl" className="glass-content">
        {err && (
          <Alert
            color="red"
            icon={<IconAlertTriangle size={16} />}
            mb="md"
            radius="lg"
            title="后端通信失败"
          >
            {err}
            <Text size="xs" mt={4} c="dimmed">
              若在浏览器里直接打开（非桌面应用），属正常——系统操作仅在打包后的应用里可用。
            </Text>
          </Alert>
        )}

        {env && !env.claude_found && view === "config" && (
          <Alert
            color="orange"
            icon={<IconAlertTriangle size={16} />}
            mb="md"
            radius="lg"
            title="还没装 Claude Code"
          >
            这个工具用来配置 Claude Code。请先安装它、确认终端能运行 <code>claude</code>，再来配置。
          </Alert>
        )}

        {view === "config" && <ConfigPanel env={env} onChanged={refreshEnv} />}
        {view === "usage" && <UsagePanel />}
        {view === "guide" && <GuidePanel env={env} />}
      </Container>
    </Box>
  );
}
