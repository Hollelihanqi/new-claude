// 新增/编辑/复制 MCP 服务的抽屉：表单字段 + 原始 JSON 同步 + 基础测试。
// 凭据边界（方案 5.1）：已有敏感值不绑定到 input；空值=保留，输入新值=替换，显式移除才删除。
// 原始 JSON 始终只展示脱敏占位；transport 跨类切换前确认；unknown 仅经原始 JSON 编辑。
// env/headers 行提升为独立 state：输入期间不反向重新生成，避免清空/锁死。
import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Badge,
  Box,
  Button,
  Drawer,
  Fieldset,
  Group,
  PasswordInput,
  Select,
  Stack,
  Switch,
  Tabs,
  Text,
  TextInput,
  Textarea,
  Title,
} from "@mantine/core";
import { IconAlertTriangle, IconCheck, IconTrash } from "@tabler/icons-react";
import type {
  McpChangeAction,
  McpLocator,
  McpScope,
  McpService,
  McpState,
  McpTestResult,
} from "../../api";
import {
  SCOPE_DESCRIPTIONS,
  SCOPE_LABELS,
  TRANSPORT_LABELS,
  inferTransport,
  redactConfig,
  restoreRedactedValues,
  sensitivePathsForConfig,
  validateLocatorForScope,
} from "./mcpForm";

const RESERVED_NAMES = [
  "workspace",
  "claude-in-chrome",
  "computer-use",
  "Claude Preview",
  "Claude Browser",
];
const NAME_RE = /^[A-Za-z0-9._-]+$/;

interface Props {
  opened: boolean;
  mode: "create" | "edit" | "copy";
  service?: McpService;
  state: McpState;
  onClose: () => void;
  onSave: (action: McpChangeAction) => Promise<void>;
  onTest: (name: string, config: Record<string, unknown>) => Promise<McpTestResult>;
}

type TransportChoice = "stdio" | "http" | "sse" | "ws" | "unknown";
type RowState = "keep" | "modified" | "removed";
interface KvRow {
  originalKey: string;
  key: string;
  value: string;
  state: RowState;
}

function isRemote(t: TransportChoice) {
  return t === "http" || t === "sse" || t === "ws";
}
function categoryOf(t: TransportChoice): "stdio" | "remote" | "unknown" {
  if (t === "stdio") return "stdio";
  if (t === "unknown") return "unknown";
  return "remote";
}
function cloneConfig(c: Record<string, unknown>): Record<string, unknown> {
  return JSON.parse(JSON.stringify(c));
}

