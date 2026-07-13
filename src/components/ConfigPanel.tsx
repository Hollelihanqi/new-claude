import { useEffect, useMemo, useState } from "react";
import {
  Card,
  Stack,
  Group,
  Button,
  TextInput,
  PasswordInput,
  Select,
  Text,
  Title,
  NavLink,
  Badge,
  Code,
  Alert,
  Box,
  Autocomplete,
  Modal,
} from "@mantine/core";
import {
  IconPlus,
  IconTrash,
  IconDeviceFloppy,
  IconWorld,
  IconUser,
  IconInfoCircle,
  IconListSearch,
  IconAlertTriangle,
} from "@tabler/icons-react";
import { api } from "../api";
import type { EnvInfo, Profile, ModelPinWarning, ProfileRuntimeInfo, UsageStats } from "../api";

// 文档里列出的常用模型别名，作为下拉候选（检测到的真实模型会合并进来）
const PRESET_MODELS = [
  "claude-sonnet-4-6",
  "claude-opus-4-7",
  "claude-haiku-4-5",
  "deepseek-v4-pro",
  "deepseek-v4-flash",
  "glm-5.2",
  "glm-5.1",
  "glm-5",
  "glm-5-turbo",
  "claude-zhipu-5.2",
  "kimi-k2.7-code",
  "kimi-k2.6",
  "minimax-m2.7",
  "qwen3.7-max",
  "qwen3.7-plus",
  "qwen3.6-flash",
  "claude-qw3.7-max",
  "claude-qw3.6-plus",
];

const empty: FormState = {
  name: "",
  type: "router",
  baseUrl: "",
  opusModel: "",
  sonnetModel: "",
  haikuModel: "",
};

const isCertError = (s: string) =>
  /cert|ssl|self.?signed|证书|signature/i.test(String(s));

type StatusType = "info" | "error" | "success";

interface FormState {
  name: string;
  type: Profile["type"];
  baseUrl: string;
  opusModel: string;
  sonnetModel: string;
  haikuModel: string;
}

