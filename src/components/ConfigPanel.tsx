import { useEffect, useMemo, useState } from "react";
import {
  Card,
  Stack,
  Group,
  Button,
  TextInput,
  PasswordInput,
  Select,
  Switch,
  Text,
  Title,
  NavLink,
  Badge,
  Code,
  Alert,
  Box,
  Autocomplete,
  Divider,
  Collapse,
} from "@mantine/core";
import {
  IconPlus,
  IconTrash,
  IconDeviceFloppy,
  IconWorld,
  IconUser,
  IconInfoCircle,
  IconCertificate,
  IconListSearch,
} from "@tabler/icons-react";
import { api } from "../api";
import type { Profile, EnvInfo } from "../api";

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
  env,
  onChanged,
}: {
  env: EnvInfo | null;
  onChanged?: () => void;
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
  const [certPath, setCertPath] = useState("");

  // 检测到的当前实例模型（合并进下方下拉）
  const [detected, setDetected] = useState<string[]>([]);
  const [detectBusy, setDetectBusy] = useState(false);

  // CA 证书区（全局）：开关 + 反馈
  // 用 lazy initializer 从 env 直接算初值，避免"先收起再展开"的闪动
  // （每次切回配置页 ConfigPanel 会重新挂载，若初值 false 再靠 effect 改 true 就会闪）。
  const [caOpen, setCaOpen] = useState(() => !!env && (env.cert_count ?? 0) > 0);
  const [caMsg, setCaMsg] = useState<{ type: StatusType; msg: string }>({
    type: "info",
    msg: "",
  });
  const certCount = env?.cert_count ?? 0;

  // 仅在 env 挂载后才到达（首次启动 env 异步加载）时，按证书张数自动展开一次
  useEffect(() => {
    if (env && env.cert_count > 0) setCaOpen(true);
  }, [env]);

  const load = () => {
    api
      .listProfiles()
      .then((ps) => setProfiles(ps || []))
      .catch((e) => setStatus({ type: "error", msg: String(e) }));
  };
  useEffect(load, []);

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
    if (/[\s\\/:*?"<>|]/.test(n))
      return '名称不能含空格或 \\ / : * ? " < > | 等字符。';
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
          ? "保存出错，疑似证书问题。可在左侧「CA 证书」导入证书后重试。原始错误：" + m
          : m,
      });
    } finally {
      setBusyAction("");
    }
  };

  const onDelete = async () => {
    if (!sel) {
      setStatus({ type: "error", msg: "请先在左侧选中一个实例。" });
      return;
    }
    setBusyAction("delete");
    try {
      await api.deleteProfile(sel);
      onNew();
      load();
      onChanged && onChanged();
      setStatus({ type: "success", msg: "已删除并更新终端集成。重开终端生效。" });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusyAction("");
    }
  };

  const onImportCert = async () => {
    if (!certPath.trim()) {
      setCaMsg({ type: "error", msg: "请填写证书文件（ca-cert.pem）的完整路径。" });
      return;
    }
    setBusyAction("import");
    try {
      const msg = await api.importCert(certPath.trim());
      onChanged && onChanged();
      setCaMsg({ type: "success", msg });
    } catch (e) {
      setCaMsg({ type: "error", msg: String(e) });
    } finally {
      setBusyAction("");
    }
  };

  const onClearCerts = async () => {
    setBusyAction("clear");
    try {
      const msg = await api.clearCerts();
      onChanged && onChanged();
      setCaMsg({ type: "success", msg });
    } catch (e) {
      setCaMsg({ type: "error", msg: String(e) });
    } finally {
      setBusyAction("");
    }
  };

  const selProfile = profiles.find((p) => p.name === sel);
  const isRouter = form.type === "router";
  const certPlaceholder =
    env?.platform === "windows" ? "C:\\ca-cert.pem" : "/Users/you/ca-cert.pem";

  return (
    <div
      style={{
        display: "flex",
        gap: 16,
        alignItems: "stretch",
        height: "100%",
      }}
    >
      {/* 左栏：实例列表 + 全局 CA 证书，独立滚动 */}
      <div
        style={{
          flex: "0 0 33.3333%",
          minWidth: 260,
          overflowY: "auto",
        }}
      >
        <Stack gap="md">
          <Card withBorder padding="sm" radius="md">
            <Group justify="space-between" mb="xs">
              <Title order={5}>实例</Title>
              <Button
                size="xs"
                variant="light"
                leftSection={<IconPlus size={14} />}
                onClick={onNew}
              >
                新建
              </Button>
            </Group>
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
                  label={p.name}
                  description={p.type === "router" ? "公司路由" : "另一个账户"}
                  leftSection={
                    p.type === "router" ? (
                      <IconWorld size={16} />
                    ) : (
                      <IconUser size={16} />
                    )
                  }
                  onClick={() => pickProfile(p)}
                />
              ))}
            </Stack>
          </Card>

          {/* 全局 CA 证书：整机共享，所有实例通用 */}
          <Card withBorder padding="sm" radius="md">
            <Group justify="space-between" mb={4}>
              <Group gap={6}>
                <IconCertificate size={16} />
                <Title order={6}>CA 证书</Title>
              </Group>
              <Badge size="sm" variant="light" color={certCount ? "teal" : "gray"}>
                {certCount ? `${certCount} 张` : "未导入"}
              </Badge>
            </Group>
            <Text size="xs" c="dimmed" mb="xs">
              整机共享：多张证书会自动合并到一个信任库，所有实例通用。
            </Text>
            <Switch
              label="使用自定义 CA 证书"
              checked={caOpen}
              onChange={(e) => setCaOpen(e.currentTarget.checked)}
            />
            <Collapse in={caOpen}>
              <Stack gap="xs" mt="sm">
                {caMsg.msg && (
                  <Text
                    size="xs"
                    c={
                      caMsg.type === "error"
                        ? "red"
                        : caMsg.type === "success"
                        ? "teal"
                        : "dimmed"
                    }
                  >
                    {caMsg.msg}
                  </Text>
                )}
                <TextInput
                  size="xs"
                  placeholder={certPlaceholder}
                  value={certPath}
                  onChange={(e) => setCertPath(e.currentTarget.value)}
                />
                <Group gap="xs">
                  <Button
                    size="xs"
                    onClick={onImportCert}
                    loading={busyAction === "import"}
                  >
                    导入（追加）
                  </Button>
                  <Button
                    size="xs"
                    variant="light"
                    color="red"
                    onClick={onClearCerts}
                    loading={busyAction === "clear"}
                    disabled={!certCount}
                  >
                    清空
                  </Button>
                </Group>
              </Stack>
            </Collapse>
          </Card>
        </Stack>
      </div>

      {/* 右栏：实例设置表单（独立滚动） */}
      <div
        style={{
          flex: 1,
          minWidth: 0,
          overflowY: "auto",
        }}
      >
        <Card withBorder padding="lg" radius="md">
          <Stack gap="sm">
            <Group justify="space-between">
              <Title order={5}>实例设置</Title>
              <Badge variant="light" color={sel ? "blue" : "green"}>
                {sel ? `编辑：${sel}` : "新建实例"}
              </Badge>
            </Group>

            <TextInput
              label="实例名称 = 你要输入的命令词"
              description={
                sel
                  ? "名称创建后不可修改（它是固定的命令词）。如需改名，请删除后重新新建。"
                  : "例如填 bj，之后在终端用 claude bj。建议英文/数字、无空格。创建后名称不可修改。"
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

                <Divider
                  my={4}
                  label="模型映射（把 Claude 的模型档位指到网关可用的模型）"
                  labelPosition="left"
                />
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

            <Text size="sm" fw={500} mt={4}>
              共享与同步（自动）
            </Text>
            <Text size="xs" c="dimmed">
              所有实例自动与主账户共享 skills / plugins / agents / commands；
              MCP 服务器与插件启用状态在每次启动 claude 时自动双向同步，
              任一实例安装或删除，其他实例下次启动即生效。
              跨实例共享的 MCP 请用 <Code>claude mcp add -s user</Code> 安装。
            </Text>

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

            <Group mt="xs">
              <Button
                leftSection={<IconDeviceFloppy size={16} />}
                onClick={onSave}
                loading={busyAction === "save"}
              >
                保存并接入终端
              </Button>
              <Button
                variant="subtle"
                color="red"
                leftSection={<IconTrash size={16} />}
                onClick={onDelete}
                loading={busyAction === "delete"}
                disabled={!sel}
              >
                删除
              </Button>
            </Group>

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
  );
}
