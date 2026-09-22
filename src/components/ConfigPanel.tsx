import { usePageActivation, usePageActive } from "./PersistentPage";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
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
  Autocomplete,
  Modal,
  ActionIcon,
  Tooltip,
  Loader,
} from "@mantine/core";
import {
  IconPlus,
  IconTrash,
  IconDeviceFloppy,
  IconWorld,
  IconUser,
  IconInfoCircle,
  IconAlertTriangle,
  IconActivityHeartbeat,
  IconClockHour4,
  IconMessageCircle,
  IconCoins,
  IconLink,
  IconCpu,
  IconTerminal2,
  IconShieldLock,
  IconFolder,
  IconRefresh,
} from "@tabler/icons-react";
import { api } from "../api";
import type { EnvInfo, Profile, ModelPinWarning, ProfileRuntimeInfo, UsageStats } from "../api";
import InstanceSettingsCard from "./InstanceSettingsCard";
import StableRefreshButton from "./StableRefreshButton";
import { buildModelOptions } from "./modelOptions";
import { profileRuntimeStatus } from "./profileRuntimeStatus";

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

function ConfigSectionHeading({
  icon,
  title,
  description,
  action,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
      <div className="environment-section-heading">
      <div className="environment-section-icon" aria-hidden="true">{icon}</div>
      <div className="environment-section-copy">
        <strong>{title}</strong>
        <small>{description}</small>
      </div>
      {action && <div className="environment-section-action">{action}</div>}
    </div>
  );
}

