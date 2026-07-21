import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Badge,
  Box,
  Button,
  Code,
  Collapse,
  Group,
  Stack,
  Switch,
  Text,
  Textarea,
} from "@mantine/core";
import {
  IconAlertTriangle,
  IconChevronDown,
  IconChevronRight,
  IconDeviceFloppy,
  IconInfoCircle,
  IconRefresh,
} from "@tabler/icons-react";
import { api } from "../api";
import type { InstanceSettings } from "../api";

// 空间从未启动过时文件还不存在，给个可直接编辑的骨架而不是空白
const EMPTY_DOC = "{\n}\n";

type Status = { type: "info" | "success" | "error"; msg: string } | null;

export default function InstanceSettingsCard({ name }: { name: string }) {
  const [data, setData] = useState<InstanceSettings | null>(null);
  const [draft, setDraft] = useState("");
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState<"toggle" | "save" | null>(null);
  const [status, setStatus] = useState<Status>(null);

  const load = useCallback(() => {
    api
      .readInstanceSettings(name)
      .then((s) => {
        setData(s);
        setDraft(s.content.trim() ? s.content : EMPTY_DOC);
        setStatus(null);
      })
      .catch((e) => setStatus({ type: "error", msg: String(e) }));
  }, [name]);

  // 切换空间时重置为收起状态，避免把上一个空间的草稿带过来
  useEffect(() => {
    setExpanded(false);
    setStatus(null);
    load();
  }, [load]);

  const onToggle = async (enabled: boolean) => {
    setBusy("toggle");
    try {
      const s = await api.setBypassPermissions(name, enabled);
      setData(s);
      setDraft(s.content.trim() ? s.content : EMPTY_DOC);
      setStatus({
        type: "success",
        msg: enabled
          ? "已写入 permissions.defaultMode = bypassPermissions，新开的会话默认跳过权限确认。"
          : "已移除 permissions.defaultMode，回到默认的逐次确认。",
      });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusy(null);
    }
  };

  const onSave = async () => {
    setBusy("save");
    try {
      const s = await api.writeInstanceSettings(name, draft, data?.revision ?? 0);
      setData(s);
      setDraft(s.content.trim() ? s.content : EMPTY_DOC);
      setStatus({ type: "success", msg: "已保存，上一版留在同目录的 settings.json.bak。" });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusy(null);
    }
  };

  // 纯前端预检：让语法错误在点保存之前就显形
  let draftError = "";
  try {
    const parsed = JSON.parse(draft);
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      draftError = "顶层必须是一个 JSON 对象";
    }
  } catch (e) {
    draftError = e instanceof Error ? e.message : String(e);
  }

  const dirty = data != null && draft !== (data.content.trim() ? data.content : EMPTY_DOC);

  return (
    <Stack gap="xs">
      <Group justify="space-between" align="flex-start" wrap="nowrap">
        <Box>
          <Text size="sm" fw={500}>
            跳过权限确认（bypassPermissions）
          </Text>
          <Text size="xs" c="dimmed">
            只对这个空间生效。开启后该空间的 Claude 不再逐次询问工具权限，等价于每次都加{" "}
            <Code>--dangerously-skip-permissions</Code>，但状态持久、可随时关闭。
          </Text>
        </Box>
        <Switch
          checked={!!data?.bypassEnabled}
          onChange={(e) => onToggle(e.currentTarget.checked)}
          disabled={busy !== null || data == null}
        />
      </Group>

      {data?.bypassEnabled && (
        <Alert variant="light" color="orange" icon={<IconAlertTriangle size={16} />}>
          <Text size="xs">
            该空间内的文件修改、命令执行不再二次确认。建议只在你信任的项目里长期开启。
          </Text>
        </Alert>
      )}

      {data?.overriddenBy && (
        <Alert variant="light" color="yellow" icon={<IconAlertTriangle size={16} />}>
          <Text size="xs">
            检测到 <Code>{data.overriddenBy}</Code> 也设置了 <Code>permissions.defaultMode</Code>。
            它的优先级高于本空间配置，在 home 目录下启动时本开关不会生效。
            项目自己的 <Code>.claude/settings.json</Code> 同理会覆盖这里。
          </Text>
        </Alert>
      )}

      <Group gap="xs">
        <Button
          size="xs"
          variant="subtle"
          leftSection={
            expanded ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />
          }
          onClick={() => setExpanded((v) => !v)}
        >
          手动编辑 settings.json
        </Button>
        {data && !data.exists && (
          <Badge size="xs" variant="light" color="gray">
            文件尚未创建
          </Badge>
        )}
      </Group>

      <Collapse in={expanded}>
        <Stack gap="xs">
          {data && (
            <Text size="xs" c="dimmed">
              <Code>{data.path}</Code>
            </Text>
          )}
          <Textarea
            value={draft}
            onChange={(e) => setDraft(e.currentTarget.value)}
            autosize
            minRows={8}
            maxRows={24}
            spellCheck={false}
            styles={{ input: { fontFamily: "var(--mantine-font-family-monospace)" } }}
            error={draftError || undefined}
          />
          <Alert variant="light" color="cyan" icon={<IconInfoCircle size={16} />}>
            <Text size="xs">
              后台同步会改写这个文件的 <Code>enabledPlugins</Code> 字段（其余内容原样保留）。
              若保存时提示已被修改，点「重新加载」取回最新内容再改。
            </Text>
          </Alert>
          <Group gap="xs">
            <Button
              size="xs"
              leftSection={<IconDeviceFloppy size={14} />}
              onClick={onSave}
              loading={busy === "save"}
              disabled={!!draftError || !dirty}
            >
              保存
            </Button>
            <Button
              size="xs"
              variant="light"
              leftSection={<IconRefresh size={14} />}
              onClick={load}
              disabled={busy !== null}
            >
              重新加载
            </Button>
          </Group>
        </Stack>
      </Collapse>

      {status && (
        <Alert
          variant="light"
          color={status.type === "error" ? "red" : status.type === "success" ? "teal" : "blue"}
          icon={<IconInfoCircle size={16} />}
        >
          <Text size="xs">{status.msg}</Text>
        </Alert>
      )}
    </Stack>
  );
}