export default function McpServiceDrawer({
  opened,
  mode,
  service,
  state,
  onClose,
  onSave,
  onTest,
}: Props) {
  const [name, setName] = useState("");
  const [scope, setScope] = useState<McpScope>("user");
  const [instanceId, setInstanceId] = useState("");
  const [projectPath, setProjectPath] = useState("");
  const [transport, setTransport] = useState<TransportChoice>("stdio");
  const [rawConfig, setRawConfig] = useState<Record<string, unknown>>({});
  // env/headers 行独立 state：输入期间不反向重新生成
  const [envRows, setEnvRows] = useState<KvRow[]>([]);
  const [headerRows, setHeaderRows] = useState<KvRow[]>([]);
  const [rawText, setRawText] = useState("");
  const [rawSensitivePaths, setRawSensitivePaths] = useState<string[]>([]);
  const [tab, setTab] = useState<"form" | "json">("form");
  const [testResult, setTestResult] = useState<McpTestResult | null>(null);
  const [err, setErr] = useState("");
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  const sensitivePaths = service?.sensitivePaths ?? [];

  function currentSensitivePaths(config: Record<string, unknown>): string[] {
    return Array.from(new Set([...sensitivePaths, ...sensitivePathsForConfig(config)]));
  }

  function clearDrafts() {
    setRawConfig({});
    setEnvRows([]);
    setHeaderRows([]);
    setRawText("");
    setRawSensitivePaths([]);
    setTestResult(null);
    setErr("");
  }

  useEffect(() => {
    if (!opened) return;
    setErr("");
    setTestResult(null);
    setTab("form");
    if (mode === "create" || !service) {
      setName("");
      setScope("user");
      setInstanceId("");
      setProjectPath("");
      setTransport("stdio");
      const cfg: Record<string, unknown> = { type: "stdio", command: "", args: [] as string[] };
      setRawConfig(cfg);
      setEnvRows([]);
      setHeaderRows([]);
      setRawText(JSON.stringify(cfg, null, 2));
      setRawSensitivePaths([]);
      return;
    }
    const cfg = cloneConfig(service.config);
    setName(mode === "copy" ? `${service.locator.name}-copy` : service.locator.name);
    setScope(service.locator.scope);
    setInstanceId(service.locator.instanceId ?? "");
    setProjectPath(service.locator.projectPath ?? "");
    setTransport(inferTransport(cfg) as TransportChoice);
    setRawConfig(cfg);
    setEnvRows(toKvRows(cfg.env));
    setHeaderRows(toKvRows(cfg.headers));
    const paths = Array.from(
      new Set([...service.sensitivePaths, ...sensitivePathsForConfig(cfg)])
    );
    setRawText(JSON.stringify(redactConfig(cfg, paths), null, 2));
    setRawSensitivePaths(paths);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opened, mode, service]);

  if (!opened) return null;

  function patch(fn: (c: Record<string, unknown>) => void) {
    setTestResult(null);
    setRawConfig((prev) => {
      const next = cloneConfig(prev);
      fn(next);
      return next;
    });
  }

  function changeTransport(next: TransportChoice) {
    if (next === transport) return;
    if (next === "unknown") {
      // 切到 unknown：保留完整配置，切到原始 JSON 页签，不清理字段、不显示虚假清理提示
      setTestResult(null);
      setTransport("unknown");
      setTab("json");
      try {
        const config = materializeConfig();
        const paths = currentSensitivePaths(config);
        setRawText(JSON.stringify(redactConfig(config, paths), null, 2));
        setRawSensitivePaths(paths);
      } catch (e) {
        setErr(String(e));
      }
      return;
    }
    // 从 unknown 或已知类型切到已知类型：跨类别时确认并一致清理目标类型不兼容字段
    if (categoryOf(transport) !== categoryOf(next)) {
      const ok = window.confirm(
        `切换 transport 将清理 ${next === "stdio" ? "url/headers" : "command/args/env"} 字段，确认继续？`
      );
      if (!ok) return;
    }
    setTestResult(null);
    setTransport(next);
    setEnvRows((prev) => (isRemote(next) ? [] : prev));
    setHeaderRows((prev) => (next === "stdio" ? [] : prev));
    patch((c) => {
      if (isRemote(next)) {
        delete c.command;
        delete c.args;
        delete c.env;
        c.type = next === "http" ? "http" : next;
      } else if (next === "stdio") {
        delete c.url;
        delete c.headers;
        c.type = "stdio";
        if (typeof c.command !== "string") c.command = "";
        if (!Array.isArray(c.args)) c.args = [] as string[];
      }
    });
  }

  // 材质化：克隆 rawConfig 并应用 env/headers 行状态，得到完整当前配置。
  function materializeConfig(): Record<string, unknown> {
    const c = cloneConfig(rawConfig);
    applyKv(c, "env", envRows);
    applyKv(c, "headers", headerRows);
    return c;
  }

  // 切到 JSON 页签：基于 materializeConfig 重新生成脱敏文本
  function switchTab(v: "form" | "json") {
    if (v === "json") {
      try {
        const m = materializeConfig();
        const paths = currentSensitivePaths(m);
        setRawText(JSON.stringify(redactConfig(m, paths), null, 2));
        setRawSensitivePaths(paths);
        setTab("json");
      } catch (e) {
        setErr(String(e));
      }
      return;
    }
    // form ← json：必须解析 rawText 并应用，解析失败则阻止切换（留在 json）
    try {
      const restored = parseRawText();
      setRawConfig(restored);
      setEnvRows(toKvRows(restored.env));
      setHeaderRows(toKvRows(restored.headers));
      setTransport(inferTransport(restored) as TransportChoice);
      setErr("");
      setTab("form");
    } catch (e) {
      setErr(String(e));
    }
  }

  const argsText = useMemo(() => {
    const a = rawConfig.args;
    return Array.isArray(a) ? a.map(String).join("\n") : "";
  }, [rawConfig.args]);

  function setArgsText(t: string) {
    patch((c) => {
      c.args = t.split("\n");
    });
  }

  // keep/modified/removed 行状态应用到 config 子对象，保留原值与未知键
  function applyKv(c: Record<string, unknown>, field: string, rows: KvRow[]) {
    const existing =
      c[field] && typeof c[field] === "object" && !Array.isArray(c[field])
        ? { ...(c[field] as Record<string, unknown>) }
        : {};
    for (const r of rows) {
      if (r.state === "removed") {
        delete existing[r.originalKey];
      } else if (r.state === "modified") {
        if (r.key.trim() !== "") {
          existing[r.key.trim()] = r.value;
        }
      }
    }
    if (Object.keys(existing).length > 0) {
      c[field] = existing;
    } else {
      delete c[field];
    }
  }

  function locatorErrs() {
    return validateLocatorForScope({
      scope,
      name,
      instanceId: instanceId || undefined,
      projectPath: projectPath || undefined,
    });
  }

  function nameErr(): string {
    const trimmed = name.trim();
    if (!trimmed) return "名称必填";
    // 历史非法名/保留名：mode=edit 且名称未改变时允许保留（后端 grandfather 可用）
    if (mode === "edit" && service && trimmed === service.locator.name) {
      return "";
    }
    if (trimmed.length > 64) return "名称不超过 64 字符";
    if (!NAME_RE.test(trimmed)) return "仅允许字母、数字、. _ -";
    if (RESERVED_NAMES.includes(trimmed)) return "该名称为保留名";
    return "";
  }

  // 解析原始 JSON 文本并还原脱敏值，previousRaw 取当前 materializeConfig（保留表单中的用户编辑）
  function parseRawText(): Record<string, unknown> {
    let parsed: unknown;
    try {
      parsed = JSON.parse(rawText);
    } catch (e) {
      throw new Error(`JSON 解析失败：${String(e)}`);
    }
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new Error("配置顶层必须是对象");
    }
    return restoreRedactedValues(
      parsed as Record<string, unknown>,
      materializeConfig(),
      rawSensitivePaths
    );
  }

  function effectiveConfig(): Record<string, unknown> | null {
    try {
      if (tab === "json") return parseRawText();
      return materializeConfig();
    } catch (e) {
      setErr(String(e));
      return null;
    }
  }

  async function handleTest() {
    setTesting(true);
    setErr("");
    const cfg = effectiveConfig();
    if (!cfg) {
      setTesting(false);
      return;
    }
    try {
      const result = await onTest(name.trim() || "preview", cloneConfig(cfg));
      setTestResult(result);
    } catch (e) {
      setErr(String(e));
    } finally {
      setTesting(false);
    }
  }

  async function handleSave() {
    const ne = nameErr();
    const le = locatorErrs();
    if (ne || Object.keys(le).length > 0) {
      setErr(ne || Object.values(le).join("；"));
      return;
    }
    const cfg = effectiveConfig();
    if (!cfg) return;
    setSaving(true);
    setErr("");
    try {
      const target: McpLocator = {
        scope,
        name: name.trim(),
        instanceId: scope === "local" ? instanceId || undefined : undefined,
        projectPath: scope === "local" || scope === "project" ? projectPath || undefined : undefined,
      };
      let original: McpLocator | undefined;
      if (mode === "edit" && service) {
        const s = service.locator;
        original = { scope: s.scope, name: s.name, instanceId: s.instanceId, projectPath: s.projectPath };
      }
      const action: McpChangeAction = {
        op: "save",
        original,
        target,
        config: cloneConfig(cfg),
        overwrite: false,
      };
      await onSave(action);
    } catch (e) {
      setErr(String(e));
    } finally {
      setSaving(false);
    }
  }

  function handleClose() {
    clearDrafts();
    onClose();
  }

  return (
    <Drawer
      opened={opened}
      onClose={handleClose}
      position="right"
      size="xl"
      title={
        <Group gap="sm">
          <Title order={4}>
            {mode === "create" ? "新建 MCP 服务" : mode === "copy" ? "复制 MCP 服务" : "编辑 MCP 服务"}
          </Title>
          {service && mode !== "create" && (
            <Badge color="gray" variant="light">{service.sourceId}</Badge>
          )}
        </Group>
      }
      padding="md"
    >
      <Stack gap="md">
        <Tabs value={tab} onChange={(v) => v && switchTab(v as "form" | "json")}>
          <Tabs.List>
            <Tabs.Tab value="form">表单</Tabs.Tab>
            <Tabs.Tab value="json">原始 JSON</Tabs.Tab>
          </Tabs.List>

          <Box pt="md">
            {tab === "form" ? (
              <Stack gap="md">
                <Fieldset legend="基本信息">
                  <div className="mcp-form-grid">
                    <TextInput
                      label="服务名称"
                      value={name}
                      onChange={(e) => {
                        setName(e.currentTarget.value);
                        setTestResult(null);
                      }}
                      error={nameErr() || undefined}
                      placeholder="如 github、filesystem"
                    />
                    <Select
                      label="作用域"
                      value={scope}
                      onChange={(v) => {
                        if (v) {
                          setScope(v as McpScope);
                          setTestResult(null);
                        }
                      }}
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
                        onChange={(v) => {
                          setInstanceId(v ?? "");
                          setTestResult(null);
                        }}
                        data={state.instances.map((i) => ({ value: i.id, label: i.label }))}
                        error={locatorErrs().instanceId}
                        placeholder="选择 Claude 实例"
                      />
                    )}
                    {(scope === "local" || scope === "project") && (
                      <Select
                        label="项目"
                        value={projectPath}
                        onChange={(v) => {
                          setProjectPath(v ?? "");
                          setTestResult(null);
                        }}
                        data={state.projects.map((p) => ({ value: p.path, label: p.label }))}
                        error={locatorErrs().projectPath}
                        placeholder="选择项目目录"
                      />
                    )}
                  </div>
                  <Text size="xs" c="dimmed" mt="xs">{SCOPE_DESCRIPTIONS[scope]}</Text>
                </Fieldset>

                <Fieldset legend="连接方式">
                  <Select
                    label="类型"
                    value={transport}
                    onChange={(v) => v && changeTransport(v as TransportChoice)}
                    data={(["stdio", "http", "sse", "ws", "unknown"] as TransportChoice[]).map(
                      (t) => ({ value: t, label: TRANSPORT_LABELS[t] })
                    )}
                  />
                  {transport === "unknown" ? (
                    <Alert color="orange" icon={<IconAlertTriangle size={16} />} mt="sm">
                      未知类型无法用表单编辑，请切换到「原始 JSON」页签修改并保存。
                    </Alert>
                  ) : transport === "stdio" ? (
                    <Stack gap="sm" mt="sm">
                      <TextInput
                        label="command"
                        value={(rawConfig.command as string) ?? ""}
                        onChange={(e) => patch((c) => { c.command = e.currentTarget.value; })}
                        placeholder="如 npx、node、/usr/local/bin/mcp"
                      />
                      <Textarea
                        label="args（每行一个）"
                        value={argsText}
                        onChange={(e) => setArgsText(e.currentTarget.value)}
                        autosize
                        minRows={2}
                      />
                      <KvEditor label="env" rows={envRows} onChange={setEnvRows} />
                    </Stack>
                  ) : (
                    <Stack gap="sm" mt="sm">
                      <TextInput
                        label="url"
                        value={(rawConfig.url as string) ?? ""}
                        onChange={(e) => patch((c) => { c.url = e.currentTarget.value; })}
                        placeholder="https://example.com/mcp"
                      />
                      <KvEditor label="headers" rows={headerRows} onChange={setHeaderRows} />
                      {transport === "sse" && (
                        <Text size="xs" c="orange">SSE 已弃用，服务支持时请迁移到 HTTP。</Text>
                      )}
                    </Stack>
                  )}
                </Fieldset>

                <Fieldset legend="高级">
                  <div className="mcp-form-grid">
                    <TextInput
                      label="timeout（可选，毫秒，>=1000）"
                      value={rawConfig.timeout !== undefined ? String(rawConfig.timeout) : ""}
                      onChange={(e) => {
                        const v = e.currentTarget.value.trim();
                        patch((c) => {
                          if (v === "") delete c.timeout;
                          else c.timeout = Number(v);
                        });
                      }}
                      placeholder="留空使用默认"
                    />
                    <Switch
                      label="alwaysLoad"
                      checked={rawConfig.alwaysLoad === true}
                      onChange={(e) =>
                        patch((c) => {
                          if (e.currentTarget.checked) c.alwaysLoad = true;
                          else delete c.alwaysLoad;
                        })
                      }
                    />
                  </div>
                </Fieldset>
              </Stack>
            ) : (
              <Stack gap="sm">
                <Textarea
                  label="配置对象（顶层必须是对象；凭据以脱敏占位显示）"
                  value={rawText}
                  onChange={(e) => {
                    setRawText(e.currentTarget.value);
                    setTestResult(null);
                  }}
                  autosize
                  minRows={12}
                  styles={{ input: { fontFamily: "monospace", fontSize: 12 } }}
                />
                <Text size="xs" c="dimmed">
                  原始 JSON 只展示脱敏占位；切回表单时会解析并还原未修改的凭据，未知字段保留。解析失败将阻止切回表单。
                </Text>
              </Stack>
            )}
          </Box>
        </Tabs>

        {err && <Alert color="red" icon={<IconAlertTriangle size={16} />}>{err}</Alert>}

        {testResult && (
          <Alert
            color={testResult.ok ? "teal" : "orange"}
            icon={<IconCheck size={16} />}
            title={testResult.ok ? "基础测试通过" : "基础测试发现问题"}
          >
            <Stack gap={4}>
              {testResult.stages.map((s, i) => (
                <Text key={i} size="xs">
                  <Badge color={stageColor(s.status)} mr={6}>{s.id}</Badge>
                  {s.detail}
                </Text>
              ))}
            </Stack>
          </Alert>
        )}

        <Group justify="space-between" mt="sm">
          <Button variant="default" loading={testing} onClick={handleTest}>基础测试</Button>
          <Group>
            <Button variant="subtle" onClick={handleClose}>取消</Button>
            <Button loading={saving} onClick={handleSave}>保存</Button>
          </Group>
        </Group>
      </Stack>
    </Drawer>
  );
}

