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
  IconDownload,
} from "@tabler/icons-react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
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
          variant="white"
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
  const [upd, setUpd] = useState({ state: "idle" });

  const checkUpdate = async (manual) => {
    try {
      setUpd({ state: "checking" });
      const update = await check();
      if (update) {
        setUpd({ state: "available", version: update.version, obj: update });
      } else {
        setUpd({ state: manual ? "latest" : "idle" });
      }
    } catch (e) {
      setUpd({ state: manual ? "error" : "idle", msg: String(e) });
    }
  };

  const installUpdate = async () => {
    try {
      setUpd((u) => ({ ...u, state: "installing", progress: 0 }));
      let total = 0;
      let downloaded = 0;
      await upd.obj.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength || 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength || 0;
          const pct = total ? Math.round((downloaded / total) * 100) : 0;
          setUpd((u) => ({ ...u, state: "installing", progress: pct }));
        } else if (event.event === "Finished") {
          setUpd((u) => ({ ...u, state: "installing", progress: 100 }));
        }
      });
      await relaunch();
    } catch (e) {
      setUpd({ state: "error", msg: String(e) });
    }
  };

  const refreshEnv = () => {
    api.environment().then(setEnv).catch((e) => setErr(String(e)));
    api.listProfiles().then(setProfiles).catch(() => {});
  };
  useEffect(refreshEnv, []);
  useEffect(() => {
    checkUpdate(false);
  }, []);

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
          background: "var(--mantine-primary-color-filled)",
        }}
      >
        <Container fluid px={36} py="lg">
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
                </Text>
              </div>
            </Group>
            <Group gap="sm" wrap="nowrap">
              <Button
                size="xs"
                variant="white"
                leftSection={<IconDownload size={14} />}
                onClick={() => checkUpdate(true)}
                loading={upd.state === "checking"}
              >
                {upd.state === "latest" ? "已是最新" : "检查更新"}
              </Button>
              <ModelDetect profiles={profiles} />
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
        {upd.state === "installing" && (
          <div style={{ height: 3, background: "rgba(0,0,0,0.06)" }}>
            <div
              style={{
                height: "100%",
                width: `${upd.progress || 0}%`,
                background: "var(--mantine-primary-color-filled)",
                transition: "width .2s ease",
              }}
            />
          </div>
        )}
      </Box>

      {/* 导航 */}
      <Container fluid px={36} pt="xl" pb="xs" className="glass-content">
        <NavPill value={view} onChange={setView} />
      </Container>

      {/* 内容 */}
      <Container fluid px={36} pt="md" pb={48} className="glass-content">
        {err && (
          <Alert color="red" icon={<IconAlertTriangle size={16} />} mb="md" title="后端通信失败">
            {err}
          </Alert>
        )}
        {upd.state === "available" && (
          <Alert color="brand" icon={<IconDownload size={16} />} mb="md" title={`发现新版本 ${upd.version}`}>
            <Group justify="space-between" align="center">
              <Text size="sm">有可用更新，点击右侧按钮即可后台下载并自动重启。</Text>
              <Button size="xs" onClick={installUpdate}>立即更新</Button>
            </Group>
          </Alert>
        )}
        {upd.state === "installing" && (
          <Alert color="brand" mb="md" title="正在更新">
            正在下载并安装更新（{upd.progress || 0}%），完成后会自动重启，请稍候…
          </Alert>
        )}
        {upd.state === "error" && (
          <Alert color="red" mb="md" title="更新失败" withCloseButton onClose={() => setUpd({ state: "idle" })}>
            {upd.msg}
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
