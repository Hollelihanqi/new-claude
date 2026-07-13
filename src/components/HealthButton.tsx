import { useState } from "react";
import {
  Button,
  Modal,
  Stack,
  Group,
  Text,
  Badge,
  Alert,
  ThemeIcon,
  Loader,
} from "@mantine/core";
import {
  IconStethoscope,
  IconCircleCheck,
  IconAlertTriangle,
  IconCircleX,
  IconFileDownload,
  IconRefresh,
  IconInfoCircle,
} from "@tabler/icons-react";
import { api } from "../api";
import type { HealthItem } from "../api";

const STATUS_UI: Record<
  HealthItem["status"],
  { color: string; Icon: typeof IconCircleCheck }
> = {
  ok: { color: "teal", Icon: IconCircleCheck },
  warn: { color: "yellow", Icon: IconAlertTriangle },
  fail: { color: "red", Icon: IconCircleX },
};

// 顶栏「健康检查」：点开弹框才实际探测（含网关连通，最长十几秒），不在启动时自动跑。
export default function HealthButton() {
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<HealthItem[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [exportBusy, setExportBusy] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [syncMsg, setSyncMsg] = useState<{ ok: boolean; msg: string }>({ ok: true, msg: "" });
  const [exportMsg, setExportMsg] = useState<{ ok: boolean; msg: string }>({
    ok: true,
    msg: "",
  });

  const run = () => {
    setBusy(true);
    setErr("");
    api
      .healthCheck()
      .then(setItems)
      .catch((e) => setErr(String(e)))
      .finally(() => setBusy(false));
  };

  const onOpen = () => {
    setOpen(true);
    setExportMsg({ ok: true, msg: "" });
    setSyncMsg({ ok: true, msg: "" });
    run();
  };

  const onSync = async () => {
    setSyncBusy(true);
    setSyncMsg({ ok: true, msg: "" });
    try {
      const msg = await api.syncAll();
      setSyncMsg({ ok: true, msg });
      run();
    } catch (e) {
      setSyncMsg({ ok: false, msg: String(e) });
    } finally {
      setSyncBusy(false);
    }
  };

  const onExport = async () => {
    setExportBusy(true);
    setExportMsg({ ok: true, msg: "" });
    try {
      const path = await api.exportDiagnostics();
      setExportMsg({
        ok: true,
        msg: `已导出到 ${path}（已在文件管理器中定位）。出问题时把这个文件发给管理员即可。`,
      });
    } catch (e) {
      setExportMsg({ ok: false, msg: String(e) });
    } finally {
      setExportBusy(false);
    }
  };

  const problems = (items || []).filter((i) => i.status !== "ok").length;

  return (
    <>
      <Button
        size="xs"
        variant="light"
        leftSection={<IconStethoscope size={14} />}
        onClick={onOpen}
      >
        健康检查
      </Button>

      <Modal
        opened={open}
        onClose={() => setOpen(false)}
        title={
          <Group gap={6} wrap="nowrap">
            <IconStethoscope size={18} />
            <Text fw={600}>健康检查</Text>
            {items && !busy && (
              <Badge
                size="sm"
                variant="light"
                color={problems ? "orange" : "teal"}
              >
                {problems ? `${problems} 项需要注意` : "一切正常"}
              </Badge>
            )}
          </Group>
        }
        size="lg"
        centered
      >
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            逐项检查 claude CLI、终端集成、共享链接、CA 证书，并实际探测各路由网关的连通性与
            Key 有效性。
          </Text>

          {busy && (
            <Group gap="xs" p="sm">
              <Loader size="sm" />
              <Text size="sm" c="dimmed">
                正在检测…（会实际请求各网关，最长约 15 秒）
              </Text>
            </Group>
          )}

          {err && (
            <Alert color="red" icon={<IconInfoCircle size={16} />}>
              {err}
            </Alert>
          )}

          {!busy &&
            items &&
            items.map((it) => {
              const { color, Icon } = STATUS_UI[it.status] || STATUS_UI.fail;
              return (
                <Group key={it.id} gap="sm" wrap="nowrap" align="flex-start">
                  <ThemeIcon variant="light" color={color} size="md" radius="xl">
                    <Icon size={16} />
                  </ThemeIcon>
                  <div style={{ minWidth: 0 }}>
                    <Text size="sm" fw={600} lh={1.3}>
                      {it.label}
                    </Text>
                    <Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>
                      {it.detail}
                    </Text>
                  </div>
                </Group>
              );
            })}

          {exportMsg.msg && (
            <Alert
              variant="light"
              color={exportMsg.ok ? "teal" : "red"}
              icon={<IconInfoCircle size={16} />}
            >
              <span style={{ wordBreak: "break-all" }}>{exportMsg.msg}</span>
            </Alert>
          )}

          {syncMsg.msg && (
            <Alert
              variant="light"
              color={syncMsg.ok ? "teal" : "red"}
              icon={<IconInfoCircle size={16} />}
            >
              <span style={{ wordBreak: "break-all" }}>{syncMsg.msg}</span>
            </Alert>
          )}

          <Group gap="xs" mt={4}>
            <Button
              size="sm"
              variant="light"
              leftSection={<IconRefresh size={14} />}
              onClick={run}
              loading={busy}
            >
              重新检测
            </Button>
            <Button
              size="sm"
              variant="default"
              leftSection={<IconRefresh size={14} />}
              onClick={onSync}
              loading={syncBusy}
              disabled={busy || exportBusy}
            >
              立即同步并修复
            </Button>
            <Button
              size="sm"
              variant="default"
              leftSection={<IconFileDownload size={14} />}
              onClick={onExport}
              loading={exportBusy}
              disabled={busy}
            >
              导出诊断文件
            </Button>
          </Group>

          <Text size="xs" c="dimmed">
            诊断文件只包含检查结果、脱敏后的实例配置和同步日志，不包含任何 API Key。
          </Text>
        </Stack>
      </Modal>
    </>
  );
}
