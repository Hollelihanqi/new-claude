import { useEffect, useRef, useState } from "react";
import { Alert, Button, Card, Checkbox, Group, Modal, Select, Stack, Text, TextInput, Title } from "@mantine/core";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type ChatGptState, type ChatGptHistory, type ChatGptPreview, type ChatGptTransferRequest, type ChatGptPending, type ChatGptJob } from "../api";

export default function ChatGptPanelHistory({ state, active, initialCopy, disabled, onBusy }: {
  state: ChatGptState; active: boolean; initialCopy: { source: string | null; target: string } | null; disabled: boolean; onBusy: (busy: boolean) => void;
}) {
  const [source, setSource] = useState<string | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const [history, setHistory] = useState<ChatGptHistory | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState("");
  const [busy, setBusy] = useState("");
  const busyRef = useRef(false), alive = useRef(true), historyGen = useRef(0), targetGen = useRef(0), stop = useRef(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [previews, setPreviews] = useState<ChatGptPreview[] | null>(null);
  const [pending, setPending] = useState<ChatGptPending[]>([]);
  const [jobs, setJobs] = useState<ChatGptJob[]>([]);
  const [discarding, setDiscarding] = useState<ChatGptPending | null>(null);
  useEffect(() => { alive.current = true; return () => { alive.current = false; historyGen.current++; targetGen.current++; stop.current = true; }; }, []);

  async function load(sourceId: string | null) {
    const generation = ++historyGen.current;
    setSource(sourceId); setSelected([]); setPreviews(null); setHistory(null); setError("");
    if (!sourceId) return;
    try {
      const value = await api.chatGptHistory(sourceId);
      if (alive.current && generation === historyGen.current) setHistory(value);
    } catch (e) { if (alive.current && generation === historyGen.current) setError(String(e)); }
  }
  async function loadTarget(targetId: string | null) {
    const generation = ++targetGen.current;
    setTarget(targetId); setPending([]); setJobs([]);
    if (!targetId) return;
    try {
      const [p, j] = await Promise.all([api.chatGptPending(targetId), api.chatGptBatchList(targetId)]);
      if (alive.current && generation === targetGen.current) { setPending(p); setJobs(j); }
    } catch (e) { if (alive.current && generation === targetGen.current) setError(String(e)); }
  }
  useEffect(() => {
    if (initialCopy) { void load(initialCopy.source); void loadTarget(initialCopy.target); }
  }, [initialCopy]);
  useEffect(() => {
    if (target && !state.profiles.some(p => p.id === target)) void loadTarget(null);
    if (source && source !== "default" && !state.profiles.some(p => p.id === source)) void load(null);
  }, [state.profiles, target, source]);

  async function run(label: string, operation: () => Promise<void>) {
    if (busyRef.current || disabled) return;
    busyRef.current = true; stop.current = false; setBusy(label); onBusy(true); setError(""); setMessage("");
    try { await operation(); } catch (e) { if (alive.current) setError(String(e)); }
    finally { busyRef.current = false; if (alive.current) { setBusy(""); onBusy(false); if (target) void loadTarget(target); } }
  }
  const locked = disabled || !!busy;
  const sourceProfile = state.profiles.find(p => p.id === source);
  const targetProfile = state.profiles.find(p => p.id === target);
  const ready = !!source && !!targetProfile && source !== target && targetProfile.status === "stopped"
    && (source === "default" || sourceProfile?.status === "stopped") && !!state.installation?.compatible && !!state.installation.cli;
  const request = (key: string, fingerprint?: string): ChatGptTransferRequest => ({ sourceId: source!, targetId: target!, key,
    revision: history!.items.find(i => i.key === key)!.revision, ...(workspace.trim() ? { workspace: workspace.trim() } : {}), ...(fingerprint ? { fingerprint } : {}) });

  async function inspect(keys: string[]) {
    await run("检查记录与附件", async () => {
      if (!ready || keys.length === 0 || keys.length > 100) return;
      const results = [];
      for (const key of keys) results.push(await api.chatGptPreview(request(key)));
      if (alive.current) { setSelected(keys); setPreviews(results); }
    });
  }
  async function drive(job: ChatGptJob) {
    job = { ...job, lastError: null };
    while (!job.cancelled && !job.lastError && job.outcomes.length < job.requests.length && !stop.current && alive.current) {
      setBusy(`正在复制 ${job.outcomes.length + 1}/${job.requests.length}`);
      job = await api.chatGptBatchStep(job.targetId, job.id);
      if (alive.current) setJobs(old => [job, ...old.filter(j => j.id !== job.id)]);
    }
    if (alive.current) {
      if (job.lastError) setError(`复制中断：${job.lastError}。修复问题后可继续队列。`);
      else setMessage(stop.current ? "复制已暂停，已完成的副本保留，可从队列继续。" : `已处理 ${job.outcomes.length} 条，成功 ${job.outcomes.filter(o => o.result).length} 条。详情见复制队列。`);
    }
  }
  const unfinished = jobs.filter(j => !j.cancelled && j.outcomes.length < j.requests.length);
  return <Card withBorder radius="md" p="lg"><Stack gap="md">
    <div><Title order={4}>复制工作记录</Title><Text size="sm" c="dimmed">复制为独立副本，两边的后续进展分别保留。每批最多 100 条，可暂停或恢复。</Text></div>
    <Alert color="blue">复制前请关闭来源和目标实例。支持本地分页、分支、压缩历史与本地图片；云端任务及受账号权限控制的附件无法直接迁移。</Alert>
    {error && <Alert role="alert" color="red">{error}</Alert>}
    {message && <Alert role="status" color="teal">{message}</Alert>}
    <Group grow align="flex-start">
      <Select label="来源实例" clearable disabled={locked} value={source} data={[{ value: "default", label: "默认 ChatGPT（只读来源）" }, ...state.profiles.map(p => ({ value: p.id, label: p.name }))]}
        onChange={value => { if (value === target) void loadTarget(null); void load(value); }} />
      <Select label="目标实例" clearable disabled={locked} value={target} data={state.profiles.filter(p => p.id !== source).map(p => ({ value: p.id, label: p.name }))} onChange={value => { setPreviews(null); void loadTarget(value); }} />
    </Group>
    <Group align="end"><TextInput label="接续项目目录（可选）" description="留空使用原目录。两边使用同一目录时会修改同一份项目文件。" style={{ flex: 1 }} value={workspace} disabled={locked} onChange={e => { setWorkspace(e.currentTarget.value); setPreviews(null); }} />
      <Button variant="default" disabled={locked} onClick={() => void run("选择项目目录", async () => { const path = await open({ directory: true, multiple: false }); if (typeof path === "string" && alive.current) { setWorkspace(path); setPreviews(null); } })}>选择目录</Button></Group>
    {source && <Group><Button variant="subtle" disabled={locked} onClick={() => void load(source)}>重新读取记录</Button>
      <Button variant="subtle" disabled={locked || !history} onClick={() => setSelected(history!.items.filter(i => i.transferable).slice(0,100).map(i => i.key))}>选择前 100 条可复制记录</Button>
      <Button variant="subtle" disabled={locked} onClick={() => setSelected([])}>清空选择</Button></Group>}
    {history?.warnings.map(w => <Text key={w} size="xs" c="dimmed">{w}</Text>)}
    {source && !history && !error && <Text role="status">正在读取工作记录…</Text>}
    {history?.items.length === 0 && <Text c="dimmed">没有本地工作记录。</Text>}
    {history?.items.map(item => <Card key={item.key} withBorder p="sm"><Group justify="space-between" wrap="nowrap">
      <Checkbox label={item.title} description={`${new Date(item.modifiedAt * 1000).toLocaleString()} · ${item.detail}`} checked={selected.includes(item.key)} disabled={locked || !item.transferable || (!selected.includes(item.key) && selected.length >= 100)}
        onChange={e => { const checked = e.currentTarget.checked; setSelected(old => checked ? [...old,item.key] : old.filter(k => k !== item.key)); setPreviews(null); }} />
      <Button size="xs" variant="light" disabled={locked || !ready || !item.transferable} onClick={() => void inspect([item.key])}>复制到目标</Button>
    </Group></Card>)}
    <Group><Button disabled={locked || !ready || selected.length === 0} onClick={() => void inspect(selected)}>预览选中的 {selected.length} 条记录</Button>{busy && <Text role="status" size="sm">{busy}</Text>}</Group>
    {source && target && !ready && <Text size="sm" c="orange">来源和目标需已关闭，且客户端包含兼容的会话服务。</Text>}
    {target && <Stack gap="xs"><Group justify="space-between"><Title order={5}>复制队列与中断恢复</Title><Button size="xs" variant="subtle" disabled={locked} onClick={() => void loadTarget(target)}>刷新队列</Button></Group>
      {pending.map(p => <Card key={p.key} withBorder p="sm"><Text>{p.title || "未完成的副本"}</Text><Group mt="xs">
        <Button size="xs" disabled={locked || targetProfile?.status !== "stopped"} onClick={() => void run("恢复副本", async () => { const value = await api.chatGptRecover(target,p.key,false); if (alive.current) setMessage(value.detail); })}>继续恢复</Button>
        <Button size="xs" color="red" variant="light" disabled={locked || targetProfile?.status !== "stopped"} onClick={() => setDiscarding(p)}>撤回未完成副本</Button></Group></Card>)}
      {unfinished.map(job => <Card key={job.id} withBorder p="sm"><Text size="sm">已处理 {job.outcomes.length}/{job.requests.length} 条</Text>{job.lastError && <Text size="sm" c="red">上次中断：{job.lastError}</Text>}<Group mt="xs">
        <Button size="xs" disabled={locked || targetProfile?.status !== "stopped"} onClick={() => void run("恢复批量复制", () => drive(job))}>继续队列</Button>
        <Button size="xs" variant="light" disabled={locked} onClick={() => void run("取消剩余复制", async () => { await api.chatGptBatchStep(target,job.id,true); })}>取消剩余复制</Button></Group></Card>)}
      {busy.startsWith("正在复制") && <Button variant="light" onClick={() => { stop.current = true; setMessage("当前记录处理完后暂停。"); }}>暂停后续复制</Button>}
      {jobs.filter(j => j.outcomes.length > 0).slice(0,10).map(job => <details key={job.id}><summary>复制结果：{job.outcomes.filter(o => o.result).length} 成功，{job.outcomes.filter(o => o.error).length} 失败{job.cancelled ? "（已取消剩余）" : ""}</summary>
        {job.outcomes.map(o => <Text key={o.key} size="sm" c={o.error ? "red" : "dimmed"}>{history?.items.find(i => i.key === o.key)?.title ?? o.key.slice(0,12)}：{o.error ?? (o.result?.duplicate ? "已有独立副本，未覆盖" : "复制成功")}</Text>)}
      </details>)}
      {pending.length === 0 && unfinished.length === 0 && <Text size="sm" c="dimmed">没有未完成的复制任务。</Text>}
    </Stack>}
    <Modal opened={active && previews !== null} onClose={() => { if (!locked) setPreviews(null); }} title="确认复制范围" centered>
      <Stack><Text>{source === "default" ? "默认 ChatGPT" : sourceProfile?.name} → {targetProfile?.name}</Text>
        {previews?.map(p => <div key={p.key}><Text fw={600}>{p.title}</Text><Text size="sm">{Math.ceil(p.bytes/1024)} KB · {p.images} 张本地图片 · 项目：{p.workspace}</Text></div>)}
        <Text size="sm">历史与列出的图片将复制到目标实例，续聊时可能由目标账号发送处理。账号凭证及工具授权保持独立；项目文件不会随记录复制。</Text>
        <Button disabled={locked || !ready} onClick={() => void run("创建复制队列", async () => { if (!previews) return; const job = await api.chatGptBatchCreate(previews.map(p => request(p.key,p.fingerprint))); setPreviews(null); await drive(job); })}>创建独立副本</Button>
      </Stack>
    </Modal>
    <Modal opened={active && discarding !== null} onClose={() => { if (!locked) setDiscarding(null); }} title="撤回未完成副本" centered><Stack>
      <Text>仅删除这次尚未完成的目标副本与临时快照。来源及其他已完成的记录保留。</Text>
      <Alert color="orange">如果你曾打开并修改过这份未完成副本，撤回也会删除其中的修改。</Alert>
      <Button color="red" disabled={locked} onClick={() => void run("撤回复制", async () => { if (!target || !discarding) return; const value = await api.chatGptRecover(target,discarding.key,true); setDiscarding(null); setMessage(value.detail); })}>确认撤回</Button>
    </Stack></Modal>
  </Stack></Card>;
}