export default function ConfigPanel({
  onChanged,
  env,
  usageData,
}: {
  onChanged?: () => void;
  env: EnvInfo | null;
  usageData: UsageStats | null;
}) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [sel, setSel] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(empty);
  const [token, setToken] = useState("");
  const [status, setStatus] = useState<{ type: StatusType; msg: string }>({
    type: "info",
    msg: "",
  });
  const [busyAction, setBusyAction] = useState("");
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [runtime, setRuntime] = useState<ProfileRuntimeInfo[]>([]);

  // 检测到的当前实例模型（合并进下方下拉）
  const [detected, setDetected] = useState<string[]>([]);
  const [detectBusy, setDetectBusy] = useState(false);

  // /model 钉死具体型号的告警（含主账户 __main__）
  const [pins, setPins] = useState<ModelPinWarning[]>([]);
  const [fixBusy, setFixBusy] = useState("");

  const loadPins = () => {
    api.modelPinWarnings().then((ws) => setPins(ws || [])).catch(() => {});
  };

  const load = () => {
    api
      .listProfiles()
      .then((ps) => setProfiles(ps || []))
      .catch((e) => setStatus({ type: "error", msg: String(e) }));
    loadPins();
    api.profileRuntimeInfo().then(setRuntime).catch(() => {});
  };
  useEffect(load, []);

  const pinLabel = (profile: string) =>
    profile === "__main__" ? "主账户" : `实例 ${profile}`;

  const onFixPin = async (profile: string) => {
    setFixBusy(profile);
    try {
      const m = await api.fixModelPin(profile);
      setStatus({ type: "success", msg: `${pinLabel(profile)}：${m}` });
      loadPins();
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setFixBusy("");
    }
  };

  const pickProfile = (p: Profile) => {
    setSel(p.name);
    setForm({
      name: p.name,
      type: p.type,
      baseUrl: p.baseUrl || "",
      opusModel: p.opusModel || "",
      sonnetModel: p.sonnetModel || "",
      haikuModel: p.haikuModel || "",
    });
    setToken("");
    setDetected([]);
    // 提示条是页面级共享状态，切实例必须清掉，否则上一个实例的报错会"跟着"过来
    setStatus({ type: "info", msg: "" });
  };

  const onNew = () => {
    setSel(null);
    setForm(empty);
    setToken("");
    setDetected([]);
    setStatus({ type: "info", msg: "填写名称（即命令词，如 bj）后保存。" });
  };

  const valid = () => {
    const n = form.name.trim();
    if (!n) return "请填写实例名称。";
    // 编辑已有实例：名称不可改（输入框已禁用），旧规则时代的名字放行，只校验新建
    if (sel) return null;
    if (!/^[A-Za-z0-9_-]{1,40}$/.test(n))
      return "名称只能包含英文字母、数字、下划线、短横线（1~40 个字符）。";
    if (n.startsWith("__")) return "名称不能以 __ 开头（内部保留前缀）。";
    if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i.test(n))
      return "该名称是 Windows 保留设备名，请换一个。";
    if (form.type === "router") {
      const url = form.baseUrl.trim();
      if (!/^https?:\/\/\S+$/i.test(url) || url.length > 2048)
        return "网关地址必须以 http:// 或 https:// 开头，且不能包含空格。";
    }
    return null;
  };

  // 模型下拉候选 = 检测到的 + 预设，去重
  const modelOpts = useMemo(
    () => Array.from(new Set([...detected, ...PRESET_MODELS])),
    [detected]
  );

  const onDetect = async () => {
    setDetectBusy(true);
    try {
      let list;
      if (sel) {
        // 编辑已存在实例：用后端存的 key
        list = await api.detectModelsFor(sel);
      } else {
        // 新建未保存：用表单里填的 baseUrl + key
        if (!form.baseUrl.trim()) {
          setStatus({ type: "error", msg: "请先填写网关地址再检测。" });
          return;
        }
        if (!token.trim()) {
          setStatus({ type: "error", msg: "新建实例需先填 API Key 才能检测模型。" });
          return;
        }
        list = await api.detectModels(form.baseUrl.trim(), token.trim());
      }
      setDetected(list || []);
      setStatus({
        type: "success",
        msg: `检测到 ${(list || []).length} 个可用模型，已加入下方下拉。`,
      });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setDetectBusy(false);
    }
  };

  const onSave = async () => {
    const v = valid();
    if (v) {
      setStatus({ type: "error", msg: v });
      return;
    }
    setBusyAction("save");
    try {
      const profile = {
        name: form.name.trim(),
        type: form.type,
        baseUrl: form.type === "router" ? form.baseUrl.trim() : "",
        opusModel: form.opusModel.trim(),
        sonnetModel: form.sonnetModel.trim(),
        haikuModel: form.haikuModel.trim(),
      };
      const msg = await api.saveProfile(profile, token || null);
      setToken("");
      load();
      onChanged && onChanged();
      setSel(profile.name);
      setStatus({
        type: "success",
        msg: `已保存「${profile.name}」。${msg} 之后在新终端里运行：claude ${profile.name}`,
      });
    } catch (e) {
      const m = String(e);
      setStatus({
        type: "error",
        msg: isCertError(m)
          ? "保存出错，疑似证书问题。可在右上角「CA 证书」导入证书后重试。原始错误：" + m
          : m,
      });
    } finally {
      setBusyAction("");
    }
  };

  const onDelete = async (purgeData: boolean) => {
    if (!sel) {
      setStatus({ type: "error", msg: "请先在左侧选中一个实例。" });
      return;
    }
    setBusyAction(purgeData ? "purge" : "remove");
    try {
      const msg = await api.deleteProfile(sel, purgeData);
      setDeleteOpen(false);
      onNew();
      load();
      onChanged && onChanged();
      setStatus({ type: "success", msg: `${msg} 已更新终端集成，重开终端生效。` });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusyAction("");
    }
  };

  const selProfile = profiles.find((p) => p.name === sel);
  const selRuntime = runtime.find((item) => item.name === sel);
  const isRouter = form.type === "router";

  const usageFor = (name: string) => {
    const today = new Date().toLocaleDateString("en-CA");
    const rows = (usageData?.daily || []).filter((row) => {
      if (row.profile !== name || row.datetime.length < 13) return false;
      return new Date(`${row.datetime}:00:00Z`).toLocaleDateString("en-CA") === today;
    });
    return rows.reduce((sum, row) => ({ requests: sum.requests + row.requests, tokens: sum.tokens + row.input + row.output + row.cacheRead + row.cacheCreate }), { requests: 0, tokens: 0 });
  };
  const selectedUsage = sel ? usageFor(sel) : { requests: 0, tokens: 0 };
  const mainUsage = usageFor("__main__");
  const fmtNumber = (value: number) => value >= 1_000_000 ? `${(value / 1_000_000).toFixed(1)}M` : value >= 1_000 ? `${(value / 1_000).toFixed(1)}K` : String(value);
  const formatLastUsed = (seconds?: number) => {
    if (!seconds) return "尚未使用";
    const delta = Math.max(0, Date.now() - seconds * 1000);
    if (delta < 60_000) return "刚刚";
    if (delta < 3_600_000) return `${Math.floor(delta / 60_000)} 分钟前`;
    if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)} 小时前`;
    return `${Math.floor(delta / 86_400_000)} 天前`;
  };
  const profileHealthy = (profile: Profile) => {
    const info = runtime.find((item) => item.name === profile.name);
    const accessReady = profile.type === "router"
      ? !!profile.baseUrl && profile.hasToken
      : !!info?.authenticated;
    return !!env?.claude_found && accessReady && info?.sharedDirsOk !== false;
  };

  return (
    <div className="config-panel">
      <Modal
        opened={deleteOpen}
        onClose={() => setDeleteOpen(false)}
        title={`移除实例${sel ? `「${sel}」` : ""}`}
        centered
      >
        <Stack gap="md">
          <Alert color="orange" icon={<IconAlertTriangle size={16} />}>
            请选择数据处理方式。彻底删除不可恢复，且会清除该实例的登录态、项目记录和历史用量数据。
          </Alert>
          <Button
            variant="light"
            onClick={() => onDelete(false)}
            loading={busyAction === "remove"}
            disabled={busyAction === "purge"}
          >
            仅移除，保留历史数据
          </Button>
          <Button
            color="red"
            leftSection={<IconTrash size={16} />}
            onClick={() => onDelete(true)}
            loading={busyAction === "purge"}
            disabled={busyAction === "remove"}
          >
            彻底删除实例与数据
          </Button>
          <Button variant="subtle" color="gray" onClick={() => setDeleteOpen(false)}>
            取消
          </Button>
        </Stack>
      </Modal>
      {/* 模型映射被绕过的告警：/model 钉死了具体型号 */}
      {pins.length > 0 && (
        <Alert
          color="orange"
          variant="light"
          icon={<IconAlertTriangle size={16} />}
          title="模型设置存在冲突"
          style={{ flex: "0 0 auto" }}
        >
          <Stack gap={6}>
            {pins.map((w) => (
              <Group key={w.profile} gap="xs" wrap="nowrap" justify="space-between">
                <Text size="sm">
                  {pinLabel(w.profile)}当前固定使用模型 <Code>{w.model}</Code>
                  {w.profile === "__main__"
                    ? "。从用户主目录启动 Claude 时，这项设置会优先于空间中的 Opus、Sonnet 和 Haiku 映射，实际模型可能与空间配置不一致。"
                    : "，因此该空间配置的 Opus、Sonnet 和 Haiku 映射不会生效。"}
                </Text>
                <Button
                  size="xs"
                  variant="light"
                  color="orange"
                  loading={fixBusy === w.profile}
                  onClick={() => onFixPin(w.profile)}
                  style={{ flexShrink: 0 }}
                >
                  恢复档位选择
                </Button>
              </Group>
            ))}
            <Text size="xs" c="dimmed">
              恢复后会清除固定型号，重新使用空间配置的模型映射。以后通过 /model
              切换模型时，请选择 Opus、Sonnet 或 Haiku，而不是带版本号的具体型号。
            </Text>
          </Stack>
        </Alert>
      )}

      <div className="config-grid">
      {/* 左栏：实例列表，独立滚动 */}
      <div className="instances-pane">
        <Card withBorder padding="sm" radius="lg" className="instances-card">
          <Group justify="space-between" mb="sm" px={4}>
            <div>
              <Title order={5}>运行环境</Title>
              <Text size="xs" c="dimmed">选择要配置的实例</Text>
            </div>
            <Button
              size="xs"
              variant="light"
              leftSection={<IconPlus size={14} />}
              onClick={onNew}
            >
              新建
            </Button>
          </Group>
          <div className="main-account-card">
            <div className="main-account-icon"><IconUser size={16} /></div>
            <div><Text fw={650} size="sm">主账户</Text><Text size="xs" c="dimmed">默认 Claude 环境 · {env?.claude_found ? "正常" : "CLI 未就绪"}</Text></div>
            <div className="main-account-usage"><strong>{mainUsage.requests}</strong><span>今日请求</span></div>
          </div>
          <Stack gap={4}>
            {profiles.length === 0 && (
              <Text size="sm" c="dimmed" p="xs">
                还没有实例。点「新建」创建第一个（比如 bj）。
              </Text>
            )}
            {profiles.map((p) => (
              <NavLink
                key={p.name}
                active={sel === p.name}
                label={<Text fw={650} size="sm">{p.name}</Text>}
                description={`${p.type === "router" ? "公司网关路由" : "独立 Claude 账户"} · ${profileHealthy(p) ? "正常" : "待完善"}`}
                leftSection={
                  p.type === "router" ? (
                    <IconWorld size={16} />
                  ) : (
                    <IconUser size={16} />
                  )
                }
                rightSection={
                  pins.some((w) => w.profile === p.name) ? (
                    <IconAlertTriangle size={15} color="var(--mantine-color-orange-6)" />
                  ) : <span className={`instance-health-dot ${profileHealthy(p) ? "ok" : "warn"}`} />
                }
                onClick={() => pickProfile(p)}
              />
            ))}
          </Stack>
        </Card>
      </div>

      {/* 右栏：实例设置表单（独立滚动） */}
      <div className="editor-scroll">
        <Card withBorder padding="lg" radius="lg" className="editor-card">
          <Stack gap="sm">
            <Group justify="space-between" className="editor-toolbar">
              <div>
                <Text size="xs" c="dimmed" fw={700} tt="uppercase" style={{ letterSpacing: 1.1 }}>Environment settings</Text>
                <Title order={4}>{sel ? sel : "创建新环境"}</Title>
              </div>
              <Group gap="xs">
                <Badge variant="light" color={sel ? "blue" : "green"}>
                  {sel ? "已创建" : "新实例"}
                </Badge>
                <Button
                  variant="subtle"
                  color="red"
                  size="xs"
                  leftSection={<IconTrash size={15} />}
                  onClick={() => setDeleteOpen(true)}
                  disabled={!sel}
                >
                  移除
                </Button>
                <Button
                  size="xs"
                  leftSection={<IconDeviceFloppy size={15} />}
                  onClick={onSave}
                  loading={busyAction === "save"}
                >
                  保存更改
                </Button>
              </Group>
            </Group>

            {sel && selProfile && (
              <div className="instance-overview">
                <div><span>运行状态</span><strong className={profileHealthy(selProfile) ? "status-ok" : "status-warn"}>{profileHealthy(selProfile) ? "环境正常" : "配置待完善"}</strong></div>
                <div><span>最近使用</span><strong>{formatLastUsed(selRuntime?.lastUsed)}</strong></div>
                <div><span>今日请求</span><strong>{selectedUsage.requests}</strong></div>
                <div><span>今日 Token</span><strong>{fmtNumber(selectedUsage.tokens)}</strong></div>
              </div>
            )}

            {selRuntime && (
              <div className="runtime-strip">
                <span>配置目录</span><Code>{selRuntime.configDir}</Code>
                <Badge size="xs" variant="light" color={selRuntime.settingsExists ? "teal" : "gray"}>{selRuntime.settingsExists ? "配置已生成" : "等待首次启动"}</Badge>
                <Badge size="xs" variant="light" color={selProfile?.type === "router" ? (selProfile.hasToken ? "teal" : "orange") : (selRuntime.authenticated ? "teal" : "orange")}>
                  {selProfile?.type === "router" ? (selProfile.hasToken ? "凭证已保存" : "缺少凭证") : (selRuntime.authenticated ? "账户已登录" : "等待登录")}
                </Badge>
                <Badge size="xs" variant="light" color={selRuntime.sharedDirsOk ? "cyan" : "orange"}>{selRuntime.sharedDirsOk ? "扩展全局共享" : "共享待修复"}</Badge>
                {selProfile?.type === "router" && <Badge size="xs" variant="light" color="blue">{[selProfile.opusModel, selProfile.sonnetModel, selProfile.haikuModel].filter(Boolean).length}/3 模型映射</Badge>}
              </div>
            )}

            <div className="form-section-label"><span>01</span><div><strong>连接信息</strong><small>实例身份与访问凭证</small></div></div>

            <TextInput
              label="实例名称 = 你要输入的命令词"
              description={
                sel
                  ? "名称创建后不可修改（它是固定的命令词）。如需改名，请删除后重新新建。"
                  : "例如填 bj，之后在终端用 claude bj。只能用英文字母/数字/下划线/短横线。创建后名称不可修改。"
              }
              placeholder="bj"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.currentTarget.value })}
              readOnly={!!sel}
              disabled={!!sel}
            />

            <Select
              label="类型"
              data={[
                { value: "router", label: "自定义路由（公司网关 / 第三方）" },
                { value: "account", label: "另一个账户（独立登录）" },
              ]}
              value={form.type}
              onChange={(v) => setForm({ ...form, type: (v || "router") as Profile["type"] })}
              allowDeselect={false}
            />

            {isRouter && (
              <>
                <TextInput
                  label="ANTHROPIC_BASE_URL（公司网关地址）"
                  description="按公司网关说明填写，通常要带 /anthropic 后缀。"
                  placeholder="https://10.0.147.128:8080/anthropic"
                  value={form.baseUrl}
                  onChange={(e) =>
                    setForm({ ...form, baseUrl: e.currentTarget.value })
                  }
                />
                <PasswordInput
                  label="API Key"
                  description={
                    selProfile?.hasToken
                      ? "已保存 Key（留空＝继续用原来的，要换才重新填）。"
                      : "公司网关发给你的 Key（如 gw-sk-...）。会加密存储（mac 钥匙串 / Windows DPAPI）。"
                  }
                  placeholder="gw-sk-••••••••"
                  value={token}
                  onChange={(e) => setToken(e.currentTarget.value)}
                />

                <div className="form-section-label"><span>02</span><div><strong>模型映射</strong><small>将 Claude 档位匹配到网关模型</small></div></div>
                <Group justify="space-between" align="center">
                  <Text size="xs" c="dimmed">
                    点「检测模型」从当前网关拉取可用模型，自动加入下方下拉。
                  </Text>
                  <Button
                    size="xs"
                    variant="light"
                    leftSection={<IconListSearch size={14} />}
                    onClick={onDetect}
                    loading={detectBusy}
                  >
                    检测模型
                  </Button>
                </Group>

                <Autocomplete
                  label="Opus 档（复杂任务，最强）"
                  placeholder="如 glm-5.2 / claude-opus-4-7"
                  data={modelOpts}
                  value={form.opusModel}
                  onChange={(v) => setForm({ ...form, opusModel: v })}
                />
                <Autocomplete
                  label="Sonnet 档（日常默认）"
                  placeholder="如 glm-5.1 / claude-sonnet-4-6"
                  data={modelOpts}
                  value={form.sonnetModel}
                  onChange={(v) => setForm({ ...form, sonnetModel: v })}
                />
                <Autocomplete
                  label="Haiku 档（轻量、快速、后台子任务）"
                  placeholder="如 glm-5-turbo / claude-haiku-4-5"
                  data={modelOpts}
                  value={form.haikuModel}
                  onChange={(v) => setForm({ ...form, haikuModel: v })}
                />
              </>
            )}

            <div className="form-section-label"><span>03</span><div><strong>自动化与命令</strong><small>共享策略与终端调用方式</small></div></div>
            <Alert variant="light" color="cyan" icon={<IconInfoCircle size={16} />}>
              <Text size="xs">
                skills / plugins / agents / commands 会自动共享；MCP 与插件启用状态在每次启动 Claude 时双向同步。
                跨实例共享的 MCP 请用 <Code>claude mcp add -s user</Code> 安装。
              </Text>
            </Alert>

            <Box>
              <Text size="sm" fw={500}>
                用法预览
              </Text>
              <Code block>
                {`cd 任意项目目录\nclaude            # 主账户，原样\nclaude ${
                  form.name.trim() || "<名称>"
                }     # 切到这个实例，跑完自动恢复`}
              </Code>
            </Box>

            {status.msg && (
              <Alert
                mt="xs"
                variant="light"
                color={
                  status.type === "error"
                    ? "red"
                    : status.type === "success"
                    ? "teal"
                    : "blue"
                }
                icon={<IconInfoCircle size={16} />}
              >
                {status.msg}
              </Alert>
            )}

            <Text size="xs" c="dimmed">
              提示：保存后<b>重开一个终端窗口</b>（或 mac 跑{" "}
              <Code>source ~/.zshrc</Code>、Windows 跑 <Code>. $PROFILE</Code>）即可生效。
            </Text>
          </Stack>
        </Card>
      </div>
      </div>
    </div>
  );
}
