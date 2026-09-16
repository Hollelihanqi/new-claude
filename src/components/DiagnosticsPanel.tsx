import { useEffect, useRef, useState } from "react";
import { Alert, Badge, Button, Card, Code, Group, Loader, Stack, Text, ThemeIcon } from "@mantine/core";
import { IconAlertTriangle, IconCircleCheck, IconCircleX, IconFileDownload, IconTool } from "@tabler/icons-react";
import { api } from "../api";
import type { HealthItem } from "../api";
import StableRefreshButton from "./StableRefreshButton";
import EnvironmentProof from "./EnvironmentProof";
import { usePageActive } from "./PersistentPage";

const STATUS = {
  ok: { color: "teal", Icon: IconCircleCheck },
  warn: { color: "yellow", Icon: IconAlertTriangle },
  fail: { color: "red", Icon: IconCircleX },
};

export default function DiagnosticsPanel() {
  const pageActive = usePageActive();
  const [items, setItems] = useState<HealthItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [healthState, setHealthState] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [healthError, setHealthError] = useState("");
  const [logError, setLogError] = useState("");
  const requestId = useRef(0);
  const logRequestId = useRef(0);
  const [logs, setLogs] = useState<string[]>([]);
  const [action, setAction] = useState("");
  const [message, setMessage] = useState<{ ok: boolean; text: string }>({ ok: true, text: "" });
  const inFlight = useRef(false);
  const loadLogs = () => {
    const request = ++logRequestId.current;
    setLogError("");
    void api.recentSyncLog()
      .then((recentLogs) => { if (request === logRequestId.current) setLogs(recentLogs); })
      .catch((e) => { if (request === logRequestId.current) { setLogs([]); setLogError(String(e)); } });
  };
  const run = (quiet = false) => {
    if (inFlight.current) return;
    inFlight.current = true;
    const request = ++requestId.current;
    if (!quiet) { setBusy(true); setHealthState("loading"); }
    setHealthError("");
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
    loadLogs();
  };
  // 进入诊断页只读取不含凭证的同步日志。完整健康检查会读取网关钥匙串凭证，
  // 因此必须由用户明确点击「开始诊断」，不能由后台预热或页面切换自动触发。
  useEffect(() => {
    if (!pageActive) return;
    loadLogs();
    return () => {
      requestId.current += 1;
      logRequestId.current += 1;
      inFlight.current = false;
      setBusy(false);
      setHealthState((state) => state === "loading" ? "idle" : state);
    };
  }, [pageActive]);
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
        <Card withBorder padding="lg" radius="lg" className="diagnostic-summary diagnostic-overview-card">
        <Group justify="space-between" align="center" wrap="wrap" gap="md">
          <div>
            <Text size="xs" c="dimmed" fw={700}>OVERALL HEALTH</Text>
            <Group gap="xs" align="center">
              <Text fw={750} size="xl">{healthState === "loading" ? "正在检测环境" : healthState === "error" ? "检测未完成，请重试" : healthState === "idle" ? "尚未诊断" : problems ? `${problems} 项需要处理` : "所有检查均正常"}</Text>
              {!busy && <Badge size="lg" variant="light" color={healthState !== "success" ? "gray" : problems ? "orange" : "teal"}>{healthState !== "success" ? "暂无结论" : problems ? "需要关注" : "健康"}</Badge>}
            </Group>
            {healthState === "idle" && <Text size="xs" c="dimmed" mt={3}>进入页面不会自动读取凭证；点击“开始诊断”后才会检查环境。</Text>}
          </div>
          <Group gap="xs">
            <StableRefreshButton busy={busy} busyLabel="检测中…" label={healthState === "idle" ? "开始诊断" : "重新检测"} onClick={run} />
            {/* 不用 Mantine 的 loading 属性：它会隐藏按钮文字只剩转圈，用户看不出
                正在同步。改成显式 Loader + 文字切换，忙碌状态一眼可辨。 */}
            <Button
              leftSection={action === "sync" ? <Loader size={15} /> : <IconTool size={15} />}
              disabled={action === "sync"}
              onClick={sync}
            >
              {action === "sync" ? "正在同步" : "同步并修复"}
            </Button>
            <Button variant="default" leftSection={<IconFileDownload size={15} />} onClick={exportReport} loading={action === "export"}>导出诊断</Button>
          </Group>
        </Group>
        </Card>
        {/* 环境证明卡：把散落各页的结论汇成一处，同事报障时先看这里 */}
        {healthState === "success" && <EnvironmentProof items={items} />}
        {message.text && (
          <Alert color={message.ok ? "teal" : "red"}>
            {/* 「同步并修复」的结果是逐行汇报（做了什么 / 影响范围 / 每条警告），必须保留换行 */}
            <Text size="sm" style={{ whiteSpace: "pre-line" }}>
              {message.text}
            </Text>
          </Alert>
        )}
        {healthError && <Alert color="red">{healthError}</Alert>}
        <div className="diagnostic-list">
          {items.map((item) => {
            const ui = STATUS[item.status] || STATUS.fail;
            return <Card key={item.id} withBorder padding="md" radius="lg"><Group wrap="nowrap" align="flex-start"><ThemeIcon color={ui.color} variant="light" radius="xl"><ui.Icon size={16} /></ThemeIcon><div><Text fw={650} size="sm">{item.label}</Text><Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>{item.detail}</Text></div></Group></Card>;
          })}
        </div>
        <Card withBorder padding="lg" radius="lg">
          {logError && <Alert color="orange">日志读取失败：{logError}</Alert>}
          <Group justify="space-between" mb="sm"><div><Text fw={700}>最近同步日志</Text><Text size="xs" c="dimmed">最多显示最近 80 行，用于追踪跨环境配置传播。</Text></div><Badge variant="light" color="gray">{logs.length} 行</Badge></Group>
          <Code block className="sync-log-block">{logs.length ? logs.join("\n") : "暂无同步日志"}</Code>
        </Card>
      </Stack>
    </div>
  );
}
