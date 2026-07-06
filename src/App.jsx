import { useEffect, useState } from "react";
import {
  Box,
  Group,
  Text,
  Badge,
  Alert,
  Container,
  SegmentedControl,
  Button,
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
  IconDownload,
} from "@tabler/icons-react";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { notifications } from "@mantine/notifications";
import { Progress } from "@mantine/core";
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

function StatusPills({ env }) {
  if (!env) return null;
  const PlatformIcon =
    env.platform === "macos" ? IconBrandApple : IconBrandWindows;
  const platformLabel =
    env.platform === "macos" ? "macOS" : env.platform === "windows" ? "Windows" : "未知";
  const pill = { radius: 999, variant: "white", size: "lg" };
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
    </Group>
  );
}

export default function App({ scheme, setScheme }) {
  const [env, setEnv] = useState(null);
  const [err, setErr] = useState("");
  const [view, setView] = useState("config");
  const [upd, setUpd] = useState({ state: "idle" });
  const [appVersion, setAppVersion] = useState("");
  const [usageData, setUsageData] = useState(null);
  const [usageErr, setUsageErr] = useState("");
  const [usageBusy, setUsageBusy] = useState(false);

  const checkUpdate = async (manual) => {
    try {
      if (manual) {
        notifications.show({ id: "upd-check", loading: true, title: "检查更新", message: "正在检查…", autoClose: false, withCloseButton: false });
      }
      const update = await check();
      if (update) {
        if (manual) notifications.hide("upd-check");
        notifications.show({
          id: "upd-available",
          color: "blue",
          title: `发现新版本 v${update.version}`,
          message: "点击开始后台下载并自动重启更新。",
          autoClose: false,
          onClick: () => installUpdate(update),
        });
        setUpd({ obj: update });
      } else if (manual) {
        notifications.update({ id: "upd-check", loading: false, color: "teal", title: "已是最新", message: "你当前已经是最新版本。", autoClose: 2500, withCloseButton: true });
      }
    } catch (e) {
      if (!manual) return;
      const raw = String(e);
      const soft = /fetch|json|platform|fallback|network|request|timeout|connect|releases|404/i.test(raw);
      notifications.update({
        id: "upd-check",
        loading: false,
        color: soft ? "teal" : "red",
        title: soft ? "已是最新" : "检查更新出错",
        message: soft ? "你当前已经是最新版本。" : raw,
        autoClose: soft ? 2500 : 6000,
        withCloseButton: true,
      });
    }
  };

  const renderProg = (pct) => (
    <div>
      <div style={{ marginBottom: 6 }}>正在下载更新 {pct}%</div>
      <Progress value={pct} color="brand" size="lg" radius="xl" striped animated />
    </div>
  );

  const installUpdate = async (update) => {
    const obj = update || upd.obj;
    if (!obj) return;
    notifications.hide("upd-available");
    const id = "upd-progress";
    let total = 0;
    let downloaded = 0;
    notifications.show({ id, loading: true, title: "正在更新", message: renderProg(0), autoClose: false, withCloseButton: false });
    try {
      await obj.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength || 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength || 0;
          const pct = total ? Math.round((downloaded / total) * 100) : 0;
          notifications.update({ id, loading: true, title: "正在更新", message: renderProg(pct), autoClose: false, withCloseButton: false });
        } else if (event.event === "Finished") {
          notifications.update({ id, loading: true, title: "正在更新", message: renderProg(100), autoClose: false, withCloseButton: false });
        }
      });
      await relaunch();
    } catch (e) {
      notifications.update({ id, loading: false, color: "red", title: "更新失败", message: "下载或安装失败：" + String(e), autoClose: 6000, withCloseButton: true });
    }
  };

  const refreshEnv = () => {
    api.environment().then(setEnv).catch((e) => setErr(String(e)));
  };
  useEffect(refreshEnv, []);

  const loadUsage = () => {
    setUsageBusy(true);
    setUsageErr("");
    api.usageStats().then(setUsageData).catch((e) => setUsageErr(String(e))).finally(() => setUsageBusy(false));
  };
  useEffect(loadUsage, []);
  useEffect(() => {
    checkUpdate(false);
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  const accentBg = scheme === "a" ? "#d7e7e3" : "#ecdfd0";

  return (
    <Box style={{ minHeight: "100vh", background: accentBg }}>
      {/* 吸顶区：顶栏 + 导航一起固定，不随内容滚动 */}
      <Box style={{ position: "sticky", top: 0, zIndex: 10 }}>
      {/* 顶栏 */}
      <Box
        style={{
          background: "var(--mantine-primary-color-filled)",
        }}
      >
        <Container fluid px={48} py="lg">
          <Group justify="space-between" wrap="nowrap">
            <Group gap="sm" wrap="nowrap">
              <Box
                style={{
                  width: 46,
                  height: 46,
                  borderRadius: 15,
                  background: "rgba(255,255,255,0.18)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  flexShrink: 0,
                }}
              >
                <IconRoute size={26} color="#fff" />
              </Box>
              <div>
                <Text fw={700} size="md" lh={1.1} c="white">
                  Claude 路由管理
                </Text>
                <Text size="xs" style={{ color: "rgba(255,255,255,0.82)" }}>
                  多账户 / 公司网关，一处配好
                  {appVersion ? ` · v${appVersion}` : ""}
                </Text>
              </div>
            </Group>
            <Group gap="sm" wrap="nowrap">
              <Button
                size="xs"
                variant="white"
                leftSection={<IconDownload size={14} />}
                onClick={() => checkUpdate(true)}
              >
                检查更新
              </Button>
              <Group gap={8}>
                {[
                  { k: "a", c: "#fd752c", t: "橘橙主题" },
                  { k: "b", c: "#0c7e9e", t: "孔雀蓝主题" },
                ].map((it) => (
                  <button
                    key={it.k}
                    onClick={() => setScheme(it.k)}
                    title={it.t}
                    style={{
                      width: 24,
                      height: 24,
                      borderRadius: "50%",
                      cursor: "pointer",
                      padding: 0,
                      background: it.c,
                      border:
                        scheme === it.k
                          ? "3px solid #fff"
                          : "2px solid rgba(255,255,255,0.5)",
                      boxShadow:
                        scheme === it.k
                          ? "0 0 0 2px rgba(255,255,255,0.9), 0 2px 6px rgba(0,0,0,0.25)"
                          : "none",
                      transition: "all .2s",
                    }}
                  />
                ))}
              </Group>
              <StatusPills env={env} />
            </Group>
          </Group>
        </Container>
      </Box>

      {/* 导航（居中） */}
      <Box style={{ background: accentBg }}>
        <Container fluid px={48} pt="md" pb="md" className="glass-content">
          <Group justify="center">
            <NavPill value={view} onChange={setView} />
          </Group>
        </Container>
      </Box>
      </Box>
      {/* 吸顶区结束 */}

      {/* 内容 */}
      <Container fluid px={48} pt="lg" pb={56} className="glass-content">
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
        {view === "usage" && (
          <UsagePanel data={usageData} err={usageErr} busy={usageBusy} onRefresh={loadUsage} />
        )}
        {view === "guide" && <GuidePanel env={env} />}
      </Container>
    </Box>
  );
}