export default function ConfigPanel({
  onChanged,
  env,
  usageData,
  refreshRevision = 0,
}: {
  onChanged?: () => void;
  env: EnvInfo | null;
  usageData: UsageStats | null;
  refreshRevision?: number;
}) {
  const pageActive = usePageActive();
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
  // 最近一次完整诊断的结论（含网关失败名单）：列表页不做实时探测，
  // 只消费这里的结论；单环境复测通过后本地立即摘除标记。
  const [verification, setVerification] = useState<{ at: number; problems: number; gatewayFails?: string[] } | null>(null);
  const [probeBusy, setProbeBusy] = useState("");

  // 当前连接的检测结果；切换连接后清空。
  const [detected, setDetected] = useState<string[]>([]);
  const [detectBusy, setDetectBusy] = useState(false);
  const [detectStatus, setDetectStatus] = useState<{ type: StatusType; msg: string } | null>(null);
  const detectRequest = useRef(0);
  const detectInFlight = useRef(false);

  // 连接或环境改变后，旧请求只能结束，不能更新当前表单。
  useEffect(() => {
    setDetected([]);
    setDetectBusy(false);
    setDetectStatus(null);
    detectInFlight.current = false;
    return () => { detectRequest.current += 1; };
  }, [sel, form.baseUrl, form.type, token]);

  // /model 钉死具体型号的告警（含默认 Claude __main__）
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
    api.lastVerification().then(setVerification).catch(() => {});
  };
  useEffect(load, [refreshRevision]);
  usePageActivation(load);

  const pinLabel = (profile: string) =>
    profile === "__main__" ? "默认 Claude" : `环境 ${profile}`;

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
    // 提示条是页面级共享状态，切环境必须清掉，否则上一个环境的报错会"跟着"过来
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
    if (!n) return "请填写环境名称。";
    // 编辑已有环境：名称不可改（输入框已禁用），旧规则时代的名字放行，只校验新建
    if (sel) return null;
    if (!/^[A-Za-z0-9_-]{1,40}$/.test(n))
      return "名称只能包含英文字母、数字、下划线、短横线（1~40 个字符）。";
    if (n.startsWith("__")) return "名称不能以 __ 开头（内部保留前缀）。";
    if (form.type === "router") {
      const url = form.baseUrl.trim();
      if (!/^https?:\/\/\S+$/i.test(url) || url.length > 2048)
        return "网关地址必须以 http:// 或 https:// 开头，且不能包含空格。";
    }
    return null;
  };

  // 检测候选只来自当前网关；已保存取值仍保留在输入框中。
  // 从未检测成功 → 预设兜底。见 modelOptions.ts
  const modelOpts = useMemo(
    () => buildModelOptions(detected),
    [detected]
  );

  const onDetect = async () => {
    if (detectInFlight.current) return;
    const baseUrl = form.baseUrl.trim();
    if (!baseUrl) {
      setDetectStatus({ type: "error", msg: "请先填写网关地址再检测。" });
      return;
    }
    const saved = profiles.find((p) => p.name === sel);
    if (sel && !token.trim() && baseUrl !== saved?.baseUrl.trim()) {
      setDetectStatus({ type: "error", msg: "网关地址已修改，请先保存更改，或填写 API Key 后检测当前地址。" });
      return;
    }
    if (!sel && !token.trim()) {
      setDetectStatus({ type: "error", msg: "新建环境需先填 API Key 才能检测模型。" });
      return;
    }
    const request = ++detectRequest.current;
    detectInFlight.current = true;
    setDetectBusy(true);
    setDetectStatus({ type: "info", msg: "正在检测当前网关模型…" });
    try {
      const list = token.trim()
        ? await api.detectModels(baseUrl, token.trim(), sel || "")
        : await api.detectModelsFor(sel!);
      if (request !== detectRequest.current) return;
      const models = [...new Set(list.map((m) => m.trim()).filter(Boolean))];
      if (!models.length) throw new Error("网关未返回可用模型，请检查地址与访问权限后重试。");
      setDetected(models);
      setDetectStatus({
        type: "success",
        msg: `检测到 ${models.length} 个可用模型。点击下方模型输入框查看候选；当前档位取值已保留。`,
      });
    } catch (e) {
      if (request === detectRequest.current) setDetectStatus({ type: "error", msg: String(e) });
    } finally {
      if (request === detectRequest.current) {
        detectInFlight.current = false;
        setDetectBusy(false);
      }
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
          ? "保存出错，疑似证书问题。可到「设置 → CA 证书」导入证书后重试。原始错误：" + m
          : m,
      });
    } finally {
      setBusyAction("");
    }
  };

  const onDelete = async () => {
    if (!sel) {
      setStatus({ type: "error", msg: "请先在左侧选中一个环境。" });
      return;
    }
    setBusyAction("delete");
    try {
      const msg = await api.deleteProfile(sel);
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
  const statusForProfile = (profile: Profile) => {
    const info = runtime.find((item) => item.name === profile.name);
    const gatewayDown = !!verification?.gatewayFails?.includes(profile.name);
    const status = profileRuntimeStatus(profile, info, !!env?.claude_found, gatewayDown);
    // 「上次诊断」这种静态措辞会让刚跑完的检测看起来像陈年旧数据；
    // 直接写明结论距离现在多久，用户自己判断新鲜度。
    if (status.gatewayDown && verification) {
      return { ...status, label: `网关未连通 · ${formatLastUsed(verification.at)}检测` };
    }
    return status;
  };

  // 网关不通是「环境当前不可用」，与本地配置未就绪（橙）区分：直接红。
  const healthClass = (profile: Profile) => {
    const status = statusForProfile(profile);
    return status.gatewayDown ? "bad" : status.healthy ? "ok" : "warn";
  };

  // 单环境网关复测：与诊断页同一套判定；结论由后端写回最近验证记录
  const onProbeGateway = async (name: string) => {
    setProbeBusy(name);
    try {
      const msg = await api.probeGateway(name);
      setStatus({ type: "success", msg: `环境 ${name}：${msg}` });
      setVerification((previous) => ({
        at: Math.floor(Date.now() / 1000),
        problems: Math.max(0, (previous?.problems ?? 0) - (previous?.gatewayFails?.includes(name) ? 1 : 0)),
        gatewayFails: (previous?.gatewayFails ?? []).filter((item) => item !== name),
      }));
    } catch (e) {
      setStatus({ type: "error", msg: `环境 ${name}：${String(e)}` });
      setVerification((previous) => {
        const failures = previous?.gatewayFails ?? [];
        const alreadyFailed = failures.includes(name);
        return {
          at: Math.floor(Date.now() / 1000),
          problems: (previous?.problems ?? 0) + (alreadyFailed ? 0 : 1),
          gatewayFails: alreadyFailed ? failures : [...failures, name],
        };
      });
    } finally {
      setProbeBusy("");
    }
  };

  return (
    <div className="config-panel">
      <Modal
        opened={pageActive && (deleteOpen)}
        onClose={() => setDeleteOpen(false)}
        title={`彻底删除环境${sel ? `「${sel}」` : ""}`}
        centered
      >
        <Stack gap="md">
          <Alert color="orange" icon={<IconAlertTriangle size={16} />}>
            此操作不可恢复，将清除该环境的配置、API Key、登录态、项目记录、历史用量数据、终端命令和同步记录。
          </Alert>
          <Button
            color="red"
            leftSection={<IconTrash size={16} />}
            onClick={onDelete}
            loading={busyAction === "delete"}
          >
            确认彻底删除
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
            {pins.map((w) =>
              // 决策 7.4：默认 Claude **只读告警，不给「一键恢复」**。
              // 应用可以发现风险并说明原因，但不替用户改写他自己那份配置。
              w.profile === "__main__" ? (
                <Stack key={w.profile} gap={2}>
                  <Text size="sm">
                    检测到默认 Claude 配置固定使用模型 <Code>{w.model}</Code>
                    。当你从用户主目录启动网关环境时，该设置可能覆盖环境的模型映射。
                    <b>应用不会修改此配置。</b>建议从实际项目目录启动；如需调整，请在
                    Claude Code 中自行修改。
                  </Text>
                  <Text size="xs" c="dimmed">
                    配置文件：<Code>{w.settingsPath}</Code>
                  </Text>
                </Stack>
              ) : (
                <Group
                  key={w.profile}
                  gap="xs"
                  wrap="nowrap"
                  justify="space-between"
                >
                  <Text size="sm">
                    环境 <Code>{w.profile}</Code> 当前固定使用模型{" "}
                    <Code>{w.model}</Code>
                    ，因此该环境配置的 Opus、Sonnet 和 Haiku 映射不会生效。
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
              )
            )}
            {/* 兜底说明只在真有环境钉死时才写：默认 Claude 那条没有按钮，写它就成了误导 */}
            {pins.some((w) => w.profile !== "__main__") && (
              <Text size="xs" c="dimmed">
                恢复后会清除固定型号，重新使用环境配置的模型映射。以后通过 /model
                切换模型时，请选择 Opus、Sonnet 或 Haiku，而不是带版本号的具体型号。
              </Text>
            )}
          </Stack>
        </Alert>
      )}

      <div className="config-grid">
      {/* 左栏：环境列表，独立滚动 */}
      <div className="instances-pane">
        <Card withBorder padding="sm" radius="lg" className="instances-card">
          <Group justify="space-between" mb="sm" px={4}>
            <div>
              <Title order={5}>运行环境</Title>
              <Text size="xs" c="dimmed">选择要配置的环境</Text>
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
            {/* 默认 Claude**不是环境** —— 它是"没切环境"时的那个账户，本应用只读不写。
                原先把这里写成「默认 Claude 环境」，等于把它当成了环境的一种。 */}
            <div><Text fw={650} size="sm">默认 Claude</Text></div>
            <div className="main-account-usage"><strong>{mainUsage.requests}</strong><span>今日请求</span></div>
          </div>
          <Stack gap={4}>
            {profiles.length === 0 && (
              <Text size="sm" c="dimmed" p="xs">
                还没有环境。点「新建」创建第一个（比如 bj）。
              </Text>
            )}
            {profiles.map((p) => (
              <NavLink
                key={p.name}
                active={sel === p.name}
                label={<Text fw={650} size="sm">{p.name}</Text>}
                description={`${p.type === "router" ? "网关环境" : "独立登录环境"} · ${statusForProfile(p).shortLabel}`}
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
                  ) : <span className={`instance-health-dot ${healthClass(p)}`} />
                }
                onClick={() => pickProfile(p)}
              />
            ))}
          </Stack>
        </Card>
      </div>

      {/* 右栏：环境设置表单（独立滚动） */}
      <div className="editor-pane">
        <div className="editor-scroll">
          <Card withBorder padding={0} radius="lg" className="editor-card">
          <Stack gap={0}>
            <Group justify="space-between" className="editor-toolbar" wrap="nowrap">
              <Group gap="sm" wrap="nowrap" className="environment-title-group">
                <div className="environment-title-icon">
                  {isRouter ? <IconWorld size={22} /> : <IconUser size={22} />}
                </div>
                <div className="environment-title-copy">
                  <Text size="xs" c="dimmed" fw={700}>环境配置</Text>
                  <Group gap="xs" wrap="wrap">
                    <Title order={3}>{sel ? sel : "创建新环境"}</Title>
                    <Badge variant="light" color={sel ? "blue" : "green"}>
                      {sel ? (isRouter ? "网关环境" : "独立登录") : "新环境"}
                    </Badge>
                  </Group>
                </div>
              </Group>
              <Group gap="xs" wrap="nowrap" className="environment-editor-actions">
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

            <div className="environment-editor-content">

            {status.msg && (
              <Alert
                variant="light"
                color={status.type === "error" ? "red" : status.type === "success" ? "teal" : "blue"}
                icon={<IconInfoCircle size={16} />}
              >
                {status.msg}
              </Alert>
            )}

            {sel && selProfile && (
              <div className="instance-overview">
                <div className="environment-stat">
                  <div className="environment-stat-icon"><IconActivityHeartbeat size={17} /></div>
                  <div className="environment-stat-copy">
                    <span>运行状态</span>
                    <strong className={`status-${healthClass(selProfile)}`}>{statusForProfile(selProfile).label}</strong>
                  </div>
                  {isRouter && (
                    <Tooltip label="检测当前环境" position="top" withArrow>
                      <ActionIcon
                        className="environment-status-probe"
                        variant="subtle"
                        radius="xl"
                        size={32}
                        aria-label="检测当前环境"
                        aria-busy={probeBusy === selProfile.name}
                        disabled={probeBusy === selProfile.name}
                        onClick={() => { void onProbeGateway(selProfile.name); }}
                      >
                        {probeBusy === selProfile.name
                          ? <Loader size={14} />
                          : <IconRefresh size={15} />}
                      </ActionIcon>
                    </Tooltip>
                  )}
                </div>
                <div className="environment-stat">
                  <div className="environment-stat-icon"><IconClockHour4 size={17} /></div>
                  <div className="environment-stat-copy"><span>最近使用</span><strong>{formatLastUsed(selRuntime?.lastUsed)}</strong></div>
                </div>
                <div className="environment-stat">
                  <div className="environment-stat-icon"><IconMessageCircle size={17} /></div>
                  <div className="environment-stat-copy"><span>今日请求</span><strong>{selectedUsage.requests}</strong></div>
                </div>
                <div className="environment-stat">
                  <div className="environment-stat-icon"><IconCoins size={17} /></div>
                  <div className="environment-stat-copy"><span>今日 Token</span><strong>{fmtNumber(selectedUsage.tokens)}</strong></div>
                </div>
              </div>
            )}

            {selRuntime && (
              <div className="environment-runtime-card">
                <div className="environment-runtime-path">
                  <IconFolder size={17} />
                  <div><span>配置目录</span><Code>{selRuntime.configDir}</Code></div>
                </div>
                <div className="environment-runtime-badges">
                  <Badge size="sm" variant="light" color={selRuntime.settingsExists ? "teal" : "gray"}>{selRuntime.settingsExists ? "配置已生成" : "等待首次启动"}</Badge>
                  <Badge size="sm" variant="light" color={selProfile?.type === "router" ? (selProfile.hasToken ? "teal" : "orange") : (selRuntime.authenticated ? "teal" : "orange")}>
                    {selProfile?.type === "router" ? (selProfile.hasToken ? "凭证已保存" : "缺少凭证") : (selRuntime.authenticated ? "账户已登录" : "等待登录")}
                  </Badge>
                  <Badge size="sm" variant="light" color={selRuntime.sharedDirsOk ? "cyan" : "orange"}>{selRuntime.sharedDirsOk ? "扩展结构正常" : "扩展待迁移"}</Badge>
                  {selProfile?.type === "router" && <Badge size="sm" variant="light" color="blue">{[selProfile.opusModel, selProfile.sonnetModel, selProfile.haikuModel].filter(Boolean).length}/3 模型映射</Badge>}
                </div>
              </div>
            )}

            <section className="environment-config-section">
              <ConfigSectionHeading icon={<IconLink size={18} />} title="连接信息" description="环境身份与访问凭证" />
              <div className="environment-field-grid">
                <TextInput
                  label="环境名称 = 你要输入的命令词"
                  description={
                    sel
                      ? "名称创建后不可修改。如需改名，请删除后重新新建。"
                      : "例如 bj，之后在终端运行 claude bj。创建后名称不可修改。"
                  }
                  placeholder="bj"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.currentTarget.value })}
                  readOnly={!!sel}
                  disabled={!!sel}
                />
                <Select
                  label="环境类型"
                  description="选择通过网关接入，或使用独立 Claude 登录。"
                  data={[
                    { value: "router", label: "网关环境（公司网关 / 第三方）" },
                    { value: "account", label: "独立登录环境（独立登录）" },
                  ]}
                  value={form.type}
                  onChange={(v) => setForm({ ...form, type: (v || "router") as Profile["type"] })}
                  allowDeselect={false}
                />
                {isRouter && (
                  <>
                <TextInput
                  className="environment-field-wide"
                  label="ANTHROPIC_BASE_URL（公司网关地址）"
                  description="按公司网关说明填写，通常要带 /anthropic 后缀。"
                  placeholder="https://gateway.example.com:8080/anthropic"
                  value={form.baseUrl}
                  onChange={(e) =>
                    setForm({ ...form, baseUrl: e.currentTarget.value })
                  }
                />
                <PasswordInput
                  className="environment-field-wide"
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
                  </>
                )}
              </div>
            </section>

            {isRouter && (
              <section className="environment-config-section">
                <ConfigSectionHeading
                  icon={<IconCpu size={18} />}
                  title="模型映射"
                  description="将 Claude 档位匹配到网关模型"
                  action={<StableRefreshButton
                    key={sel ?? "new"}
                    size="xs"
                    iconSize={14}
                    label="检测模型"
                    busyLabel="检测中…"
                    onClick={onDetect}
                    busy={detectBusy}
                  />}
                />
                {detectStatus && (
                  <Alert
                    data-model-detection-status
                    role={detectStatus.type === "error" ? "alert" : "status"}
                    variant="light"
                    color={detectStatus.type === "error" ? "red" : detectStatus.type === "success" ? "teal" : "blue"}
                    icon={<IconInfoCircle size={16} />}
                  >
                    {detectStatus.msg}
                  </Alert>
                )}
                <div className="environment-model-grid">
                  <Autocomplete
                    label="Opus · 复杂任务"
                    placeholder="如 glm-5.2"
                    data={modelOpts}
                    value={form.opusModel}
                    onChange={(v) => setForm({ ...form, opusModel: v })}
                  />
                  <Autocomplete
                    label="Sonnet · 日常默认"
                    placeholder="如 glm-5.1"
                    data={modelOpts}
                    value={form.sonnetModel}
                    onChange={(v) => setForm({ ...form, sonnetModel: v })}
                  />
                  <Autocomplete
                    label="Haiku · 轻量快速"
                    placeholder="如 glm-5-turbo"
                    data={modelOpts}
                    value={form.haikuModel}
                    onChange={(v) => setForm({ ...form, haikuModel: v })}
                  />
                </div>
              </section>
            )}

            <section className="environment-config-section">
              <ConfigSectionHeading icon={<IconTerminal2 size={18} />} title="自动化与命令" description="共享策略与终端调用方式" />
              <div className="environment-info-note">
                <IconInfoCircle size={17} />
                <Text size="xs">
                  Skills 与 Agents 在扩展中心逐项共享；Plugins 按环境安装；跨环境 MCP 请在
                  「MCP 服务」中选择「所有环境」。
                </Text>
              </div>
              <div className="environment-command-preview">
                <div className="environment-command-heading">
                  <IconTerminal2 size={17} />
                  <div><strong>启动命令</strong><span>保存后在新终端窗口中使用</span></div>
                </div>
                <Code block>
                  {`claude            # 默认 Claude\nclaude ${
                    form.name.trim() || "<名称>"
                  }     # 使用该环境`}
                </Code>
              </div>
            </section>

            {sel && (
              <section className="environment-config-section environment-advanced-section">
                <ConfigSectionHeading icon={<IconShieldLock size={18} />} title="权限与高级配置" description="该环境的 settings.json" />
                <InstanceSettingsCard key={sel} name={sel} />
              </section>
            )}

            {env?.platform_ui?.shell_reload_instruction && (
              <Text size="xs" c="dimmed" className="environment-terminal-hint">
                {env.platform_ui.shell_reload_instruction}
              </Text>
            )}
            </div>
          </Stack>
          </Card>
        </div>
      </div>
      </div>
    </div>
  );
}
