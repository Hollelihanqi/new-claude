import { useCallback, useEffect, useRef, useState } from "react";
import {
  Box,
  Text,
  Badge,
  Alert,
} from "@mantine/core";
import {
  IconLayoutDashboard,
  IconChartLine,
  IconCircleCheck,
  IconAlertTriangle,
  IconChevronRight,
  IconStack2,
  IconStethoscope,
  IconSettings,
  IconHelpCircle,
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
import ExtensionsPanel from "./components/ExtensionsPanel";
import DiagnosticsPanel from "./components/DiagnosticsPanel";
import SettingsPanel from "./components/SettingsPanel";

type ViewId = "environment" | "extensions" | "insights" | "diagnostics" | "settings" | "guide";
type Scheme = "a" | "b";

const USAGE_AUTO_KEY = "cc-usage-auto-refresh";
const USAGE_AUTO_CHOICES = USAGE_AUTO_OPTIONS.map((o) => o.value);

const NAV: { id: ViewId; label: string; desc: string; icon: typeof IconLayoutDashboard }[] = [
  { id: "environment", label: "空间", desc: "实例、网关与模型", icon: IconLayoutDashboard },
  { id: "extensions", label: "扩展", desc: "Skills、MCP 与 Agents", icon: IconStack2 },
  { id: "insights", label: "洞察", desc: "用量、模型与趋势", icon: IconChartLine },
  { id: "diagnostics", label: "诊断", desc: "检查、日志与修复", icon: IconStethoscope },
  { id: "settings", label: "设置", desc: "更新、证书与安全", icon: IconSettings },
];

const VIEW_TITLES: Record<ViewId, string> = {
  environment: "空间管理",
  extensions: "扩展中心",
  insights: "用量洞察",
  diagnostics: "诊断中心",
  settings: "系统设置",
  guide: "使用帮助",
};

function BrandGlyph() {
  return (
    <svg viewBox="0 0 32 32" aria-hidden="true">
      <path d="M21.4 8.8a8.8 8.8 0 1 0 0 14.4" fill="none" stroke="currentColor" strokeWidth="3.8" strokeLinecap="round" />
      <path d="M15.8 16h7" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" />
      <circle cx="15.8" cy="16" r="2.35" fill="#61d9e9" />
      <circle cx="15.8" cy="16" r="1.05" fill="#073e4b" />
    </svg>
  );
}

function SideNav({
  value,
  onChange,
}: {
  value: ViewId;
  onChange: (v: ViewId) => void;
}) {
  return (
    <nav className="side-nav">
      {NAV.map((it) => {
        const Icon = it.icon;
        const active = value === it.id;
        return (
          <button key={it.id} onClick={() => onChange(it.id)} className={`side-nav-item ${active ? "active" : ""}`}>
            <span className="side-nav-icon"><Icon size={19} stroke={1.8} /></span>
            <span className="side-nav-copy"><strong>{it.label}</strong><small>{it.desc}</small></span>
            {active && <IconChevronRight size={15} />}
          </button>
        );
      })}
    </nav>
  );
}

function EnvironmentStatus({ env }: { env: EnvInfo | null }) {
  if (!env) return null;
  return (
    <div className={`env-status ${env.claude_found ? "ready" : "warning"}`}>
      {env.claude_found ? <IconCircleCheck size={17} /> : <IconAlertTriangle size={17} />}
      <div><strong>{env.claude_found ? "环境运行正常" : "环境需要处理"}</strong><span>{env.claude_found ? "Claude Code 已就绪" : "未检测到 Claude CLI"}</span></div>
    </div>
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
  const [view, setView] = useState<ViewId>("environment");
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
  // 启动即建齐共享链接并跑一轮 MCP/插件启用状态合并。
  // 失败不阻断应用，但必须让用户知道当前配置可能尚未同步。
  useEffect(() => {
    api.syncAll().catch((e) => {
      notifications.show({
        color: "orange",
        title: "环境同步未完成",
        message: `${String(e)}。可打开右上角「健康检查」查看原因，修复后重启应用。`,
        autoClose: false,
      });
    });
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
    if (view !== "insights") return;
    loadUsage(true);
    if (!usageAuto) return;
    const t = setInterval(() => loadUsage(true), usageAuto * 1000);
    return () => clearInterval(t);
  }, [view, usageAuto, loadUsage]);
  useEffect(() => {
    checkUpdate(false);
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  return (
    <div className="app-shell">
      <aside className="app-sidebar">
        <div className="brand-block">
          <div className="brand-mark"><BrandGlyph /></div>
          <div className="brand-copy"><strong>Claude 管理中心</strong><span>Claude 本地工作空间</span></div>
        </div>
        <Text className="nav-eyebrow">工作台</Text>
        <SideNav value={view} onChange={setView} />
        <div className="sidebar-spacer" />
        <button className={`sidebar-help ${view === "guide" ? "active" : ""}`} onClick={() => setView("guide")}>
          <IconHelpCircle size={17} /><span>使用帮助</span>
        </button>
        <div className="sidebar-footer">
          <span>Claude Center</span>
          <Badge variant="light" color="gray">v{appVersion || "--"}</Badge>
        </div>
      </aside>

      <main className="app-main">
        <header className="app-header">
          <div>
            <Text className="page-kicker">CLAUDE ENVIRONMENT</Text>
            <Text className="page-title">{VIEW_TITLES[view]}</Text>
          </div>
          <EnvironmentStatus env={env} />
        </header>

        <section className="app-content">
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
          {env && !env.claude_found && view === "environment" && (
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
          <Box className="view-stage">
            {view === "environment" && <ConfigPanel onChanged={refreshEnv} env={env} usageData={usageData} />}
            {view === "extensions" && <ExtensionsPanel />}
            {view === "insights" && (
              <div className="view-scroll">
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
            {view === "diagnostics" && <DiagnosticsPanel />}
            {view === "settings" && <SettingsPanel env={env} scheme={scheme} setScheme={setScheme} appVersion={appVersion} onCheckUpdate={() => checkUpdate(true)} onEnvironmentChanged={refreshEnv} />}
            {view === "guide" && (
              <div className="view-scroll">
                <GuidePanel />
              </div>
            )}
          </Box>
        </section>
      </main>
    </div>
  );
}
