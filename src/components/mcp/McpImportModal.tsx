// 粘贴 MCP JSON 导入：支持单个服务对象或完整 mcpServers 对象。
// 选择语义：include 始终可操作；既有默认不选；选中既有须显式勾选覆盖才进 batch；
// 单对象也须显式确认覆盖；action.overwrite 取用户的 overwrite 标记，不取 it.exists。
// 关闭立即清空 text/items/选择/错误，不残留凭据草稿。
import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Badge,
  Box,
  Button,
  Checkbox,
  Group,
  Modal,
  Select,
  Stack,
  Text,
  Textarea,
  Title,
} from "@mantine/core";
import { IconAlertTriangle } from "@tabler/icons-react";
import type { McpChangeAction, McpScope, McpState } from "../../api";
import { SCOPE_DESCRIPTIONS, SCOPE_LABELS, inferTransport, TRANSPORT_LABELS } from "./mcpForm";

const MAX_BYTES = 1024 * 1024; // 1 MiB
const MAX_ITEMS = 100;

interface ParsedItem {
  name: string;
  config: Record<string, unknown>;
}
interface EffItem {
  name: string;
  config: Record<string, unknown>;
  exists: boolean;
  overwrite: boolean;
}

interface Props {
  opened: boolean;
  state: McpState;
  onClose: () => void;
  onSave: (action: McpChangeAction) => Promise<void>;
}

