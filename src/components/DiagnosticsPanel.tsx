import { useEffect, useRef, useState } from "react";
import { Alert, Badge, Button, Card, Code, Group, Loader, Stack, Text, ThemeIcon, Title } from "@mantine/core";
import { IconAlertTriangle, IconCircleCheck, IconCircleX, IconFileDownload, IconTool } from "@tabler/icons-react";
import { api } from "../api";
import type { HealthItem } from "../api";
import StableRefreshButton from "./StableRefreshButton";
import { usePageActivation } from "./PersistentPage";

const STATUS = {
  ok: { color: "teal", Icon: IconCircleCheck },
  warn: { color: "yellow", Icon: IconAlertTriangle },
  fail: { color: "red", Icon: IconCircleX },
};

export default function DiagnosticsPanel() {
  const [items, setItems] = useState<HealthItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [healthState, setHealthState] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [healthError, setHealthError] = useState("");
  const [logError, setLogError] = useState("");
  const requestId = useRef(0);
  const [logs, setLogs] = useState<string[]>([]);
  const [action, setAction] = useState("");
  const [message, setMessage] = useState<{ ok: boolean; text: string }>({ ok: true, text: "" });
  const inFlight = useRef(false);
  const run = (quiet = false) => {
    if (inFlight.current) return;
    inFlight.current = true;
    const request = ++requestId.current;
    if (!quiet) { setBusy(true); setHealthState("loading"); }
    setHealthError("");
    setLogError("");
    void api.healthCheck()
      .then((health) => {
        if (request !== requestId.current) return;
        if (!health.length) {
          setHealthError("未返回检查结果，请重新检测。");
          if (!quiet) { setItems([]); setHealthState("error"); }
          return;
        }
        setItems(health);
        setHealthState("success");
      })
      .catch((e) => {
        if (request !== requestId.current) return;
        if (!quiet) { setItems([]); setHealthState("error"); }
        setHealthError(String(e));
      })
      .finally(() => { if (request === requestId.current) { inFlight.current = false; setBusy(false); } });
    void api.recentSyncLog()
      .then((recentLogs) => { if (request === requestId.current) setLogs(recentLogs); })
      .catch((e) => { if (request === requestId.current) { setLogs([]); setLogError(String(e)); } });
  };
  useEffect(() => { run(); return () => { requestId.current += 1; inFlight.current = false; }; }, []);
  usePageActivation(() => run(true));
  const sync = async () => {
    setAction("sync");
    try { setMessage({ ok: true, text: await api.syncAll() }); run(); }
    catch (e) { setMessage({ ok: false, text: String(e) }); }
    finally { setAction(""); }
  };
  const exportReport = async () => {
    setAction("export");
    try { setMessage({ ok: true, text: `诊断文件已导出：${await api.exportDiagnostics()}` }); }
    catch (e) { setMessage({ ok: false, text: String(e) }); }
    finally { setAction(""); }
  };
  const problems = items.filter((item) => item.status !== "ok").length;

  return (
    <div className="view-scroll">
      <Stack gap="md">
        <Group justify="space-between">
          <div><Title order={3}>诊断中心</Title><Text size="sm" c="dimmed">集中检查、修复和导出环境状态。</Text></div>
          <Group gap="xs">
            <StableRefreshButton busy={busy} busyLabel="检测中…" label="重新检测" onClick={run} />
            <Button leftSection={<IconTool size={15} />} onClick={sync} loading={action === "sync"}>同步并修复</Button>
            <Button variant="default" leftSection={<IconFileDownload size={15} />} onClick={exportReport} loading={action === "export"}>导出诊断</Button>
          </Group>
        </Group>
        <Card withBorder padding="lg" radius="lg" className="diagnostic-summary">
          <Group justify="space-between">
            <div><Text size="xs" c="dimmed" fw={700}>OVERALL HEALTH</Text><Text fw={750} size="xl">{healthState === "loading" ? "正在检测环境" : healthState === "error" ? "检测未完成，请重试" : healthState === "idle" ? "尚未检测" : problems ? `${problems} 项需要处理` : "所有检查均正常"}</Text></div>
            {busy ? <Loader /> : <Badge size="xl" variant="light" color={healthState !== "success" ? "gray" : problems ? "orange" : "teal"}>{healthState !== "success" ? "暂无结论" : problems ? "需要关注" : "健康"}</Badge>}
          </Group>
        </Card>
        {message.text && <Alert color={message.ok ? "teal" : "red"}>{message.text}</Alert>}
        {healthError && <Alert color="red">{healthError}</Alert>}
        <div className="diagnostic-list">
          {items.map((item) => {
            const ui = STATUS[item.status] || STATUS.fail;
            return <Card key={item.id} withBorder padding="md" radius="lg"><Group wrap="nowrap" align="flex-start"><ThemeIcon color={ui.color} variant="light" radius="xl"><ui.Icon size={16} /></ThemeIcon><div><Text fw={650} size="sm">{item.label}</Text><Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>{item.detail}</Text></div></Group></Card>;
          })}
        </div>
        <Card withBorder padding="lg" radius="lg">
          {logError && <Alert color="orange">日志读取失败：{logError}</Alert>}
          <Group justify="space-between" mb="sm"><div><Text fw={700}>最近同步日志</Text><Text size="xs" c="dimmed">最多显示最近 80 行，用于追踪跨实例配置传播。</Text></div><Badge variant="light" color="gray">{logs.length} 行</Badge></Group>
          <Code block className="sync-log-block">{logs.length ? logs.join("\n") : "暂无同步日志"}</Code>
        </Card>
      </Stack>
    </div>
  );
}
