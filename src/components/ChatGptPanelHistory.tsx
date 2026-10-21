import { useEffect, useRef, useState } from "react";
import { Alert, Button, Card, Checkbox, Collapse, Group, Modal, Select, Stack, Text, TextInput, Title } from "@mantine/core";
import { IconArrowRight, IconChevronDown, IconCopy, IconFolder, IconHistory, IconLoader2, IconRefresh, IconSearch, IconSettings, IconUser, IconRobot } from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { relativeTime } from "./EnvironmentProof";
import { api, type ChatGptConversation, type ChatGptState, type ChatGptHistory, type ChatGptHistoryItem, type ChatGptPreview, type ChatGptTransferRequest, type ChatGptPending, type ChatGptJob } from "../api";

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
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [conversation, setConversation] = useState<ChatGptConversation | null>(null);
  const [conversationError, setConversationError] = useState("");
  const [conversationLoading, setConversationLoading] = useState(false);
  const [conversationRetry, setConversationRetry] = useState(0);
  const [openAfterCopy, setOpenAfterCopy] = useState(false);
  const [settingsOpened, setSettingsOpened] = useState(false);
  const [visibleLimit, setVisibleLimit] = useState(50);
  const [collapsedGroups, setCollapsedGroups] = useState<string[]>([]);
  const [expandedGroups, setExpandedGroups] = useState<string[]>([]);
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
    setSource(sourceId); if (sourceId !== source) { setSelected([]); setProject(null); } setPreviews(null); setHistory(null); setFocusedKey(null); setConversation(null); setQuery(""); setVisibleLimit(50); setCollapsedGroups([]); setExpandedGroups([]); setError("");
    if (!sourceId) return;
    try {
      const value = await api.chatGptHistory(sourceId);
      if (alive.current && generation === historyGen.current) setHistory(value);
    } catch (e) { if (alive.current && generation === historyGen.current) setError(String(e)); }
  }
  async function loadTarget(targetId: string | null) {
    const generation = ++targetGen.current;
    setTarget(targetId); setPending([]); setJobs([]); if (targetId !== target) setOpenAfterCopy(false);
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
      setSettingsOpened(saved?.scope === "project" || saved?.scope === "all");
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
  const locked = !!busy || previews !== null || confirmClosingTarget !== null;
  const sourceProfile = state.profiles.find(p => p.id === source);
  const targetProfile = state.profiles.find(p => p.id === target);
  const request = (key: string, fingerprint?: string): ChatGptTransferRequest => ({ sourceId: source!, targetId: target!, key,
    revision: history!.items.find(i => i.key === key)!.revision, ...(workspace.trim() ? { workspace: workspace.trim() } : {}), ...(fingerprint ? { fingerprint } : {}) });

  async function inspect(keys: string[]) {
    await run("检查复制范围", async () => {
      if (!source || !target || source === target || keys.length === 0) return;
      const results: ChatGptPreview[] = [];
      for (let index = 0; index < keys.length; index += 100) {
        if (stop.current || !alive.current) return;
        setBusy(`正在检查 ${index + 1}/${keys.length}`);
        results.push(...await api.chatGptPreviewMany(keys.slice(index, index + 100).map((key) => request(key))));
      }
      if (alive.current && !stop.current) { setSelected(keys); setPreviews(results); }
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
  const visibleItems = (history?.items ?? []).filter(i => !query || `${i.title} ${i.summary ?? ""} ${i.workspace}`.toLowerCase().includes(query.toLowerCase()));
  const focusedItem = visibleItems.find(item => item.key === focusedKey);
  const previewItem = focusedItem ?? visibleItems[0];
  useEffect(() => {
    let cancelled = false;
    setConversation(null); setConversationError("");
    if (!active || !source || !previewItem) { setConversationLoading(false); return; }
    setConversationLoading(true);
    api.chatGptConversation(source, previewItem.key, previewItem.revision).then(value => {
      if (!cancelled && alive.current) setConversation(value);
    }).catch(error => { if (!cancelled && alive.current) setConversationError(String(error)); })
      .finally(() => { if (!cancelled && alive.current) setConversationLoading(false); });
    return () => { cancelled = true; };
  }, [active, source, previewItem?.key, previewItem?.revision, conversationRetry]);
  const profileLabel = (id: string) => {
    const profile = state.profiles.find(p => p.id === id);
    return profile ? `${profile.name}${profile.api?.active ? " · API 模式" : profile.account?.status === "chatgpt" ? ` · ${profile.account.email}` : ""}` : "";
  };
  const projectName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "未关联项目";
  const displayTitle = (item: NonNullable<typeof previewItem>) => item.title === "未命名会话" ? `未命名会话 · ${new Date(item.modifiedAt * 1000).toLocaleString()}` : item.title;
  const timeLabel = (seconds: number) => {
    const age = Date.now() / 1000 - seconds;
    return age > 30 * 86400
      ? new Date(seconds * 1000).toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" })
      : relativeTime(seconds);
  };
  // ChatGPT-style grouping: one collapsible section per project folder,
  // newest project first, sessions inside ordered by recency.
  const groupedItems = (() => {
    const groups = new Map<string, ChatGptHistoryItem[]>();
    for (const item of visibleItems.slice(0, visibleLimit)) {
      const list = groups.get(item.workspace);
      if (list) list.push(item); else groups.set(item.workspace, [item]);
    }
    return [...groups.entries()]
      .map(([key, items]) => ({
        key,
        label: key ? projectName(key) : "未关联项目",
        items,
        latest: Math.max(...items.map(item => item.modifiedAt)),
        collapsed: collapsedGroups.includes(key),
        expanded: query !== "" || expandedGroups.includes(key),
      }))
      .sort((a, b) => a.key === "" ? 1 : b.key === "" ? -1 : b.latest - a.latest);
  })();
  const toggleGroup = (key: string) => setCollapsedGroups(old => old.includes(key) ? old.filter(value => value !== key) : [...old, key]);
  const showAllInGroup = (key: string) => setExpandedGroups(old => old.includes(key) ? old : [...old, key]);
  const summaryOf = (item: ChatGptHistoryItem) => {
    const summary = item.summary?.trim() ?? "";
    const title = displayTitle(item);
    return summary && summary !== title && !summary.startsWith(title) ? summary : "";
  };
  const copyBlockedReason = () => {
    if (!source) return "先选择来源实例。";
    if (!target) return "再选择目标实例。";
    if (source === target) return "来源和目标不能是同一个实例。";
    if (!history) return "正在读取会话列表…";
    if (scopeItems.length === 0) return scope === "selected" ? "先在左侧勾选要复制的会话。" : "当前范围没有会话。";
    if (!state.installation?.compatible || !state.installation.cli) return "当前客户端版本不支持本地会话复制。";
    if (scope !== "selected" && !history.complete) return "会话列表读取不完整，暂不能按项目或全部复制。";
    if (scope === "all" && workspace.trim()) return "复制全部会话时不能指定接续项目目录。";
    const blocked = scopeItems.find(item => !item.transferable);
    return blocked ? `「${displayTitle(blocked)}」当前不能复制：${blocked.detail}` : "";
  };
  async function copySelected(confirmedTarget: string | null = null) {
    await run("准备复制", async () => {
      if (!source || !target || !history || !previews || source === target || !state.installation?.compatible || !state.installation.cli) return;
      const plans = previews.map(preview => request(preview.key, preview.fingerprint));
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
          throw new Error("复制已暂停。复制队列已保存，可从下方继续；目标实例尚未打开。");
        }
        const failed = completed.outcomes.filter(o => o.error);
        if (failed.length) throw new Error(`${failed.length} 条会话复制失败。请查看复制队列；目标实例尚未打开。`);
        copied += completed.outcomes.length;
        openId ??= completed.outcomes.find(o => o.result)?.result?.targetThreadId ?? null;
      }
      setPreviews(null);
      if (openAfterCopy && openId) {
        setBusy("正在打开目标会话");
        try { await api.chatGptOpenThread(target, openId); }
        catch (error) { throw new Error(`已复制 ${copied} 条会话，但打开目标会话失败：${String(error)}。副本已保留，可手动打开目标实例。`); }
      }
      setMessage(`已复制 ${copied} 条会话到${targetProfile?.name ?? "目标实例"}${openAfterCopy && openId ? "，并打开一条目标会话" : ""}。来源记录保留。`);
    });
  }
  const unfinished = jobs.filter(j => !j.cancelled && j.outcomes.length < j.requests.length);
  const canInspect = !!source && !!target && source !== target && !!history && scopeItems.length > 0
    && scopeItems.every(item => item.transferable) && !!state.installation?.compatible && !!state.installation.cli
    && (scope === "selected" || !!history.complete) && !(scope === "all" && !!workspace.trim());
  const blockedReason = copyBlockedReason();
  return <section className="chatgpt-history" aria-label="复制会话"><Stack gap="md">
    <div className="chatgpt-history-heading"><Title order={4}>复制会话</Title>
      <Text size="sm" c="dimmed" mt={3}>把本地对话复制到另一个实例，继续处理工作。普通云端聊天不包含在此列表中。</Text>
      <Text size="xs" c="dimmed" mt={2}>用法：选择来源 → 勾选会话（或在“更多设置”里按项目/全部）→ 选择目标 → 点右下角复制。</Text>
    </div>
    {error && <Alert role="alert" color="red">{error}</Alert>}
    {message && <Alert role="status" color="teal">{message}</Alert>}
    <div className="chatgpt-copy-workspace">
      <div className="chatgpt-copy-toolbar">
        <Select label="来源实例" placeholder="选择来源" clearable disabled={locked} value={source}
          data={[{ value: "default", label: "默认实例的本地 Codex 记录" }, ...state.profiles.map(p => ({ value: p.id, label: profileLabel(p.id) }))]}
          onChange={value => { if (value === target) void loadTarget(null); void load(value); }} />
        <IconArrowRight className="chatgpt-copy-direction" size={23} aria-hidden="true" />
        <Select label="目标实例" placeholder="选择复制到哪个实例" clearable disabled={locked} value={target}
          data={state.profiles.filter(p => p.id !== source).map(p => ({ value: p.id, label: profileLabel(p.id) }))}
          onChange={value => { setPreviews(null); void loadTarget(value); }} />
        <Button className="chatgpt-copy-settings-button" variant="subtle" leftSection={<IconSettings size={17} />} rightSection={<IconChevronDown size={14} />} disabled={locked} onClick={() => setSettingsOpened(value => !value)} aria-expanded={settingsOpened}>更多设置</Button>
      </div>
      {settingsOpened && <div className="chatgpt-copy-settings">
        <Select label="复制范围" description={scope !== "selected" ? "按范围复制，无需逐条勾选。" : undefined} disabled={locked} value={scope} onChange={value => { setScope((value ?? "selected") as typeof scope); if (value === "all") setWorkspace(""); setPreviews(null); }} data={[{ value: "selected", label: "勾选的会话" }, { value: "project", label: "整个项目" }, { value: "all", label: "全部本地会话" }]} />
        {scope === "project" && <Select label="选择项目" searchable disabled={locked} value={project} onChange={setProject} data={projects.map(p => ({ value: p, label: p }))} />}
        <Group align="end" className="chatgpt-sync-directory"><TextInput label="接续项目目录（可选）" description="留空使用原目录；项目文件不会复制。" style={{ flex: 1 }} value={workspace} disabled={locked || scope === "all"} onChange={e => { setWorkspace(e.currentTarget.value); setPreviews(null); }} />
          <Button variant="default" disabled={locked || scope === "all"} leftSection={<IconHistory size={15} />} onClick={() => void run("选择项目目录", async () => { const path = await open({ directory: true, multiple: false }); if (typeof path === "string" && alive.current) { setWorkspace(path); setPreviews(null); } })}>选择目录</Button></Group>
          {history?.warnings.length ? <details className="chatgpt-history-notes"><summary>记录读取说明{!history.complete ? " · 列表不完整" : ""}</summary>{history.warnings.map(w => <Text key={w} size="xs" c="dimmed">{w}</Text>)}</details> : null}
          {history && visibleItems.some(item => item.transferable) && <Button variant="subtle" size="xs" disabled={locked} onClick={() => setSelected(visibleItems.filter(item => item.transferable).slice(0, 100).map(item => item.key))}>选择当前结果的前 100 条可复制会话</Button>}
      </div>}
      <div className="chatgpt-conversation-grid">
        <div className="chatgpt-records">
          <TextInput label="查找会话" placeholder="搜索标题、提问内容或项目" leftSection={<IconSearch size={17} />} value={query} onChange={e => { setQuery(e.currentTarget.value); setVisibleLimit(50); }} />
          <Group justify="space-between" className="chatgpt-record-actions"><Text fw={600}>会话 · {visibleItems.length} 条</Text>
            <Button size="xs" variant="subtle" leftSection={<IconRefresh size={15} />} disabled={!source || locked} onClick={() => void load(source)}>刷新记录</Button></Group>
          {history && !history.complete && <Text size="xs" c="orange">列表读取不完整，请在更多设置中查看原因；暂不能复制整个项目或全部会话。</Text>}
          {!source && <div className="chatgpt-records-placeholder"><IconHistory size={26} /><Text c="dimmed" size="sm">选择来源实例后，在这里查看会话。</Text></div>}
          {source && !history && !error && <Text role="status">正在读取会话…</Text>}
          {history && visibleItems.length === 0 && <Text c="dimmed" size="sm">{query ? "没有匹配的会话，请换个关键词。" : "没有可读取的本地会话。普通云端聊天不会出现在这里。"}</Text>}
          <div className="chatgpt-record-list">{groupedItems.map(group => <div className="chatgpt-record-group" key={group.key}>
            <button type="button" className="chatgpt-record-group-toggle" aria-expanded={!group.collapsed} title={group.key || "未关联项目"} onClick={() => toggleGroup(group.key)}>
              <IconFolder size={15} aria-hidden="true" />
              <span className="chatgpt-record-group-name">{group.label}</span>
              <span className="chatgpt-record-group-count">{group.items.length}</span>
              <IconChevronDown size={14} className="chatgpt-record-group-chevron" data-collapsed={group.collapsed || undefined} aria-hidden="true" />
            </button>
            <Collapse in={!group.collapsed}>
              {(group.expanded ? group.items : group.items.slice(0, 5)).map(item => <div className="chatgpt-conversation-row" data-focused={previewItem?.key === item.key} key={item.key}>
                <Checkbox aria-label={`选择会话：${displayTitle(item)}`} checked={scope === "selected" ? selected.includes(item.key) : scopeItems.includes(item)} disabled={locked || scope !== "selected" || !item.transferable || (!selected.includes(item.key) && selected.length >= 100)}
                  onChange={e => { const checked = e.currentTarget.checked; setSelected(old => checked ? [...new Set([...old, item.key])] : old.filter(k => k !== item.key)); }} />
                <button type="button" className="chatgpt-conversation-select" aria-label={`预览会话：${displayTitle(item)}`} aria-pressed={previewItem?.key === item.key} onClick={() => setFocusedKey(item.key)}>
                  <span className="chatgpt-conversation-title">{displayTitle(item)}</span>
                  {summaryOf(item) && <span className="chatgpt-conversation-summary">{summaryOf(item)}</span>}
                  {!item.transferable && <span className="chatgpt-conversation-unavailable">暂不能复制：{item.detail}</span>}
                </button>
                <span className="chatgpt-conversation-time" title={new Date(item.modifiedAt * 1000).toLocaleString()}>{timeLabel(item.modifiedAt)}</span>
              </div>)}
              {!group.expanded && group.items.length > 5 && <Button className="chatgpt-record-more" variant="subtle" size="xs" onClick={() => showAllInGroup(group.key)}>展开显示（还有 {group.items.length - 5} 条）</Button>}
            </Collapse>
          </div>)}</div>
          {visibleItems.length > visibleLimit && <Button variant="subtle" size="xs" onClick={() => setVisibleLimit(value => value + 50)}>加载更多会话（剩余 {visibleItems.length - visibleLimit} 条）</Button>}
        </div>
        <div className="chatgpt-conversation-preview" aria-label="对话预览" aria-busy={conversationLoading}>
          <Title order={5}>对话预览</Title>
          <Text size="xs" c="dimmed" className="chatgpt-preview-hint">只读查看这条会话里真实的提问与回答；复制前用来核对内容，不会修改来源记录。</Text>
          {previewItem ? <><Text className="chatgpt-preview-title" fw={700}>{displayTitle(previewItem)}</Text>
            <Text size="xs" c="dimmed">项目：{projectName(previewItem.workspace)} · {new Date(previewItem.modifiedAt * 1000).toLocaleString()}</Text>
            {conversationLoading && <Text role="status" size="sm">正在读取对话正文…</Text>}
            {conversationError && <Alert color="orange"><Text size="sm">{conversationError}</Text><Button variant="subtle" size="xs" onClick={() => setConversationRetry(value => value + 1)}>重试正文预览</Button></Alert>}
            {conversation && <div className="chatgpt-conversation-messages">{conversation.messages.length === 0 && <Text c="dimmed" size="sm">没有提取到可显示的用户提问或回答。这条记录可能只包含工具调用，仍可复制；记录详情仍可查看。</Text>}{conversation.messages.map((item, index) => <div className="chatgpt-conversation-message" data-role={item.role} key={index}>
              <span className="chatgpt-message-avatar" aria-hidden="true">{item.role === "user" ? <IconUser size={19} /> : <IconRobot size={19} />}</span>
              <div className="chatgpt-message-copy"><Group gap="sm"><Text fw={600} size="sm">{item.role === "user" ? "你" : "助手"}</Text>{item.timestamp && <Text size="xs" c="dimmed">{new Date(item.timestamp).toLocaleString()}</Text>}</Group><Text size="sm" className="chatgpt-message-text">{item.text}</Text></div>
            </div>)}{conversation.truncated && <Text size="xs" c="dimmed">正文预览仅展示前 100 条消息，每条最多 6000 字；复制时仍检查完整记录。</Text>}</div>}
            <details className="chatgpt-record-details"><summary>记录详情</summary><Text size="xs">会话编号：{previewItem.threadId}</Text><Text size="xs">项目目录：{previewItem.workspace || "未关联项目"}</Text><Text size="xs">记录大小：{Math.ceil(previewItem.bytes / 1024)} KB</Text><Text size="xs">{previewItem.detail}</Text></details>
          </> : <div className="chatgpt-records-placeholder"><IconHistory size={26} /><Text c="dimmed" size="sm">点击左侧会话，查看真实提问和回答。</Text></div>}
        </div>
      </div>
      <div className="chatgpt-copy-footer">
        <div className="chatgpt-copy-summary"><Group gap="sm"><Text fw={600}>已选择 {scopeItems.length} 条会话</Text>{scope !== "selected" && <Text size="xs" c="dimmed">范围：{scope === "project" ? "整个项目" : "全部本地会话"}</Text>}<Button size="compact-xs" variant="subtle" disabled={locked || selected.length === 0} onClick={() => setSelected([])}>清空选择</Button><Text size="sm" c="dimmed">目标：{targetProfile?.name ?? "未选择"}</Text></Group>
          {query && scopeItems.some(item => !visibleItems.includes(item)) && <Text size="xs" c="orange">其中 {scopeItems.filter(item => !visibleItems.includes(item)).length} 条已选会话不在当前搜索结果中。</Text>}
          <Text size="xs" c="dimmed">创建独立副本，来源会话保留；项目文件不会随会话复制。</Text></div>
        <Group className="chatgpt-copy-footer-actions"><Checkbox label="复制完成后打开目标会话" checked={openAfterCopy} disabled={locked} onChange={e => setOpenAfterCopy(e.currentTarget.checked)} />
          <Button leftSection={actionPending("检查复制范围") ? <IconLoader2 size={17} className="chatgpt-button-spinner" /> : <IconCopy size={17} />} aria-busy={actionPending("检查复制范围")} disabled={locked || !canInspect} onClick={() => void inspect(scopeItems.map(item => item.key))}>{`${scope === "selected" ? "复制所选会话" : scope === "project" ? "复制项目会话" : "复制全部会话"}（${scopeItems.length} 条）`}</Button></Group>
        {!canInspect && blockedReason && <Text size="xs" c="dimmed" className="chatgpt-copy-reason">{blockedReason}</Text>}
        {busy && <Text role="status" size="sm" className="chatgpt-copy-progress">{busy}</Text>}
      </div>
    </div>
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
    <Modal opened={active && previews !== null && confirmClosingTarget === null} onClose={() => { if (!busy) setPreviews(null); }} title="确认复制范围" centered>
      <Stack><Text>{source === "default" ? "默认实例的本地 Codex 记录" : sourceProfile?.name} → {targetProfile?.name}</Text>
        {previews?.slice(0, 10).map(p => <div key={p.key}><Text fw={600}>{p.title}</Text><Text size="sm">{Math.ceil(p.bytes/1024)} KB · {p.images} 张本地图片 · 项目：{p.workspace}</Text></div>)}
        <Text size="sm">历史与列出的图片将复制到目标实例，续聊时可能由目标账号发送处理。账号凭证及工具授权保持独立；项目文件不会随记录复制。</Text>
        {previews && previews.length > 10 && <Text size="sm">另有 {previews.length - 10} 条，共 {previews.length} 条会话。</Text>}
        <Text size="sm">复制完成后{openAfterCopy ? "会打开目标实例中的一条会话。" : "保持目标实例关闭，可稍后手动打开。"}</Text>
        <Button aria-busy={actionPending("准备复制")} disabled={!!busy} onClick={() => void copySelected()}>确认复制</Button>
      </Stack>
    </Modal>
    <Modal opened={active && discarding !== null} onClose={() => { if (!locked) setDiscarding(null); }} title="撤回未完成副本" centered><Stack>
      <Text>仅删除这次尚未完成的目标副本与临时快照。来源及其他已完成的记录保留。</Text>
      <Alert color="orange">如果你曾打开并修改过这份未完成副本，撤回也会删除其中的修改。</Alert>
      <Button color="red" aria-busy={actionPending("撤回复制")} onClick={() => void run("撤回复制", async () => { if (!target || !discarding) return; const value = await api.chatGptRecover(target,discarding.key,true); setDiscarding(null); setMessage(value.detail); })}>确认撤回</Button>
    </Stack></Modal>
    <Modal opened={active && confirmClosingTarget !== null} onClose={() => setConfirmClosingTarget(null)} title="关闭目标窗口后复制？" centered><Stack>
      <Text>目标实例正在运行。复制前需要关闭它；是否在完成后打开由你勾选的选项决定。正在进行的问答或任务可能中断，请先在目标窗口完成当前工作。</Text>
      <Group justify="flex-end"><Button variant="default" onClick={() => setConfirmClosingTarget(null)}>暂不关闭</Button>
        <Button color="orange" onClick={() => { const confirmed = confirmClosingTarget; setConfirmClosingTarget(null); if (confirmed) void copySelected(confirmed); }}>确认关闭并复制</Button></Group>
    </Stack></Modal>
  </Stack></section>;
}
