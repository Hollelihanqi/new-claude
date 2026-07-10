import { useCallback, useEffect, useRef, useState } from "react";
import {
  Box,
  Group,
  Text,
  Badge,
  Alert,
  Container,
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
  IconPuzzle,
} from "@tabler/icons-react";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { notifications } from "@mantine/notifications";
import { Progress } from "@mantine/core";
import type { Update } from "@tauri-apps/plugin-updater";
import { api } from "./api";
import type { EnvInfo } from "./api";
import type { UsageStats } from "./api";
import ConfigPanel from "./components/ConfigPanel";
import UsagePanel, { USAGE_AUTO_OPTIONS } from "./components/UsagePanel";
import GuidePanel from "./components/GuidePanel";
import MarketplacePanel from "./components/MarketplacePanel";
import CaCertButton from "./components/CaCertButton";
import HealthButton from "./components/HealthButton";

type ViewId = "config" | "usage" | "guide" | "marketplace";
type Scheme = "a" | "b";

const USAGE_AUTO_KEY = "cc-usage-auto-refresh";
const USAGE_AUTO_CHOICES = USAGE_AUTO_OPTIONS.map((o) => o.value);

const NAV: { id: ViewId; label: string; icon: typeof IconSettings }[] = [
  { id: "config", label: "实例配置", icon: IconSettings },
  { id: "marketplace", label: "市场", icon: IconPuzzle },
  { id: "usage", label: "用量统计", icon: IconChartLine },
  { id: "guide", label: "使用指南", icon: IconBook2 },
];

function NavPill({
  value,
  onChange,
}: {
  value: ViewId;
  onChange: (v: ViewId) => void;
}) {
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

function StatusPills({ env }: { env: EnvInfo | null }) {
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

export default function App({
  scheme,
  setScheme,
}: {
  scheme: Scheme;
  setScheme: (s: Scheme) => void;
}) {
  const [env, setEnv] = useState<EnvInfo | null>(null);
  const [err, setErr] = useState("");
  const [view, setView] = useState<ViewId>("config");
  const [upd, setUpd] = useState<{ obj?: Update }>({});
  const [appVersion, setAppVersion] = useState("");
  const [usageData, setUsageData] = useState<UsageStats | null>(null);
  const [usageErr, setUsageErr] = useState("");
  const [usageBusy, setUsageBusy] = useState(false);

  const checkUpdate = async (manual: boolean) => {
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

  const renderProg = (pct: number) => (
    <div>
      <div style={{ marginBottom: 6 }}>正在下载更新 {pct}%</div>
      <Progress value={pct} color="brand" size="lg" radius="xl" striped animated />
    </div>
  );

  const installUpdate = async (update?: Update) => {
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
  // 启动即建齐共享链接并跑一轮 MCP/插件启用状态合并(静默,失败不打扰)
  useEffect(() => {
    api.syncAll().catch(() => {});
  }, []);

  // silent=true 时不亮加载态（自动刷新在后台悄悄换数据，不闪按钮 spinner）；
  // 用 ref 防重入：上一次扫描没回来之前，后续触发直接丢弃。
  const usageInflight = useRef(false);
  const loadUsage = useCallback((silent = false) => {
    if (usageInflight.current) return;
    usageInflight.current = true;
    if (!silent) {
      setUsageBusy(true);
      setUsageErr("");
    }
    api.usageStats()
      .then((d) => {
        setUsageData(d);
        setUsageErr("");
      })
      .catch((e) => setUsageErr(String(e)))
      .finally(() => {
        usageInflight.current = false;
        if (!silent) setUsageBusy(false);
      });
  }, []);
  // 自动刷新间隔（秒），0 表示关闭；在用量页可改，选择记进 localStorage。
  const [usageAuto, setUsageAuto] = useState(() => {
    const v = parseInt(localStorage.getItem(USAGE_AUTO_KEY) || "", 10);
    return USAGE_AUTO_CHOICES.includes(v) ? v : 30;
  });
  const changeUsageAuto = useCallback((sec: number) => {
    setUsageAuto(sec);
    localStorage.setItem(USAGE_AUTO_KEY, String(sec));
  }, []);
  // 后端按文件 mtime/size 缓存解析结果，只有首次是全量扫描，之后仅重读有变动的
  // 活跃会话文件。因此启动即后台预扫一次（首次进用量页立刻有数据），
  // 停留在用量页期间按所选间隔静默刷新；手动「刷新」按钮保留，随时可立即重扫。
  useEffect(() => {
    loadUsage(true);
  }, [loadUsage]);
  useEffect(() => {
    if (view !== "usage") return;
    loadUsage(true);
    if (!usageAuto) return;
    const t = setInterval(() => loadUsage(true), usageAuto * 1000);
    return () => clearInterval(t);
  }, [view, usageAuto, loadUsage]);
  useEffect(() => {
    checkUpdate(false);
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  const accentBg = scheme === "a" ? "#d7e7e3" : "#ecdfd0";

  return (
    <Box
      style={{
        height: "100vh",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
        background: accentBg,
      }}
    >
      {/* 顶栏 + 导航：固定占自然高度，整页不滚 */}
      <Box style={{ flex: "0 0 auto", zIndex: 10 }}>
        {/* 顶栏 */}
        <Box style={{ background: "var(--mantine-primary-color-filled)" }}>
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
                <HealthButton />
                <CaCertButton env={env} onChanged={refreshEnv} />
                <Group gap={8}>
                  {([
                    { k: "a", c: "#fd752c", t: "橘橙主题" },
                    { k: "b", c: "#0c7e9e", t: "孔雀蓝主题" },
                  ] as { k: Scheme; c: string; t: string }[]).map((it) => (
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

      {/* 内容区：填满剩余空间，内部各自滚动，整页不滚 */}
      <Box
        style={{ flex: "1 1 auto", minHeight: 0, overflow: "hidden" }}
        className="glass-content"
      >
        <Container
          fluid
          px={48}
          py="lg"
          style={{ height: "100%", display: "flex", flexDirection: "column" }}
        >
          {err && (
            <Alert
              color="red"
              icon={<IconAlertTriangle size={16} />}
              mb="md"
              title="后端通信失败"
              style={{ flex: "0 0 auto" }}
            >
              {err}
            </Alert>
          )}
          {env && !env.claude_found && (view === "config" || view === "marketplace") && (
            <Alert
              color="orange"
              icon={<IconAlertTriangle size={16} />}
              mb="md"
              title="还没装 Claude Code"
              style={{ flex: "0 0 auto" }}
            >
              请先安装 Claude Code、确认终端能运行 <code>claude</code>，再来配置。
            </Alert>
          )}

          {/* 各视图填满剩余高度；超出则各自滚动 */}
          <Box style={{ flex: "1 1 auto", minHeight: 0 }}>
            {view === "config" && <ConfigPanel onChanged={refreshEnv} />}
            {view === "marketplace" && <MarketplacePanel />}
            {view === "usage" && (
              <div style={{ height: "100%", overflowY: "auto" }}>
                <UsagePanel
                  data={usageData}
                  err={usageErr}
                  busy={usageBusy}
                  autoSec={usageAuto}
                  onAutoChange={changeUsageAuto}
                  onRefresh={() => loadUsage()}
                />
              </div>
            )}
            {view === "guide" && (
              <div style={{ height: "100%", overflowY: "auto" }}>
                <GuidePanel />
              </div>
            )}
          </Box>
        </Container>
      </Box>
    </Box>
  );
}
