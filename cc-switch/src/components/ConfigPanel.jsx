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
} from "@mantine/core";
import {
  IconPlus,
  IconTrash,
  IconDeviceFloppy,
  IconLink,
  IconWorld,
  IconUser,
  IconInfoCircle,
} from "@tabler/icons-react";
import { api } from "../api.js";

const empty = {
  name: "",
  type: "router",
  baseUrl: "",
  shareSkills: false,
  sharePlugins: false,
};

export default function ConfigPanel({ onChanged }) {
  const [profiles, setProfiles] = useState([]);
  const [sel, setSel] = useState(null); // 选中的 name
  const [form, setForm] = useState(empty);
  const [token, setToken] = useState("");
  const [status, setStatus] = useState({ type: "info", msg: "" });
  const [busy, setBusy] = useState(false);

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
    });
    setToken("");
  };

  const onNew = () => {
    setSel(null);
    setForm(empty);
    setToken("");
    setStatus({ type: "info", msg: "填写名称（即命令词，如 corp）后保存。" });
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
    setBusy(true);
    try {
      const profile = {
        name: form.name.trim(),
        type: form.type,
        baseUrl: form.type === "router" ? form.baseUrl.trim() : "",
        shareSkills: form.shareSkills,
        sharePlugins: form.sharePlugins,
      };
      const msg = await api.saveProfile(profile, token || null);
      setToken("");
      load();
      onChanged && onChanged();
      setSel(profile.name);
      setStatus({
        type: "success",
        msg: `已保存「${profile.name}」。${msg} 之后在项目目录运行：claude ${profile.name}`,
      });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const onDelete = async () => {
    if (!sel) {
      setStatus({ type: "error", msg: "请先在左侧选中一个实例。" });
      return;
    }
    setBusy(true);
    try {
      await api.deleteProfile(sel);
      onNew();
      load();
      onChanged && onChanged();
      setStatus({ type: "success", msg: "已删除并更新终端集成。重开终端生效。" });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const onLink = async () => {
    const n = form.name.trim();
    if (!n) {
      setStatus({ type: "error", msg: "请先选中或填写一个实例。" });
      return;
    }
    setBusy(true);
    try {
      const msg = await api.syncLinks(n, form.shareSkills, form.sharePlugins);
      setStatus({ type: "success", msg });
    } catch (e) {
      setStatus({ type: "error", msg: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const isRouter = form.type === "router";

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
          <ScrollArea.Autosize mah={360}>
            <Stack gap={4}>
              {profiles.length === 0 && (
                <Text size="sm" c="dimmed" p="xs">
                  还没有实例。点「新建」创建第一个（比如 corp）。
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
            <Title order={5}>实例设置</Title>

            <TextInput
              label="实例名称 = 你要输入的命令词"
              description="例如填 corp，之后在终端用 claude corp。建议英文/数字、无空格。"
              placeholder="corp"
              value={form.name}
              onChange={(e) =>
                setForm({ ...form, name: e.currentTarget.value })
              }
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
                  placeholder="https://gateway.example.com"
                  value={form.baseUrl}
                  onChange={(e) =>
                    setForm({ ...form, baseUrl: e.currentTarget.value })
                  }
                />
                <PasswordInput
                  label="Token"
                  description="留空表示不修改。会加密存储（mac 钥匙串 / Windows DPAPI）。"
                  placeholder="••••••••"
                  value={token}
                  onChange={(e) => setToken(e.currentTarget.value)}
                />
              </>
            )}

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
                loading={busy}
              >
                保存并接入终端
              </Button>
              <Button
                variant="default"
                leftSection={<IconLink size={16} />}
                onClick={onLink}
                loading={busy}
              >
                同步 skills/plugins
              </Button>
              <Button
                variant="subtle"
                color="red"
                leftSection={<IconTrash size={16} />}
                onClick={onDelete}
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
              改了配置才需要重开一次，之后正常用。
            </Text>
          </Stack>
        </Card>
      </Grid.Col>
    </Grid>
  );
}
