import { useEffect, useRef, useState } from "react";
import { Alert, Button, Card, Checkbox, Group, Modal, Select, Stack, Text, TextInput, Title } from "@mantine/core";
import { IconHistory, IconLoader2 } from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type ChatGptState, type ChatGptHistory, type ChatGptPreview, type ChatGptTransferRequest, type ChatGptPending, type ChatGptJob } from "../api";

export default function ChatGptPanelHistory({ state, active, initialCopy, coordinate }: {
  state: ChatGptState; active: boolean; initialCopy: { source: string | null; target: string } | null; coordinate: (operation: () => Promise<void>) => Promise<void>;
}) {
  const [source, setSource] = useState<string | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const [history, setHistory] = useState<ChatGptHistory | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [scope, setScope] = useState<"selected" | "project" | "all">("selected");
  const [project, setProject] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [loadedPair, setLoadedPair] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [busy, setBusy] = useState("");
  const [pendingActions, setPendingActions] = useState<string[]>([]);
  const pendingActionsRef = useRef(new Set<string>());
  const alive = useRef(true), historyGen = useRef(0), targetGen = useRef(0), stop = useRef(false), restoredPair = useRef(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [previews, setPreviews] = useState<ChatGptPreview[] | null>(null);
  const [pending, setPending] = useState<ChatGptPending[]>([]);
  const [jobs, setJobs] = useState<ChatGptJob[]>([]);
  const [discarding, setDiscarding] = useState<ChatGptPending | null>(null);
  const [confirmClosingTarget, setConfirmClosingTarget] = useState<string | null>(null);
  useEffect(() => { alive.current = true; return () => { alive.current = false; historyGen.current++; targetGen.current++; stop.current = true; }; }, []);

  async function load(sourceId: string | null) {
    const generation = ++historyGen.current;
    setSource(sourceId); if (sourceId !== source) { setSelected([]); setProject(null); } setPreviews(null); setHistory(null); setError("");
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
    if (restoredPair.current || initialCopy) return;
    restoredPair.current = true;
    try {
      const saved = JSON.parse(localStorage.getItem("pathmux:chatgpt-last-pair") ?? "null");
      const savedSource = saved?.source === "default" || state.profiles.some(p => p.id === saved?.source) ? saved.source : null;
      const savedTarget = state.profiles.some(p => p.id === saved?.target) ? saved.target : null;
      if (savedSource && savedTarget && savedSource !== savedTarget) { void load(savedSource); void loadTarget(savedTarget); }
    } catch { /* A missing or old preference leaves the selectors empty. */ }
  }, [initialCopy, state.profiles]);
  useEffect(() => {
    if (!source || !target || source === target) return;
    try { localStorage.setItem("pathmux:chatgpt-last-pair", JSON.stringify({ source, target })); } catch { /* The current selection still works. */ }
  }, [source, target]);
  useEffect(() => {
    if (target && !state.profiles.some(p => p.id === target)) void loadTarget(null);
    if (source && source !== "default" && !state.profiles.some(p => p.id === source)) void load(null);
  }, [state.profiles, target, source]);
  useEffect(() => {
    const pair = source && target ? `${source}:${target}` : "";
    if (!pair) { setLoadedPair(""); return; }
    try {
      const saved = JSON.parse(localStorage.getItem(`pathmux:chatgpt-sync:${pair}`) ?? "null");
      setScope(["selected", "project", "all"].includes(saved?.scope) ? saved.scope : "selected");
      setProject(typeof saved?.project === "string" ? saved.project : null);
      setSelected(Array.isArray(saved?.selected) ? saved.selected.filter((key: unknown) => typeof key === "string").slice(0,100) : []);
    } catch { setScope("selected"); setProject(null); setSelected([]); }
    setLoadedPair(pair);
  }, [source, target]);
  useEffect(() => {
    const pair = source && target ? `${source}:${target}` : "";
    if (!pair || loadedPair !== pair) return;
    try { localStorage.setItem(`pathmux:chatgpt-sync:${pair}`, JSON.stringify({ scope, project, selected })); } catch { /* Storage may be disabled; the current selection still works. */ }
  }, [source, target, loadedPair, scope, project, selected]);

  async function run(label: string, operation: () => Promise<unknown>) {
    if (pendingActionsRef.current.has(label)) return;
    pendingActionsRef.current.add(label);
    setPendingActions([...pendingActionsRef.current]);
    await coordinate(async () => {
      stop.current = false; setBusy(label); setError(""); setMessage("");
      try { await operation(); } catch (e) { if (alive.current) setError(String(e)); }
      finally {
        pendingActionsRef.current.delete(label);
        if (alive.current) { setPendingActions([...pendingActionsRef.current]); setBusy(""); if (target) void loadTarget(target); }
      }
    });
  }
  const actionPending = (label: string) => pendingActions.includes(label);
  const locked = !!busy;
  const sourceProfile = state.profiles.find(p => p.id === source);
  const targetProfile = state.profiles.find(p => p.id === target);
  const ready = !!source && !!targetProfile && source !== target && targetProfile.status === "stopped"
    && !!state.installation?.compatible && !!state.installation.cli;
  const request = (key: string, fingerprint?: string): ChatGptTransferRequest => ({ sourceId: source!, targetId: target!, key,
    revision: history!.items.find(i => i.key === key)!.revision, ...(workspace.trim() ? { workspace: workspace.trim() } : {}), ...(fingerprint ? { fingerprint } : {}) });

  async function inspect(keys: string[]) {
    await run(`检查记录与附件:${keys.join(",")}`, async () => {
      if (!ready || keys.length === 0 || keys.length > 100) return;
      const results = [];
      for (const key of keys) results.push(await api.chatGptPreview(request(key)));
      if (alive.current) { setSelected(keys); setPreviews(results); }
    });
  }
  async function drive(job: ChatGptJob): Promise<ChatGptJob> {
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
    return job;
  }
  const projects = [...new Set(history?.items.map(i => i.workspace).filter(Boolean) ?? [])].sort();
  const scopeItems = scope === "selected" ? history?.items.filter(i => selected.includes(i.key)) ?? []
    : scope === "project" ? history?.items.filter(i => i.workspace === project) ?? [] : history?.items ?? [];
  const visibleItems = (history?.items ?? []).filter(i => !query || `${i.title} ${i.workspace}`.toLowerCase().includes(query.toLowerCase()));
  async function syncAndSwitch(confirmedTarget: string | null = null) {
    await run("准备同步", async () => {
      if (!source || !target || !history || source === target || !state.installation?.compatible || !state.installation.cli) return;
      if (scope === "all" && workspace.trim()) throw new Error("全部会话包含不同项目，请清空接续项目目录后再同步。");
      if (scope !== "selected" && !history.complete) throw new Error("记录扫描未完整完成，不能保证同步整个项目或全部记录。请先处理列表中的提示。");
      if (scopeItems.length === 0) throw new Error("请先选择会话、项目或全部记录。");
      const unsupported = scopeItems.filter(i => !i.transferable);
      if (unsupported.length) throw new Error(`所选范围中有 ${unsupported.length} 条记录暂不能迁移。请改选可迁移的会话。`);
      const current = await api.chatGptState();
      if (current.profiles.some(p => p.id === target && (p.status === "running" || p.status === "background")) && confirmedTarget !== target) {
        setConfirmClosingTarget(target);
        return;
      }
      const plans: ChatGptTransferRequest[] = [];
      for (let index = 0; index < scopeItems.length; index++) {
        if (stop.current) break;
        setBusy(`正在检查 ${index + 1}/${scopeItems.length}`);
        const item = scopeItems[index];
        const preview = await api.chatGptPreview(request(item.key));
        plans.push(request(item.key, preview.fingerprint));
      }
      if (stop.current) { setMessage("已暂停，尚未建立新的复制队列。"); return; }
      const latest = await api.chatGptState();
      const status = latest.profiles.find(p => p.id === target)?.status;
      if (status === "running" || status === "background") {
        if (confirmedTarget !== target) {
          setConfirmClosingTarget(target);
          return;
        }
        setBusy("正在关闭目标实例");
        await api.chatGptProfileAction(target, "stop");
        let stopped = false;
        for (let attempt = 0; attempt < 20; attempt++) {
          await new Promise(resolve => setTimeout(resolve, 500));
          const current = await api.chatGptState();
          if (current.profiles.find(p => p.id === target)?.status === "stopped") { stopped = true; break; }
        }
        if (!stopped) throw new Error("目标实例尚未完全退出。请在 ChatGPT 中退出该实例后重试。");
      } else if (status !== "stopped") throw new Error("目标实例当前不能安全写入，请刷新状态后重试。");
      let openId: string | null = null;
      let copied = 0;
      for (let offset = 0; offset < plans.length; offset += 100) {
        const job = await api.chatGptBatchCreate(plans.slice(offset, offset + 100));
        const completed = await drive(job);
        if (completed.lastError || stop.current || completed.outcomes.length < completed.requests.length) {
          throw new Error("同步已暂停。复制队列已保存，可从下方继续；目标实例尚未打开。");
        }
        const failed = completed.outcomes.filter(o => o.error);
        if (failed.length) throw new Error(`${failed.length} 条会话同步失败。请查看复制队列；目标实例尚未打开。`);
        copied += completed.outcomes.length;
        openId ??= completed.outcomes.find(o => o.result)?.result?.targetThreadId ?? null;
      }
      if (!openId) throw new Error("未能找到可打开的目标会话。");
      setBusy("正在打开目标会话");
      await api.chatGptOpenThread(target, openId);
      setMessage(`已同步 ${copied} 条本地会话，并在目标账号中打开最近的一条。源账号和原记录保持完整。`);
    });
  }
  const unfinished = jobs.filter(j => !j.cancelled && j.outcomes.length < j.requests.length);
  return <section className="chatgpt-history" aria-label="跨账号会话同步"><Stack gap="md">
    <div className="chatgpt-history-heading"><Title order={4}>跨账号会话同步</Title>
      <Text size="sm" c="dimmed" mt={3}>选择来源与目标，复制本地工作记录。云端聊天与实时同步暂不支持。</Text>
    </div>
    {error && <Alert role="alert" color="red">{error}</Alert>}
    {message && <Alert role="status" color="teal">{message}</Alert>}
    <div className="chatgpt-history-grid"><div className="chatgpt-sync-options">
    <Stack gap="sm"><Group grow align="flex-start" className="chatgpt-sync-pair">
      <Select label="来源实例" clearable disabled={locked} value={source} data={[{ value: "default", label: "默认实例的本地 Codex 记录" }, ...state.profiles.map(p => ({ value: p.id, label: p.name }))]}
        onChange={value => { if (value === target) void loadTarget(null); void load(value); }} />
      <Select label="目标实例" clearable disabled={locked} value={target} data={state.profiles.filter(p => p.id !== source).map(p => ({ value: p.id, label: p.name }))} onChange={value => { setPreviews(null); void loadTarget(value); }} />
    </Group>
    <Group align="end" className="chatgpt-sync-directory"><TextInput label="接续项目目录（可选）" description="留空使用原目录；项目文件不会复制。" style={{ flex: 1 }} value={workspace} disabled={locked || scope === "all"} onChange={e => { setWorkspace(e.currentTarget.value); setPreviews(null); }} />
      <Button variant="default" disabled={scope === "all"} leftSection={actionPending("选择项目目录") ? <IconLoader2 size={15} className="chatgpt-button-spinner" /> : <IconHistory size={15} />} aria-busy={actionPending("选择项目目录")} onClick={() => void run("选择项目目录", async () => { const path = await open({ directory: true, multiple: false }); if (typeof path === "string" && alive.current) { setWorkspace(path); setPreviews(null); } })}>选择目录</Button></Group>
    <Group grow align="flex-start" className="chatgpt-sync-scope"><Select label="同步范围" disabled={locked} value={scope} onChange={value => { setScope((value ?? "selected") as typeof scope); if (value === "all") setWorkspace(""); setPreviews(null); }} data={[{ value: "selected", label: "勾选的会话" }, { value: "project", label: "整个项目" }, { value: "all", label: "全部本地会话" }]} />
      {scope === "project" && <Select label="选择项目" searchable disabled={locked} value={project} onChange={setProject} data={projects.map(p => ({ value: p, label: p }))} />}</Group>
    <Button className="chatgpt-sync-primary" leftSection={actionPending("准备同步") ? <IconLoader2 size={16} className="chatgpt-button-spinner" /> : <IconHistory size={16} />} aria-busy={actionPending("准备同步")} disabled={!source || !target || !history || scopeItems.length === 0 || !state.installation?.compatible} onClick={() => void syncAndSwitch()}>同步并切换账号（{scopeItems.length} 条）</Button>
    </Stack></div>
    <div className="chatgpt-records"><Title order={5}>工作记录</Title>
    {source && <Group gap="xs" className="chatgpt-record-actions"><Button size="xs" variant="subtle" onClick={() => void load(source)}>重新读取记录</Button>
      <Button variant="subtle" disabled={!history} onClick={() => setSelected(history!.items.filter(i => i.transferable).slice(0,100).map(i => i.key))}>选择前 100 条可复制记录</Button>
      <Button variant="subtle" onClick={() => setSelected([])}>清空选择</Button></Group>}
    {history?.warnings.map(w => <Text key={w} size="xs" c="dimmed">{w}</Text>)}
    {!source && <div className="chatgpt-records-placeholder"><span className="chatgpt-records-placeholder-icon"><IconHistory size={24} /></span><Text size="sm" c="dimmed">选择来源实例后，在这里查看会话。</Text></div>}
    {source && !history && !error && <Text role="status">正在读取工作记录…</Text>}
    {history?.items.length === 0 && <Text c="dimmed">没有可复制的本地 Codex 工作记录。普通 ChatGPT 聊天不会出现在这里。</Text>}
    {history && <TextInput label="查找会话" placeholder="标题或项目路径" value={query} disabled={locked} onChange={e => setQuery(e.currentTarget.value)} />}
    {source && <div className="chatgpt-record-list">{visibleItems.slice(0,200).map(item => <Card key={item.key} className="chatgpt-record-row" p="sm"><Group justify="space-between" wrap="nowrap">
      <Checkbox label={item.title} description={`${new Date(item.modifiedAt * 1000).toLocaleString()} · ${item.workspace} · ${item.detail}`} checked={selected.includes(item.key)} disabled={locked || !item.transferable || (!selected.includes(item.key) && selected.length >= 100)}
        onChange={e => { const checked = e.currentTarget.checked; setSelected(old => checked ? [...old,item.key] : old.filter(k => k !== item.key)); setPreviews(null); }} />
      <Button size="xs" variant="light" aria-busy={actionPending(`检查记录与附件:${item.key}`)} disabled={!ready || !item.transferable} onClick={() => void inspect([item.key])}>复制到目标</Button>
    </Group></Card>)}</div>}
    {visibleItems.length > 200 && <Text size="sm" c="dimmed">列表只渲染前 200 条。可用项目或全部范围同步其余记录，也可搜索具体会话。</Text>}
    {(selected.length > 0 || busy) && <Group className="chatgpt-record-footer">{selected.length > 0 && <Button aria-busy={actionPending(`检查记录与附件:${selected.join(",")}`)} disabled={!ready} onClick={() => void inspect(selected)}>预览选中的 {selected.length} 条记录</Button>}{busy && <Text role="status" size="sm">{busy}</Text>}</Group>}
    {source && target && !ready && <Text size="sm" c="orange">单独复制需要先关闭目标实例；“同步并切换账号”会在关闭运行中的目标前请你确认。</Text>}
    </div></div>
    {target && (pending.length > 0 || unfinished.length > 0 || jobs.some(job => job.outcomes.length > 0) || busy.startsWith("正在复制") || busy.startsWith("正在检查")) && <Stack className="chatgpt-queue" gap="xs"><Group justify="space-between"><Title order={5}>复制队列与中断恢复</Title><Button size="xs" variant="subtle" onClick={() => void loadTarget(target)}>刷新队列</Button></Group>
      {pending.map(p => <Card key={p.key} className="chatgpt-queue-row" p="sm"><Text>{p.title || "未完成的副本"}</Text><Group mt="xs">
        <Button size="xs" aria-busy={actionPending(`恢复副本:${p.key}`)} disabled={targetProfile?.status !== "stopped"} onClick={() => void run(`恢复副本:${p.key}`, async () => { const value = await api.chatGptRecover(target,p.key,false); if (alive.current) setMessage(value.detail); })}>继续恢复</Button>
        <Button size="xs" color="red" variant="light" disabled={targetProfile?.status !== "stopped"} onClick={() => setDiscarding(p)}>撤回未完成副本</Button></Group></Card>)}
      {unfinished.map(job => <Card key={job.id} className="chatgpt-queue-row" p="sm"><Text size="sm">已处理 {job.outcomes.length}/{job.requests.length} 条</Text>{job.lastError && <Text size="sm" c="red">上次中断：{job.lastError}</Text>}<Group mt="xs">
        <Button size="xs" aria-busy={actionPending(`恢复批量复制:${job.id}`)} disabled={targetProfile?.status !== "stopped"} onClick={() => void run(`恢复批量复制:${job.id}`, () => drive(job))}>继续队列</Button>
        <Button size="xs" variant="light" aria-busy={actionPending(`取消剩余复制:${job.id}`)} onClick={() => void run(`取消剩余复制:${job.id}`, async () => { await api.chatGptBatchStep(target,job.id,true); })}>取消剩余复制</Button></Group></Card>)}
      {(busy.startsWith("正在复制") || busy.startsWith("正在检查")) && <Button variant="light" onClick={() => { stop.current = true; setMessage("当前记录处理完后暂停。"); }}>暂停后续复制</Button>}
      {jobs.filter(j => j.outcomes.length > 0).slice(0,10).map(job => <details key={job.id}><summary>复制结果：{job.outcomes.filter(o => o.result).length} 成功，{job.outcomes.filter(o => o.error).length} 失败{job.cancelled ? "（已取消剩余）" : ""}</summary>
        {job.outcomes.map(o => <Text key={o.key} size="sm" c={o.error ? "red" : "dimmed"}>{history?.items.find(i => i.key === o.key)?.title ?? o.key.slice(0,12)}：{o.error ?? (o.result?.duplicate ? "已有独立副本，未覆盖" : "复制成功")}</Text>)}
      </details>)}
      {pending.length === 0 && unfinished.length === 0 && <Text size="sm" c="dimmed">没有未完成的复制任务。</Text>}
    </Stack>}
    <Modal opened={active && previews !== null} onClose={() => { if (!locked) setPreviews(null); }} title="确认复制范围" centered>
      <Stack><Text>{source === "default" ? "默认实例的本地 Codex 记录" : sourceProfile?.name} → {targetProfile?.name}</Text>
        {previews?.map(p => <div key={p.key}><Text fw={600}>{p.title}</Text><Text size="sm">{Math.ceil(p.bytes/1024)} KB · {p.images} 张本地图片 · 项目：{p.workspace}</Text></div>)}
        <Text size="sm">历史与列出的图片将复制到目标实例，续聊时可能由目标账号发送处理。账号凭证及工具授权保持独立；项目文件不会随记录复制。</Text>
        <Button aria-busy={actionPending("创建复制队列")} disabled={!ready} onClick={() => void run("创建复制队列", async () => { if (!previews) return; const job = await api.chatGptBatchCreate(previews.map(p => request(p.key,p.fingerprint))); setPreviews(null); await drive(job); })}>创建独立副本</Button>
      </Stack>
    </Modal>
    <Modal opened={active && discarding !== null} onClose={() => { if (!locked) setDiscarding(null); }} title="撤回未完成副本" centered><Stack>
      <Text>仅删除这次尚未完成的目标副本与临时快照。来源及其他已完成的记录保留。</Text>
      <Alert color="orange">如果你曾打开并修改过这份未完成副本，撤回也会删除其中的修改。</Alert>
      <Button color="red" aria-busy={actionPending("撤回复制")} onClick={() => void run("撤回复制", async () => { if (!target || !discarding) return; const value = await api.chatGptRecover(target,discarding.key,true); setDiscarding(null); setMessage(value.detail); })}>确认撤回</Button>
    </Stack></Modal>
    <Modal opened={active && confirmClosingTarget !== null} onClose={() => setConfirmClosingTarget(null)} title="关闭目标窗口后同步？" centered><Stack>
      <Text>目标实例正在运行。继续会关闭它；复制成功后才会重新打开。正在进行的问答或任务可能中断，请先在目标窗口完成当前工作。</Text>
      <Group justify="flex-end"><Button variant="default" onClick={() => setConfirmClosingTarget(null)}>暂不关闭</Button>
        <Button color="orange" onClick={() => { const confirmed = confirmClosingTarget; setConfirmClosingTarget(null); if (confirmed) void syncAndSwitch(confirmed); }}>确认关闭并同步</Button></Group>
    </Stack></Modal>
  </Stack></section>;
}
