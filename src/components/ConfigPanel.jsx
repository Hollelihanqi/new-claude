import { useEffect, useState } from "react";
import {
  Grid,
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
  ScrollArea,
  NavLink,
  Badge,
  Code,
  Alert,
  Box,
  Autocomplete,
  Divider,
} from "@mantine/core";
import {
  IconPlus,
  IconTrash,
  IconDeviceFloppy,
  IconWorld,
  IconUser,
  IconInfoCircle,
  IconRefresh,
  IconCertificate,
} from "@tabler/icons-react";
import { api } from "../api.js";

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

const empty = {
  name: "",
  type: "router",
  baseUrl: "",
  shareSkills: false,
  sharePlugins: false,
  opusModel: "",
  sonnetModel: "",
  haikuModel: "",
};

const isCertError = (s) => /cert|ssl|self.?signed|证书|signature/i.test(String(s));

export default function ConfigPanel({ env, onChanged }) {
  const [profiles, setProfiles] = useState([]);
  const [sel, setSel] = useState(null);
  const [form, setForm] = useState(empty);
  const [token, setToken] = useState("");
  const [status, setStatus] = useState({ type: "info", msg: "" });
  const [busyAction, setBusyAction] = useState("");
  const [models, setModels] = useState([]);
  const [certPath, setCertPath] = useState("");

  const load = () => {
    api
      .listProfiles()
      .then((ps) => setProfiles(ps || []))
      .catch((e) => setStatus({ type: "error", msg: String(e) }));
  };
  useEffect(load, []);

  const pickProfile = (p) => {
    setSel(p.name);
    setForm({
      name: p.name,
      type: p.type,
      baseUrl: p.baseUrl || "",
      shareSkills: !!p.shareSkills,
      sharePlugins: !!p.sharePlugins,
      opusModel: p.opusModel || "",
      sonnetModel: p.sonnetModel || "",
      haikuModel: p.haikuModel || "",
    });
    setToken("");
  };

  const onNew = () => {
    setSel(null);
    setForm(empty);
    setToken("");
    setStatus({ type: "info", msg: "填写名称（即命令词，如 bj）后保存。" });
  };

  const valid = () => {
    const n = form.name.trim();
    if (!n) return "请填写实例名称。";
    if (/[\s\\/:*?"<>|]/.test(n))
      return '名称不能含空格或 \\ / : * ? " < > | 等字符。';
    return null;
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
        shareSkills: form.shareSkills,
        sharePlugins: form.sharePlugins,
        opusModel: form.opusModel.trim(),
        sonnetModel: form.sonnetModel.trim(),
        haikuModel: form.haikuModel.trim(),
      };
      const msg = await api.saveProfile(profile, token || null);
      let extra = "";
      if (form.shareSkills || form.sharePlugins) {
        try {
          const linkMsg = await api.syncLinks(
            profile.name,
            form.shareSkills,
            form.sharePlugins
          );
          extra = "（同步：" + linkMsg + "）";
        } catch (e) {
          extra = "（skills/plugins 同步失败：" + String(e) + "）";
        }
      }
      setToken("");
      load();
      onChanged && onChanged();
      setSel(profile.name);
      setStatus({
        type: "success",
        msg: `已保存「${profile.name}」。${msg}${extra} 之后在新终端里运行：claude ${profile.name}`,
      });
    } catch (e) {
      const m = String(e);
      setStatus({
        type: "error",
        msg: isCertError(m)
          ? "保存出错，疑似证书问题。可在下方导入 CA 证书后重试。原始错误：" + m
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

  const onDetect = async () => {
    if (!form.baseUrl.trim()) {
      setStatus({ type: "error", msg: "请先填写网关地址。" });
      return;
    }
    if (!token.trim()) {
      setStatus({ type: "error", msg: "请先填写 API Key（检测需要鉴权）。" });
      return;
    }
    setBusyAction("detect");
    try {
      const list = await api.detectModels(form.baseUrl.trim(), token.trim());
      setModels(list);
      setStatus({
        type: "success",
        msg: `检测到 ${list.length} 个可用模型，已加入下方模型下拉候选：${list.join("、")}`,
      });
    } catch (e) {
      const m = String(e);
      setStatus({
        type: "error",
        msg: isCertError(m)
          ? "检测失败，疑似证书未导入。请在下方「导入 CA 证书」处导入后重试。原始错误：" + m
          : "检测失败：" + m,
      });
    } finally {
      setBusyAction("");
    }
  };

  const onImportCert = async () => {
    if (!certPath.trim()) {
      setStatus({ type: "error", msg: "请填写证书文件（ca-cert.pem）的完整路径。" });
      return;
    }
    setBusyAction("import");
    try {
      const msg = await api.importCert(certPath.trim());
      onChanged && onChanged();
      setStatus({ type: "success", msg });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusyAction("");
    }
  };

  const selProfile = profiles.find((p) => p.name === sel);
  const isRouter = form.type === "router";
  const modelData = Array.from(new Set([...models, ...PRESET_MODELS]));
  const certPlaceholder =
    env?.platform === "windows" ? "C:\\ca-cert.pem" : "/Users/you/ca-cert.pem";

  return (
    <Grid gutter="md">
      <Grid.Col span={{ base: 12, sm: 4 }}>
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
          <ScrollArea.Autosize mah={420}>
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
          </ScrollArea.Autosize>
        </Card>
      </Grid.Col>

      <Grid.Col span={{ base: 12, sm: 8 }}>
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
              onChange={(v) => setForm({ ...form, type: v || "router" })}
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
                <Group justify="space-between" align="flex-end">
                  <Text size="xs" c="dimmed" style={{ flex: 1 }}>
                    填好网关地址和 API Key 后，可点「检测可用模型」自动拉取，再从下拉里选。
                  </Text>
                  <Button
                    size="xs"
                    variant="light"
                    leftSection={<IconRefresh size={14} />}
                    onClick={onDetect}
                    loading={busyAction === "detect"}
                  >
                    检测可用模型
                  </Button>
                </Group>
                <Autocomplete
                  label="Opus 档（复杂任务，最强）"
                  placeholder="如 glm-5.2 / claude-opus-4-7"
                  data={modelData}
                  value={form.opusModel}
                  onChange={(v) => setForm({ ...form, opusModel: v })}
                />
                <Autocomplete
                  label="Sonnet 档（日常默认）"
                  placeholder="如 glm-5.1 / claude-sonnet-4-6"
                  data={modelData}
                  value={form.sonnetModel}
                  onChange={(v) => setForm({ ...form, sonnetModel: v })}
                />
                <Autocomplete
                  label="Haiku 档（轻量、快速、后台子任务）"
                  placeholder="如 glm-5-turbo / claude-haiku-4-5"
                  data={modelData}
                  value={form.haikuModel}
                  onChange={(v) => setForm({ ...form, haikuModel: v })}
                />

                <Divider my={4} label="CA 证书（整机一次，所有实例共享）" labelPosition="left" />
                <Alert
                  variant="light"
                  color={env?.cert_imported ? "teal" : "yellow"}
                  icon={<IconCertificate size={16} />}
                  title={
                    env?.cert_imported
                      ? "已导入 CA 证书"
                      : "公司网关需先导入 CA 证书（否则连不上）"
                  }
                >
                  <Text size="sm" mb={6}>
                    公司网关用自签名证书。把管理员给的 <Code>ca-cert.pem</Code>{" "}
                    填上完整路径后点「导入」，整机生效一次、所有实例共用，无需每个实例都弄。
                  </Text>
                  <Group align="flex-end" gap="xs">
                    <TextInput
                      style={{ flex: 1 }}
                      size="xs"
                      placeholder={certPlaceholder}
                      value={certPath}
                      onChange={(e) => setCertPath(e.currentTarget.value)}
                    />
                    <Button size="xs" onClick={onImportCert} loading={busyAction === "import"}>
                      导入
                    </Button>
                  </Group>
                </Alert>
              </>
            )}

            <Text size="sm" fw={500} mt={4}>
              共享（可选）
            </Text>
            <Text size="xs" c="dimmed">
              打开后，点「保存」时会自动把主账户的 skills / plugins 链接给这个实例。
            </Text>
            <Group gap="xl" mt={4}>
              <Switch
                label="共享主账户的 skills"
                checked={form.shareSkills}
                onChange={(e) =>
                  setForm({ ...form, shareSkills: e.currentTarget.checked })
                }
              />
              <Switch
                label="共享主账户的 plugins"
                checked={form.sharePlugins}
                onChange={(e) =>
                  setForm({ ...form, sharePlugins: e.currentTarget.checked })
                }
              />
            </Group>

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
      </Grid.Col>
    </Grid>
  );
}