function stageColor(status: string): string {
  if (status === "ok") return "teal";
  if (status === "warn") return "orange";
  if (status === "fail") return "red";
  return "gray";
}

function toKvRows(v: unknown): KvRow[] {
  if (v && typeof v === "object" && !Array.isArray(v)) {
    return Object.keys(v as Record<string, unknown>).map((key) => ({
      originalKey: key,
      key,
      value: "",
      state: "keep" as RowState,
    }));
  }
  return [];
}

function KvEditor({
  label,
  rows,
  onChange,
}: {
  label: string;
  rows: KvRow[];
  onChange: (rows: KvRow[]) => void;
}) {
  // 本地副本仅用于即时输入反馈；onChange 把变更回写给父 state（独立行 state）。
  const [local, setLocal] = useState<KvRow[]>(rows);
  useEffect(() => setLocal(rows), [rows]);
  function update(i: number, p: Partial<KvRow>) {
    const next = local.map((r, idx) => (idx === i ? { ...r, ...p } : r));
    setLocal(next);
    onChange(next);
  }
  return (
    <Box>
      <Text size="sm" fw={500} mb={4}>{label}</Text>
      <Stack gap={6}>
        {local.map((r, i) => {
          const keyReadonly = r.originalKey !== "";
          const placeholder =
            r.state === "keep" ? "已保存，留空不修改" : r.state === "removed" ? "已移除" : "新值";
          return (
            <div className="mcp-kv-row" key={i} style={r.state === "removed" ? { opacity: 0.5 } : undefined}>
              <TextInput
                value={r.key}
                onChange={(e) => update(i, { key: e.currentTarget.value })}
                placeholder="键"
                disabled={keyReadonly || r.state === "removed"}
              />
              <PasswordInput
                value={r.state === "removed" ? "" : r.value}
                onChange={(e) => update(i, { value: e.currentTarget.value, state: "modified" })}
                placeholder={placeholder}
                disabled={r.state === "removed"}
              />
              <Button
                variant="subtle"
                color="red"
                leftSection={<IconTrash size={14} />}
                onClick={() =>
                  update(i, {
                    state: r.state === "removed" ? "keep" : "removed",
                    value: "",
                  })
                }
              >
                {r.state === "removed" ? "恢复" : "移除"}
              </Button>
            </div>
          );
        })}
        <Button
          variant="light"
          size="xs"
          onClick={() => {
            const next = [...local, { originalKey: "", key: "", value: "", state: "modified" as RowState }];
            setLocal(next);
            onChange(next);
          }}
        >
          添加一行
        </Button>
      </Stack>
    </Box>
  );
}