export default function McpImportModal({ opened, state, onClose, onSave }: Props) {
  const [text, setText] = useState("");
  const [items, setItems] = useState<ParsedItem[]>([]);
  const [parseSource, setParseSource] = useState<"single" | "multi">("multi");
  const [singleName, setSingleName] = useState("");
  const [singleSelected, setSingleSelected] = useState(true);
  const [singleOverwrite, setSingleOverwrite] = useState(false);
  const [scope, setScope] = useState<McpScope>("user");
  const [instanceId, setInstanceId] = useState("");
  const [projectPath, setProjectPath] = useState("");
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [overwrite, setOverwrite] = useState<Record<string, boolean>>({});
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  function clearAll() {
    setText("");
    setItems([]);
    setParseSource("multi");
    setSingleName("");
    setSingleSelected(true);
    setSingleOverwrite(false);
    setScope("user");
    setInstanceId("");
    setProjectPath("");
    setSelected({});
    setOverwrite({});
    setErr("");
    setBusy(false);
  }

  // 打开与关闭都清理：关闭时立即清空粘贴原文与凭据，不残留在挂载组件内存
  useEffect(() => {
    clearAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opened]);

  function handleClose() {
    clearAll();
    onClose();
  }

  function existingAt(name: string): boolean {
    if (!name) return false;
    return state.services.some(
      (s) =>
        s.locator.name === name &&
        s.locator.scope === scope &&
        (s.locator.instanceId ?? "") === (scope === "local" ? instanceId : "") &&
        (s.locator.projectPath ?? "") ===
          (scope === "local" || scope === "project" ? projectPath : "")
    );
  }

  function parse() {
    setErr("");
    setItems([]);
    setSelected({});
    setOverwrite({});
    setSingleSelected(true);
    setSingleOverwrite(false);
    const trimmed = text.trim();
    if (!trimmed) {
      setErr("请粘贴 JSON");
      return;
    }
    if (new Blob([trimmed]).size > MAX_BYTES) {
      setErr("粘贴内容超过 1 MiB 上限");
      return;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(trimmed);
    } catch (e) {
      setErr(`JSON 解析失败：${String(e)}`);
      return;
    }
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      setErr("顶层必须是对象");
      return;
    }
    const obj = parsed as Record<string, unknown>;
    if (obj.mcpServers !== undefined) {
      // 检测到 mcpServers 属性：先验证它是非数组对象
      if (
        typeof obj.mcpServers !== "object" ||
        obj.mcpServers === null ||
        Array.isArray(obj.mcpServers)
      ) {
        setErr("mcpServers 必须是对象");
        return;
      }
      const servers = obj.mcpServers as Record<string, unknown>;
      const keys = Object.keys(servers);
      // 100 项限制统计原始服务数（不是过滤后的数量）
      if (keys.length > MAX_ITEMS) {
        setErr(`服务数量 ${keys.length} 超过 ${MAX_ITEMS} 上限`);
        return;
      }
      const list: ParsedItem[] = [];
      for (const name of keys) {
        const cfg = servers[name];
        // 任一成员不是配置对象：报告 JSON 路径和类型，拒绝整个导入，不静默丢弃
        if (cfg === null || typeof cfg !== "object" || Array.isArray(cfg)) {
          const t = cfg === null ? "null" : Array.isArray(cfg) ? "array" : typeof cfg;
          setErr(`mcpServers["${name}"] 必须是配置对象（当前：${t}）`);
          return;
        }
        list.push({ name, config: cfg as Record<string, unknown> });
      }
      if (list.length === 0) {
        setErr("mcpServers 内没有有效服务");
        return;
      }
      setItems(list);
      setParseSource("multi");
      // 既有默认不选；新服务默认选中
      const sel: Record<string, boolean> = {};
      const ov: Record<string, boolean> = {};
      for (const it of list) {
        sel[it.name] = !existingAt(it.name);
        ov[it.name] = false;
      }
      setSelected(sel);
      setOverwrite(ov);
      return;
    }
    // 单个服务对象：需用户命名
    setItems([{ name: "", config: obj }]);
    setParseSource("single");
    setSingleName("");
    setSingleSelected(true);
    setSingleOverwrite(false);
  }

  const targetLocatorFieldsValid = scope !== "local" || (!!instanceId && !!projectPath);
  const projectValid = scope !== "project" || !!projectPath;

  const effectiveItems: EffItem[] = useMemo(() => {
    if (items.length === 0) return [];
    if (parseSource === "single") {
      const nm = singleName.trim();
      if (!nm || !singleSelected) return [];
      const ex = existingAt(nm);
      if (ex && !singleOverwrite) return [];
      return [{ name: nm, config: items[0].config, exists: ex, overwrite: singleOverwrite }];
    }
    return items
      .map((it) => ({
        name: it.name,
        config: it.config,
        exists: existingAt(it.name),
        overwrite: overwrite[it.name] === true,
      }))
      .filter((it) => selected[it.name] === true && (!it.exists || it.overwrite));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, parseSource, singleName, singleSelected, singleOverwrite, selected, overwrite, scope, instanceId, projectPath]);

  // 所有 Hook 必须在条件返回之前调用，否则关闭后再打开弹窗会改变 Hook 顺序并导致整页崩溃。
  if (!opened) return null;

  async function handleSave() {
    if (effectiveItems.length === 0) {
      setErr("没有可导入的服务（未选择或已存在项未勾选覆盖）");
      return;
    }
    if (!targetLocatorFieldsValid || !projectValid) {
      setErr("请补全目标作用域所需的实例/项目");
      return;
    }
    setBusy(true);
    setErr("");
    try {
      const action: McpChangeAction = {
        op: "batchSave",
        items: effectiveItems.map((it) => ({
          target: {
            scope,
            name: it.name,
            instanceId: scope === "local" ? instanceId || undefined : undefined,
            projectPath: scope === "local" || scope === "project" ? projectPath || undefined : undefined,
          },
          config: JSON.parse(JSON.stringify(it.config)),
          overwrite: it.overwrite,
        })),
      };
      await onSave(action);
      // 只有 preview 成功（onSave 未抛错）后才关闭并清空 Modal，同时清理 busy
      clearAll();
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  // 单对象的当前名称与是否同名
  const singleNm = singleName.trim();
  const singleExists = singleNm ? existingAt(singleNm) : false;

  return (
    <Modal opened={opened} onClose={handleClose} size="lg" title={<Title order={4}>导入 MCP JSON</Title>}>
      <Stack gap="md">
        <Textarea
          label="粘贴 JSON"
          description='支持单个服务对象，或 { "mcpServers": { ... } }'
          value={text}
          onChange={(e) => setText(e.currentTarget.value)}
          autosize
          minRows={6}
          styles={{ input: { fontFamily: "monospace", fontSize: 12 } }}
        />
        <Button variant="light" onClick={parse}>解析预览</Button>

        {err && <Alert color="red" icon={<IconAlertTriangle size={16} />}>{err}</Alert>}

        {items.length > 0 && (
          <>
            <Text size="sm" fw={600}>目标作用域</Text>
            <Group grow align="flex-start">
              <Select
                label="作用域"
                value={scope}
                onChange={(v) => v && setScope(v as McpScope)}
                data={[
                  { value: "user", label: SCOPE_LABELS.user },
                  { value: "local", label: SCOPE_LABELS.local },
                  { value: "project", label: SCOPE_LABELS.project },
                ]}
              />
              {scope === "local" && (
                <Select
                  label="实例"
                  value={instanceId}
                  onChange={(v) => setInstanceId(v ?? "")}
                  data={state.instances.map((i) => ({ value: i.id, label: i.label }))}
                />
              )}
              {(scope === "local" || scope === "project") && (
                <Select
                  label="项目"
                  value={projectPath}
                  onChange={(v) => setProjectPath(v ?? "")}
                  data={state.projects.map((p) => ({ value: p.path, label: p.label }))}
                />
              )}
            </Group>
            <Text size="xs" c="dimmed">{SCOPE_DESCRIPTIONS[scope]}</Text>

            {parseSource === "single" ? (
              <Box>
                <Text size="sm" fw={500} mb={4}>服务名称</Text>
                <input
                  value={singleName}
                  onChange={(e) => setSingleName(e.currentTarget.value)}
                  placeholder="为该服务命名"
                  style={{ width: "100%", padding: "8px 10px", borderRadius: 6, border: "1px solid var(--app-border)" }}
                />
                <Group gap="sm" align="center" mt="sm">
                  <Checkbox
                    checked={singleSelected}
                    onChange={(e) => setSingleSelected(e.currentTarget.checked)}
                    label="导入该项"
                  />
                  {singleExists && (
                    <Checkbox
                      checked={singleOverwrite}
                      onChange={(e) => setSingleOverwrite(e.currentTarget.checked)}
                      label="覆盖同名服务"
                      disabled={!singleSelected}
                    />
                  )}
                </Group>
              </Box>
            ) : (
              <Stack gap={4}>
                <Text size="sm" fw={500}>导入预览（{items.length} 项）</Text>
                {items.map((it) => {
                  const ex = existingAt(it.name);
                  const isSel = selected[it.name] === true;
                  const isOv = overwrite[it.name] === true;
                  return (
                    <Group key={it.name} gap="sm" align="center">
                      <Checkbox
                        checked={isSel}
                        onChange={(e) =>
                          setSelected((s) => ({ ...s, [it.name]: e.currentTarget.checked }))
                        }
                        label={
                          <Group gap={6}>
                            <Text size="sm">{it.name}</Text>
                            <Badge size="xs" variant="light">{TRANSPORT_LABELS[inferTransport(it.config)]}</Badge>
                            {ex && <Badge size="xs" color="orange" variant="light">已存在</Badge>}
                          </Group>
                        }
                      />
                      {ex && (
                        <Checkbox
                          checked={isOv}
                          onChange={(e) =>
                            setOverwrite((o) => ({ ...o, [it.name]: e.currentTarget.checked }))
                          }
                          label="覆盖"
                          size="xs"
                          disabled={!isSel}
                        />
                      )}
                    </Group>
                  );
                })}
              </Stack>
            )}

            <Group justify="flex-end">
              <Button variant="subtle" onClick={handleClose}>取消</Button>
              <Button loading={busy} onClick={handleSave}>
                导入 {effectiveItems.length} 项
              </Button>
            </Group>
          </>
        )}
      </Stack>
    </Modal>
  );
}
