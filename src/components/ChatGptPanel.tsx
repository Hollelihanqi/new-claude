import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Alert, Badge, Button, Card, Group, Modal, Select, SimpleGrid, Stack, Text, TextInput, Title } from "@mantine/core";
import { IconBrandOpenai, IconFolderOpen, IconPlayerPlay, IconPlus, IconRefresh, IconTransfer } from "@tabler/icons-react";
import { api, type ChatGptAction, type ChatGptHistory, type ChatGptHistoryItem, type ChatGptProfile, type ChatGptState } from "../api";
import RiskConfirm from "./RiskConfirm";

const STATUS = {
  running: { label: "运行中", color: "teal" },
  stopped: { label: "已关闭", color: "gray" },
  closing: { label: "后台进程仍在运行", color: "orange" },
  error: { label: "需要处理", color: "red" },
};

export default function ChatGptPanel({ active = true }: { active?: boolean }) {
  const [state, setState] = useState<ChatGptState | null>(null);
  const [busy, setBusy] = useState("");
  const busyRef = useRef(false);
  const stateGeneration = useRef(0);
  const historyGeneration = useRef(0);
  const alive = useRef(true);
  const [message, setMessage] = useState<{ error: boolean; text: string } | null>(null);
  const [editing, setEditing] = useState<ChatGptProfile | "new" | null>(null);
  const [name, setName] = useState("");
  const [deleting, setDeleting] = useState<ChatGptProfile | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const [history, setHistory] = useState<ChatGptHistory | null>(null);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [historyError, setHistoryError] = useState("");
  const [transferring, setTransferring] = useState<ChatGptHistoryItem | null>(null);

  useEffect(() => { alive.current = true; return () => { alive.current = false; stateGeneration.current++; historyGeneration.current++; }; }, []);
  const refresh = useCallback(async () => {
    if (busyRef.current) return;
    const generation = ++stateGeneration.current;
    try {
      const next = await api.chatGptState();
      if (alive.current && generation === stateGeneration.current) setState(next);
    } catch (error) {
      if (alive.current && generation === stateGeneration.current) setMessage({ error: true, text: String(error) });
    }
  }, []);
  useEffect(() => {
    if (!active) return;
    void refresh();
    const timer = setInterval(() => void refresh(), 15000);
    return () => clearInterval(timer);
  }, [active, refresh]);

  const run = async (label: string, operation: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true; stateGeneration.current++; setBusy(label); setMessage(null);
    try { await operation(); }
    catch (error) { if (alive.current) setMessage({ error: true, text: String(error) }); }
    finally { busyRef.current = false; if (alive.current) { setBusy(""); void refresh(); } }
  };

  const action = (p: ChatGptProfile, kind: ChatGptAction) => run(`${kind}:${p.id}`, async () => {
    const next = await api.chatGptProfileAction(p.id, kind);
    if (!alive.current) return;
    setState(next);
    setMessage({ error: false, text: kind === "stop"
      ? "已请求关闭该实例。若仍有后台进程，请在 ChatGPT 中选择退出后刷新。"
      : kind === "delete" ? "实例数据已删除，其他实例及已复制的记录保持完整。"
      : "已打开实例，请在官方窗口核对登录账号。首次登录请依次完成。" });
    if (kind === "delete") { setDeleting(null); if (source === p.id) { historyGeneration.current++; setSource(null); setHistory(null); setHistoryBusy(false); setTransferring(null); } if (target === p.id) setTarget(null); }
  });

  const loadHistory = async (sourceId: string | null) => {
    const generation = ++historyGeneration.current;
    setHistory(null); setHistoryError(""); setTransferring(null);
    if (!sourceId) { setHistoryBusy(false); return; }
    setHistoryBusy(true);
    try {
      const result = await api.chatGptHistory(sourceId);
      if (alive.current && generation === historyGeneration.current) setHistory(result);
    } catch (error) {
      if (alive.current && generation === historyGeneration.current) setHistoryError(String(error));
    } finally {
      if (alive.current && generation === historyGeneration.current) setHistoryBusy(false);
    }
  };

  const pick = () => run("pick", async () => {
    if (!state) return;
    const path = await open({ title: state.pickerTitle, multiple: false, filters: [{ name: "ChatGPT", extensions: state.pickerExtensions }] });
    if (typeof path === "string") { const next = await api.chatGptSetInstallation(path); if (alive.current) setState(next); }
  });

  const save = () => run("save", async () => {
    if (!editing || !name.trim()) return;
    const next = editing === "new" ? await api.chatGptCreateProfile(name.trim()) : await api.chatGptProfileAction(editing.id, "rename", name.trim());
    if (alive.current) { setState(next); setEditing(null); setName(""); }
  });

  const send = () => run("transfer", async () => {
    if (!source || !target || !transferring) return;
    const result = await api.chatGptTransfer({ sourceId: source, targetId: target, key: transferring.key, revision: transferring.revision });
    if (alive.current) { setMessage({ error: false, text: result.detail }); setTransferring(null); }
  });

  const profiles = state?.profiles ?? [];
  const sourceProfile = profiles.find((p) => p.id === source);
  const targetProfile = profiles.find((p) => p.id === target);
  const sourceLabel = source === "default" ? "默认 ChatGPT" : sourceProfile?.name ?? "";
  const canTransfer = !!targetProfile && targetProfile.status === "stopped" && source !== target
    && (source === "default" || sourceProfile?.status === "stopped") && !!state?.installation?.cli && !!state?.installation?.compatible;

  return <Stack gap="lg">
    <Group justify="space-between" align="flex-start">
      <div>
        <Group gap="xs"><IconBrandOpenai size={24} /><Title order={3}>ChatGPT 多开</Title><Badge color="orange" variant="light">验证阶段</Badge></Group>
        <Text c="dimmed" size="sm" mt={6}>各账号分别登录、同时使用。按需复制工作记录，在另一实例继续。</Text>
      </div>
      <Group gap="xs">
        <Button variant="default" leftSection={<IconRefresh size={16} />} disabled={!!busy} onClick={() => void refresh()}>刷新</Button>
        <Button leftSection={<IconPlus size={16} />} disabled={!!busy || !state} onClick={() => { setEditing("new"); setName(""); }}>创建实例</Button>
      </Group>
    </Group>

    {message && <Alert color={message.error ? "red" : "teal"} role={message.error ? "alert" : "status"}>{message.text}</Alert>}
    {!state ? <Text role="status">正在检测客户端与实例…</Text> : <>
      <Card withBorder radius="md" p="lg">
        <Group justify="space-between" wrap="wrap">
          <Stack gap={4} style={{ flex: 1, minWidth: 200 }}>
            <Text fw={600}>{state.installation ? `ChatGPT ${state.installation.version}` : "选择官方客户端"}</Text>
            <Text size="sm" c="dimmed" style={{ overflowWrap: "anywhere" }}>{state.installation?.path ?? state.installationIssue}</Text>
            {state.installation && <Text size="sm" c={state.installation.compatible ? "dimmed" : "red"}>{state.installation.detail}</Text>}
          </Stack>
          <Button variant="light" leftSection={<IconFolderOpen size={16} />} loading={busy === "pick"} disabled={!!busy && busy !== "pick"} onClick={() => void pick()}>选择客户端</Button>
        </Group>
      </Card>

      {profiles.length === 0 ? <Card withBorder radius="md" p="xl">
        <Stack align="center" gap="xs">
          <Title order={4}>为每个账号创建一个实例</Title>
          <Text c="dimmed" size="sm">例如“个人”和“工作”。创建后打开官方窗口，分别完成登录。</Text>
        </Stack>
      </Card> : <SimpleGrid cols={{ base: 1, md: 2 }}>
        {profiles.map((p) => <Card key={p.id} withBorder radius="md" p="lg">
          <Stack gap="sm">
            <Group justify="space-between"><Text fw={600}>{p.name}</Text><Badge color={STATUS[p.status].color} variant="light">{STATUS[p.status].label}</Badge></Group>
            <Text size="xs" c="dimmed">实例名称为自定义标签，登录账号请在 ChatGPT 窗口中核对。</Text>
            <Text size="xs" c="dimmed" style={{ overflowWrap: "anywhere" }}>{p.directory}</Text>
            {p.issue && <Alert color="red">{p.issue}</Alert>}
            <Group gap="xs">
              <Button size="xs" leftSection={<IconPlayerPlay size={14} />}
                loading={busy === `launch:${p.id}` || busy === `focus:${p.id}`}
                disabled={!!busy || p.status === "error" || p.status === "closing" || (p.status !== "running" && !state.installation?.compatible)}
                onClick={() => void action(p, p.status === "running" ? "focus" : "launch")}>{p.status === "running" ? "打开窗口" : "启动"}</Button>
              <Button size="xs" variant="default" disabled={!!busy || p.status !== "running"} onClick={() => void action(p, "stop")}>关闭</Button>
              <Button size="xs" variant="subtle" disabled={!!busy} onClick={() => { setEditing(p); setName(p.name); }}>改名</Button>
              <Button size="xs" variant="subtle" color="red" disabled={!!busy || p.status !== "stopped"} onClick={() => setDeleting(p)}>删除</Button>
            </Group>
          </Stack>
        </Card>)}
      </SimpleGrid>}

      <Card withBorder radius="md" p="lg">
        <Stack gap="md">
          <div><Title order={4}>复制工作记录</Title><Text size="sm" c="dimmed" mt={4}>选定一条本地记录，创建独立副本。双方的后续进展分别保留。</Text></div>
          <Alert color="blue">导入前需关闭来源和目标实例。分页历史、云端记录及外部附件暂不支持完整迁移；记录列表会标明限制。首次跨账号续聊仍需实际验证。</Alert>
          <SimpleGrid cols={{ base: 1, sm: 2 }}>
            <Select label="来源实例" placeholder="选择要读取的工作记录" value={source} disabled={!!busy}
              data={[{ value: "default", label: "默认 ChatGPT（只读来源）" }, ...profiles.map((p) => ({ value: p.id, label: p.name }))]}
              onChange={(value) => { setSource(value); if (value === target) setTarget(null); void loadHistory(value); }} />
            <Select label="目标实例" placeholder="选择接续账号所在实例" value={target} disabled={!!busy}
              data={profiles.filter((p) => p.id !== source).map((p) => ({ value: p.id, label: p.name }))}
              onChange={(value) => { setTarget(value); setTransferring(null); }} />
          </SimpleGrid>
          {source && <Group justify="space-between"><Text size="sm" c="dimmed">{history ? `${history.items.length} 条记录` : ""}</Text><Button variant="subtle" size="xs" loading={historyBusy} disabled={!!busy} onClick={() => void loadHistory(source)}>重新读取记录</Button></Group>}
          {historyError && <Alert color="red" role="alert">{historyError}</Alert>}
          {historyBusy && <Text role="status" size="sm">正在读取工作记录…</Text>}
          {history?.warnings.map((w) => <Text key={w} c="dimmed" size="xs">{w}</Text>)}
          {history && history.items.length === 0 && <Text c="dimmed" size="sm">没有找到可列出的本地工作记录。</Text>}
          {history?.items.map((item) => <Card key={item.key} withBorder radius="sm" p="sm">
            <Group justify="space-between" align="flex-start" wrap="wrap">
              <Stack gap={3} style={{ flex: 1, minWidth: 180 }}>
                <Text size="sm" fw={500} style={{ overflowWrap: "anywhere" }}>{item.title}</Text>
                <Text size="xs" c="dimmed">{new Date(item.modifiedAt * 1000).toLocaleString()} · {Math.ceil(item.bytes / 1024)} KB</Text>
                <Text size="xs" c={item.transferable ? "dimmed" : "orange"}>{item.detail}</Text>
              </Stack>
              <Button size="xs" variant="light" leftSection={<IconTransfer size={14} />} disabled={!!busy || !canTransfer || !item.transferable} onClick={() => setTransferring(item)}>复制到目标</Button>
            </Group>
          </Card>)}
          {source && target && !canTransfer && <Text size="xs" c="orange">请确认来源和目标已关闭，且已选择包含会话服务的兼容客户端。</Text>}
        </Stack>
      </Card>
      <Text size="xs" c="dimmed">账号与运行数据按实例保存；同一系统用户仍可访问本机文件。PathMux 关闭后，已启动的 ChatGPT 窗口继续运行。</Text>
    </>}

    <Modal opened={active && editing !== null} onClose={() => { if (!busy) setEditing(null); }} title={editing === "new" ? "创建 ChatGPT 实例" : "修改实例名称"} centered>
      <Stack>
        <TextInput label="实例名称" placeholder="例如：工作账号" value={name} maxLength={40} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === "Enter" && !e.nativeEvent.isComposing && name.trim()) void save(); }} />
        <Text size="sm" c="dimmed">账号登录在官方 ChatGPT 窗口中完成。创建后可从“复制工作记录”导入已有历史。</Text>
        <Group justify="flex-end"><Button loading={busy === "save"} disabled={!!busy || !name.trim()} onClick={() => void save()}>保存</Button></Group>
      </Stack>
    </Modal>
    <RiskConfirm opened={active && deleting !== null} level="high" title={`删除实例：${deleting?.name ?? ""}`}
      consequences={["永久删除此实例的本地记录、缓存及保存在实例内的登录数据。", "其他实例、默认账号及已复制到其他实例的记录保持完整。", "云端账号和云端记录不会被此操作删除。"]}
      confirmLabel="删除实例数据" busy={!!busy} onCancel={() => { if (!busy) setDeleting(null); }} onConfirm={() => { if (deleting) void action(deleting, "delete"); }} />
    <Modal opened={active && transferring !== null} onClose={() => { if (!busy) setTransferring(null); }} title="复制到另一实例" centered>
      <Stack>
        <Text>{sourceLabel} → {targetProfile?.name}</Text>
        <Text fw={500}>{transferring?.title}</Text>
        <Text size="sm">目标实例将得到独立的对话副本。续聊时，对话内容可能由目标账号发送处理。原项目文件仍在原工作目录，两边继续工作会形成各自的记录。</Text>
        <Text size="sm" c="dimmed">登录凭证和工具授权保持独立。当前版本只接受能独立读取且通过附件检查的历史格式。</Text>
        <Button loading={busy === "transfer"} disabled={!!busy || !canTransfer} onClick={() => void send()}>创建独立副本</Button>
      </Stack>
    </Modal>
  </Stack>;
}
