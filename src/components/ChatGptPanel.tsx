import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Alert, Badge, Button, Card, Group, Modal, Select, SimpleGrid, Stack, Text, TextInput, Title } from "@mantine/core";
import { IconBrandOpenai, IconFolderOpen, IconPlayerPlay, IconPlus, IconRefresh, IconShieldLock } from "@tabler/icons-react";
import { api, type ChatGptAction, type ChatGptProfile, type ChatGptState } from "../api";
import RiskConfirm from "./RiskConfirm";
import ChatGptPanelHistory from "./ChatGptPanelHistory";

const STATUS = {
  running: { label: "运行中", color: "teal" },
  stopped: { label: "已退出", color: "gray" },
  closing: { label: "后台进程仍在运行", color: "orange" },
  error: { label: "需要处理", color: "red" },
};

export default function ChatGptPanel({ active = true }: { active?: boolean }) {
  const [state, setState] = useState<ChatGptState | null>(null);
  const [busy, setBusy] = useState("");
  const busyRef = useRef(false);
  const stateGeneration = useRef(0);
  const alive = useRef(true);
  const [message, setMessage] = useState<{ error: boolean; text: string } | null>(null);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [deleting, setDeleting] = useState<ChatGptProfile | null>(null);
  const [copySource, setCopySource] = useState<string | null>(null);
  const [initialCopy, setInitialCopy] = useState<{ source: string | null; target: string } | null>(null);

  useEffect(() => { alive.current = true; return () => { alive.current = false; stateGeneration.current++; }; }, []);
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
    if (kind === "launch" || kind === "focus") setMessage(null);
    else setMessage({ error: false, text: kind === "stop"
      ? next.profiles.find(item => item.id === p.id)?.status === "stopped"
        ? "实例已退出，登录和记录保留在独立目录中。"
        : "已请求退出该实例。若仍有后台进程，请在 ChatGPT 中选择退出后刷新。"
      : kind === "cleanup" ? "已检查并清理该实例遗留的崩溃报告进程。"
      : kind === "delete" ? "实例数据已删除，其他实例及已复制的记录保持完整。"
      : "操作已完成。" });
    if (kind === "delete") setDeleting(null);
  });

  const pick = () => run("pick", async () => {
    if (!state) return;
    const path = await open({ title: state.pickerTitle, multiple: false, filters: [{ name: "ChatGPT", extensions: state.pickerExtensions }] });
    if (typeof path === "string") { const next = await api.chatGptSetInstallation(path); if (alive.current) setState(next); }
  });

  const save = () => run("save", async () => {
    if (!creating || !name.trim()) return;
    const next = await api.chatGptCreateProfile(name.trim());
    if (alive.current) {
      const added = next.profiles.find(p => !state?.profiles.some(old => old.id === p.id));
      if (added) setInitialCopy({ source: copySource, target: added.id });
      setState(next); setCreating(false); setName(""); setCopySource(null);
    }
  });

  const profiles = state?.profiles ?? [];

  return <div className="view-scroll chatgpt-scroll"><Stack className={`chatgpt-page${state && profiles.length === 0 ? " chatgpt-page-empty" : ""}`} gap="lg">
    <section className="chatgpt-console" aria-label="ChatGPT 客户端与实例">
      <div className="chatgpt-console-top">
        <div className="chatgpt-client-identity">
          <span className="chatgpt-client-icon"><IconBrandOpenai size={24} /></span>
          <div className="chatgpt-client-copy">
            <Group gap="xs"><Text fw={700}>{state?.installation ? `ChatGPT ${state.installation.version}` : "官方 ChatGPT 客户端"}</Text><Badge size="sm" variant="light" color={state?.installation?.compatible ? "teal" : "orange"}>{state?.installation?.compatible ? "已连接" : "待检查"}</Badge></Group>
            <Text size="xs" c="dimmed" className="chatgpt-path">{state?.installation?.path ?? state?.installationIssue ?? "正在检测客户端…"}</Text>
            {state?.installation && !state.installation.compatible && <Text size="xs" c="red">{state.installation.detail}</Text>}
          </div>
        </div>
        <Group gap="xs" className="chatgpt-console-actions">
          <Button variant="subtle" leftSection={<IconFolderOpen size={16} />} loading={busy === "pick"} disabled={!!busy && busy !== "pick"} onClick={() => void pick()}>选择客户端</Button>
          <Button variant="subtle" leftSection={<IconRefresh size={16} />} disabled={!!busy} onClick={() => void refresh()}>刷新</Button>
          {profiles.length > 0 && <Button leftSection={<IconPlus size={16} />} disabled={!!busy} onClick={() => { setCreating(true); setName(""); }}>创建实例</Button>}
        </Group>
      </div>
      {state && profiles.length === 0 && <div className="chatgpt-onboarding">
        <div className="chatgpt-onboarding-art" aria-hidden="true"><span><IconBrandOpenai size={28} /></span><span><IconBrandOpenai size={28} /></span></div>
        <Title order={3}>让两个账号，各自就位</Title>
        <Text size="sm" c="dimmed">创建独立实例后，分别在官方窗口登录即可。</Text>
        <Button size="md" leftSection={<IconPlus size={18} />} disabled={!!busy} onClick={() => { setCreating(true); setName(""); }}>创建第一个实例</Button>
      </div>}
    </section>

    {message && <Alert color={message.error ? "red" : "teal"} role={message.error ? "alert" : "status"}><Text size="sm" style={{ whiteSpace: "pre-line" }}>{message.text}</Text></Alert>}
    {!state ? <Text role="status">正在检测客户端与实例…</Text> : profiles.length > 0 && <>
      <section className="chatgpt-section" aria-label="账号实例">
        <div className="chatgpt-section-heading"><Title order={4}>账号实例</Title></div>
      <SimpleGrid cols={{ base: 1, md: 2 }}>
        {profiles.map((p) => <Card key={p.id} className="chatgpt-profile" p="lg">
          <Stack gap="sm">
            <Group justify="space-between" wrap="nowrap"><Group gap="sm" wrap="nowrap"><span className="chatgpt-profile-icon"><IconBrandOpenai size={20} /></span><div className="chatgpt-profile-name"><Text fw={700}>{p.name}</Text><Text size="xs" c="dimmed">独立账号数据</Text></div></Group><Badge color={STATUS[p.status].color} variant="light">{STATUS[p.status].label}</Badge></Group>
            <Text size="xs" c="dimmed" className="chatgpt-profile-path">{p.directory}</Text>
            {p.issue && <Alert color="red">{p.issue}</Alert>}
            <Group gap="xs" className="chatgpt-profile-actions">
              <Button size="xs" leftSection={<IconPlayerPlay size={14} />}
                loading={busy === `launch:${p.id}` || busy === `focus:${p.id}`}
                disabled={!!busy || p.status === "error" || p.status === "closing" || (p.status !== "running" && !state.installation?.compatible)}
                onClick={() => void action(p, p.status === "running" ? "focus" : "launch")}>{p.status === "running" ? "打开窗口" : "启动"}</Button>
              {p.status === "closing" && <Button size="xs" variant="default" disabled={!!busy} onClick={() => void action(p, "cleanup")}>清理后台进程</Button>}
              <Button size="xs" variant="default" disabled={!!busy || p.status !== "running"} onClick={() => void action(p, "stop")}>退出</Button>
              <Button size="xs" variant="subtle" disabled={!!busy || p.status !== "stopped"} onClick={() => void run(`diagnose:${p.id}`, async () => { const lines = await api.chatGptDiagnose(p.id); if (alive.current) setMessage({ error: false, text: `${p.name}\n${lines.join("\n")}` }); })}>检查隔离与账号</Button>
              <Button size="xs" variant="subtle" color="red" disabled={!!busy || p.status !== "stopped"} onClick={() => setDeleting(p)}>删除</Button>
            </Group>
          </Stack>
        </Card>)}
      </SimpleGrid>
      </section>

      <ChatGptPanelHistory state={state} active={active} initialCopy={initialCopy} disabled={!!busy && busy !== "history"} onBusy={(value) => { busyRef.current = value; setBusy(value ? "history" : ""); }} />
      <Text size="xs" c="dimmed" className="chatgpt-footnote"><IconShieldLock size={14} />账号与运行数据按实例保存。PathMux 退出后，已启动的 ChatGPT 窗口继续运行。</Text>
    </>}

    <Modal opened={active && creating} onClose={() => { if (!busy) setCreating(false); }} title="创建 ChatGPT 实例" centered>
      <Stack>
        <TextInput label="实例名称" placeholder="例如：工作账号" value={name} maxLength={40} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === "Enter" && !e.nativeEvent.isComposing && name.trim()) void save(); }} />
        <Select label="创建后选择本地记录（可选）" clearable value={copySource} onChange={setCopySource} data={[{ value: "default", label: "默认实例的本地 Codex 记录" }, ...profiles.map(p => ({ value: p.id, label: p.name }))]} />
        <Text size="sm" c="dimmed">账号登录在官方 ChatGPT 窗口中完成。创建后可选择复制本地 Codex 工作记录。</Text>
        <Group justify="flex-end"><Button loading={busy === "save"} disabled={!!busy || !name.trim()} onClick={() => void save()}>保存</Button></Group>
      </Stack>
    </Modal>
    <RiskConfirm opened={active && deleting !== null} level="high" title={`删除实例：${deleting?.name ?? ""}`}
      consequences={["仅可删除已退出的实例。将移除其独立目录：登录数据、本地会话、缓存、日志及未完成的复制队列。", "外部项目文件、其他实例和默认账号的数据不会删除。", "云端账号和云端记录不会被此操作删除。删除失败时会提示残留路径。"]}
      confirmLabel="删除实例数据" busy={!!busy} onCancel={() => { if (!busy) setDeleting(null); }} onConfirm={() => { if (deleting) void action(deleting, "delete"); }} />
  </Stack></div>;
}
