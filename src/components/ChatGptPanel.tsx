import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Alert, Badge, Button, Group, Modal, PasswordInput, Select, Stack, Switch, Text, TextInput, Title } from "@mantine/core";
import { IconBrandOpenai, IconCircleCheck, IconDeviceFloppy, IconFolderOpen, IconInfoCircle, IconKey, IconLoader2, IconPlayerPlay, IconPlayerStop, IconPlus, IconRefresh, IconX } from "@tabler/icons-react";
import { api, type ChatGptAction, type ChatGptProfile, type ChatGptState } from "../api";
import RiskConfirm from "./RiskConfirm";
import ChatGptPanelHistory from "./ChatGptPanelHistory";

const STATUS = {
  running: "运行中",
  stopped: "已关闭",
  closing: "后台进程仍在运行",
  error: "需要处理",
};

export default function ChatGptPanel({ active = true }: { active?: boolean }) {
  const [state, setState] = useState<ChatGptState | null>(null);
  const [pendingActions, setPendingActions] = useState<string[]>([]);
  const pendingActionsRef = useRef(new Set<string>());
  const operationTail = useRef<Promise<void>>(Promise.resolve());
  const operationCount = useRef(0);
  const stateGeneration = useRef(0);
  const diagnosticGeneration = useRef(new Map<string, number>());
  const diagnosticTasks = useRef(new Map<string, Promise<{ healthy: boolean; details: string[] }>>());
  const alive = useRef(true);
  const [message, setMessage] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<Record<string, { pending: boolean; healthy: boolean; details: string[]; error: string | null }>>({});
  const [diagnosticDetailsId, setDiagnosticDetailsId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [deleting, setDeleting] = useState<ChatGptProfile | null>(null);
  const [apiConfigId, setApiConfigId] = useState<string | null>(null);
  const [apiBaseUrl, setApiBaseUrl] = useState("");
  const [apiModel, setApiModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [apiModels, setApiModels] = useState<{ id: string; name: string }[]>([]);
  const [apiModelsLoading, setApiModelsLoading] = useState(false);
  const [apiModelsError, setApiModelsError] = useState<string | null>(null);
  const [apiManualModel, setApiManualModel] = useState(false);
  const [apiRefreshSequence, setApiRefreshSequence] = useState(0);
  const apiDiscoveryGeneration = useRef(0);
  const apiSavedProfile = state?.profiles.find(profile => profile.id === apiConfigId);
  const apiSavedBaseUrl = apiSavedProfile?.api?.baseUrl;
  const apiHasSavedKey = apiSavedProfile?.api?.hasKey;
  const [copySource, setCopySource] = useState<string | null>(null);
  const [initialCopy, setInitialCopy] = useState<{ source: string | null; target: string } | null>(null);

  useEffect(() => { alive.current = true; return () => { alive.current = false; stateGeneration.current++; diagnosticGeneration.current.clear(); }; }, []);
  const refresh = useCallback(async () => {
    const generation = ++stateGeneration.current;
    try {
      const next = await api.chatGptState();
      if (alive.current && generation === stateGeneration.current) setState(next);
    } catch (error) {
      if (alive.current && generation === stateGeneration.current) setMessage(String(error));
    }
  }, []);
  useEffect(() => {
    if (!active) return;
    void refresh();
    const timer = setInterval(() => { if (operationCount.current === 0) void refresh(); }, 15000);
    return () => clearInterval(timer);
  }, [active, refresh]);

  const coordinate = useCallback(async <T,>(operation: () => Promise<T>): Promise<T> => {
    const previous = operationTail.current;
    let release!: () => void;
    operationTail.current = new Promise<void>(resolve => { release = resolve; });
    operationCount.current++;
    await previous;
    try { return await operation(); }
    finally { operationCount.current--; release(); }
  }, []);
  const run = async (label: string, operation: () => Promise<void>) => {
    if (pendingActionsRef.current.has(label)) return;
    pendingActionsRef.current.add(label);
    setPendingActions([...pendingActionsRef.current]);
    await coordinate(async () => {
      stateGeneration.current++;
      setMessage(null);
      try { await operation(); }
      catch (error) { if (alive.current) setMessage(String(error)); }
      finally {
        pendingActionsRef.current.delete(label);
        if (alive.current) { setPendingActions([...pendingActionsRef.current]); void refresh(); }
      }
    });
  };
  const pending = (label: string) => pendingActions.includes(label);

  const action = (p: ChatGptProfile, kind: ChatGptAction) => run(`${kind}:${p.id}`, async () => {
    const next = await api.chatGptProfileAction(p.id, kind);
    if (!alive.current) return;
    setState(next);
    diagnosticGeneration.current.set(p.id, (diagnosticGeneration.current.get(p.id) ?? 0) + 1);
    setDiagnostics(current => { const next = { ...current }; delete next[p.id]; return next; });
    if (kind === "delete") setDeleting(null);
  });

  const diagnose = async (p: ChatGptProfile) => {
    if (diagnosticTasks.current.has(p.id)) return;
    const generation = (diagnosticGeneration.current.get(p.id) ?? 0) + 1;
    diagnosticGeneration.current.set(p.id, generation);
    setDiagnostics(current => ({ ...current, [p.id]: { pending: true, healthy: false, details: [], error: null } }));
    const task = coordinate(() => api.chatGptDiagnose(p.id));
    diagnosticTasks.current.set(p.id, task);
    try {
      const result = await task;
      if (alive.current && generation === diagnosticGeneration.current.get(p.id)) setDiagnostics(current => ({ ...current, [p.id]: { pending: false, healthy: result.healthy, details: result.details, error: null } }));
    } catch (error) {
      if (alive.current && generation === diagnosticGeneration.current.get(p.id)) setDiagnostics(current => ({ ...current, [p.id]: { pending: false, healthy: false, details: [], error: String(error) } }));
    } finally {
      if (diagnosticTasks.current.get(p.id) === task) diagnosticTasks.current.delete(p.id);
    }
  };
  useEffect(() => {
    if (!active || !state) return;
    for (const profile of state.profiles) {
      if (profile.status === "stopped" && !diagnostics[profile.id] && !diagnosticTasks.current.has(profile.id)) void diagnose(profile);
    }
  }, [active, state, diagnostics]);

  const manualRefresh = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try { await operationTail.current; await refresh(); }
    finally { if (alive.current) setRefreshing(false); }
  };

  const pick = () => run("pick", async () => {
    if (!state) return;
    const path = await open({ title: state.pickerTitle, multiple: false, filters: [{ name: "ChatGPT", extensions: state.pickerExtensions }] });
    if (typeof path === "string") { const next = await api.chatGptSetInstallation(path); if (alive.current) setState(next); }
  });

  const openPrimary = () => run("open-primary", () => api.chatGptOpenPrimary());

  const save = () => run("save", async () => {
    if (!creating || !name.trim()) return;
    const next = await api.chatGptCreateProfile(name.trim());
    if (alive.current) {
      const added = next.profiles.find(p => !state?.profiles.some(old => old.id === p.id));
      if (added) setInitialCopy({ source: copySource, target: added.id });
      setState(next); setCreating(false); setName(""); setCopySource(null);
    }
  });

  const openApiConfig = (profile: ChatGptProfile) => {
    setApiConfigId(profile.id);
    setApiBaseUrl(profile.api?.baseUrl || "https://api.openai.com/v1");
    setApiModel(profile.api?.model || "");
    setApiModels(profile.api?.models || []);
    setApiModelsError(null);
    setApiManualModel(false);
    setApiKey("");
  };
  const closeApiConfig = () => { apiDiscoveryGeneration.current++; setApiConfigId(null); setApiKey(""); setApiModelsLoading(false); };
  useEffect(() => {
    if (!active || !apiConfigId || !apiBaseUrl.trim()) return;
    if (!apiKey.trim() && (!apiHasSavedKey || apiSavedBaseUrl !== apiBaseUrl.trim().replace(/\/$/, ""))) return;
    const generation = ++apiDiscoveryGeneration.current;
    const timer = setTimeout(async () => {
      setApiModelsLoading(true);
      setApiModelsError(null);
      try {
        const models = await api.chatGptDiscoverApiModels(apiConfigId, apiBaseUrl, apiKey);
        if (!alive.current || generation !== apiDiscoveryGeneration.current) return;
        setApiModels(models);
        setApiManualModel(false);
        setApiModel(current => models.some(model => model.id === current) ? current : models[0]?.id || "");
      } catch (error) {
        if (!alive.current || generation !== apiDiscoveryGeneration.current) return;
        setApiModels([]);
        setApiModelsError(String(error));
      } finally {
        if (alive.current && generation === apiDiscoveryGeneration.current) setApiModelsLoading(false);
      }
    }, 650);
    return () => { clearTimeout(timer); apiDiscoveryGeneration.current++; };
  }, [active, apiConfigId, apiBaseUrl, apiKey, apiSavedBaseUrl, apiHasSavedKey, apiRefreshSequence]);
  const saveApiConfig = () => {
    if (!apiConfigId) return;
    void run(`api:${apiConfigId}`, async () => {
      const next = await api.chatGptSaveApiConfig(apiConfigId, apiBaseUrl, apiModel, apiKey, apiManualModel);
      if (alive.current) {
        diagnosticGeneration.current.set(apiConfigId, (diagnosticGeneration.current.get(apiConfigId) ?? 0) + 1);
        setDiagnostics(current => { const updated = { ...current }; delete updated[apiConfigId]; return updated; });
        setState(next); closeApiConfig();
      }
    });
  };
  const useAccountLogin = () => {
    if (!apiConfigId) return;
    void run(`api:${apiConfigId}`, async () => {
      const next = await api.chatGptUseAccountLogin(apiConfigId);
      if (alive.current) {
        diagnosticGeneration.current.set(apiConfigId, (diagnosticGeneration.current.get(apiConfigId) ?? 0) + 1);
        setDiagnostics(current => { const updated = { ...current }; delete updated[apiConfigId]; return updated; });
        setState(next); closeApiConfig();
      }
    });
  };

  const setCompatibility = (profile: ChatGptProfile, enabled: boolean) => {
    void run(`compatibility:${profile.id}`, async () => {
      const next = await api.chatGptSetApiCompatibility(profile.id, enabled);
      if (alive.current) setState(next);
    });
  };

  const profiles = state?.profiles ?? [];

  return <div className="view-scroll chatgpt-scroll"><Stack className={`chatgpt-page${state && profiles.length === 0 ? " chatgpt-page-empty" : ""}`} gap="md">
    <section className="chatgpt-console" aria-label="ChatGPT 客户端">
      <div className="chatgpt-console-top">
        <div className="chatgpt-client-identity">
          <span className="chatgpt-client-icon"><IconBrandOpenai size={24} /></span>
          <div className="chatgpt-client-copy">
            <Group gap="xs"><Text fw={700}>{state?.installation ? `ChatGPT ${state.installation.version}` : "官方 ChatGPT 客户端"}</Text><Badge size="sm" variant="light" color={state?.installation?.compatible ? "teal" : "orange"}>{state?.installation?.compatible ? "已连接" : "待检查"}</Badge></Group>
            {!state?.installation && <Text size="xs" c="dimmed">{state?.installationIssue ?? "正在检测客户端…"}</Text>}
            {state?.installation && !state.installation.compatible && <Text size="xs" c="red">{state.installation.detail}</Text>}
          </div>
        </div>
        <Group gap="xs" className="chatgpt-console-actions">
          {state?.installation && <Button variant="subtle" leftSection={pending("open-primary") ? <IconLoader2 size={16} className="chatgpt-button-spinner" /> : <IconBrandOpenai size={16} />} aria-busy={pending("open-primary")} onClick={() => void openPrimary()}>打开主 ChatGPT</Button>}
          <Button variant="subtle" leftSection={pending("pick") ? <IconLoader2 size={16} className="chatgpt-button-spinner" /> : <IconFolderOpen size={16} />} aria-busy={pending("pick")} onClick={() => void pick()}>选择客户端</Button>
          <Button variant="subtle" leftSection={<IconRefresh size={16} className={refreshing ? "chatgpt-button-spinner" : undefined} />} aria-busy={refreshing} onClick={() => void manualRefresh()}>刷新</Button>
          {profiles.length > 0 && <Button leftSection={<IconPlus size={16} />} onClick={() => { setCreating(true); setName(""); }}>创建实例</Button>}
        </Group>
      </div>
      {state && profiles.length === 0 && <div className="chatgpt-onboarding">
        <div className="chatgpt-onboarding-art" aria-hidden="true"><span><IconBrandOpenai size={28} /></span><span><IconBrandOpenai size={28} /></span></div>
        <Title order={3}>让不同登录，各自独立</Title>
        <Text size="sm" c="dimmed">创建独立实例后，可在官方窗口选择 ChatGPT 账号或 API Key 登录。</Text>
        <Button size="md" leftSection={<IconPlus size={18} />} onClick={() => { setCreating(true); setName(""); }}>创建第一个实例</Button>
      </div>}
    </section>

    {state && profiles.length > 0 && <section className="chatgpt-profile-section" aria-label="独立实例">
      <Title order={4}>独立实例</Title>
      <div className="chatgpt-profile-list">
      {profiles.map((p) => { const diagnostic = diagnostics[p.id]; const tone = p.status === "stopped" ? !diagnostic || diagnostic.pending ? "checking" : diagnostic.healthy ? "healthy" : "warning" : p.status; return <div key={p.id} className="chatgpt-profile" data-profile-tone={tone}>
        <div className="chatgpt-profile-top">
          <div className="chatgpt-profile-main">
            <span className="chatgpt-profile-icon"><IconBrandOpenai size={23} /></span>
            <div className="chatgpt-profile-copy">
              <Text fw={700} className="chatgpt-profile-name">{p.name}</Text>
              {p.api && <Text size="xs" c="dimmed" className="chatgpt-profile-api-mode">{p.api.active ? `API · ${p.api.model}` : "账号登录 · API 配置已保存"}</Text>}
            </div>
          </div>
          <div className="chatgpt-profile-health">
            {p.status === "stopped" && (!diagnostic || diagnostic.pending ? <span className="chatgpt-health-label chatgpt-health-pending"><IconLoader2 size={14} className="chatgpt-button-spinner" />检查中</span> : diagnostic.healthy ? <span className="chatgpt-health-label chatgpt-health-ok"><IconCircleCheck size={14} />正常</span> : <Button size="compact-xs" variant="subtle" color="orange" leftSection={<IconInfoCircle size={15} />} onClick={() => setDiagnosticDetailsId(p.id)}>{diagnostic.details.some(line => line.includes("客户端未返回已登录账号")) ? "待登录" : "需处理"}</Button>)}
          </div>
        </div>
        <div className={`chatgpt-runtime-status chatgpt-runtime-${p.status}${p.status === "stopped" ? ` chatgpt-runtime-${tone}` : ""}`} aria-label={`实例状态：${STATUS[p.status]}`}>
          <span className="chatgpt-runtime-line" aria-hidden="true" />
        </div>
        <div className="chatgpt-profile-actions">
            <Button className="chatgpt-profile-action" size="sm" variant="light" leftSection={pending(`launch:${p.id}`) || pending(`focus:${p.id}`) ? <IconLoader2 size={15} className="chatgpt-button-spinner" /> : <IconPlayerPlay size={15} />}
              aria-busy={pending(`launch:${p.id}`) || pending(`focus:${p.id}`)}
              disabled={p.status === "error" || p.status === "closing" || (p.status !== "running" && !state.installation?.compatible)}
              onClick={() => void action(p, p.status === "running" ? "focus" : "launch")}>{p.status === "running" ? "打开窗口" : "启动"}</Button>
            <Button className="chatgpt-profile-action" size="sm" variant="light" leftSection={<IconKey size={15} />} onClick={() => openApiConfig(p)}>API 配置</Button>
            {p.status === "closing" && <Button className="chatgpt-profile-action" size="sm" variant="light" leftSection={pending(`cleanup:${p.id}`) ? <IconLoader2 size={15} className="chatgpt-button-spinner" /> : <IconRefresh size={15} />} aria-busy={pending(`cleanup:${p.id}`)} onClick={() => void action(p, "cleanup")}>清理后台进程</Button>}
            {p.status === "running" && <Button className="chatgpt-profile-action" size="sm" variant="light" leftSection={pending(`stop:${p.id}`) ? <IconLoader2 size={15} className="chatgpt-button-spinner" /> : <IconPlayerStop size={15} />} aria-busy={pending(`stop:${p.id}`)} onClick={() => void action(p, "stop")}>关闭</Button>}
            {p.status === "stopped" && <Button className="chatgpt-profile-action chatgpt-profile-action-danger" size="sm" variant="light" color="red" leftSection={<IconX size={15} />} onClick={() => setDeleting(p)}>删除</Button>}
        </div>
        {p.api?.active && <div className="chatgpt-profile-compatibility" data-enabled={p.api.compatibilityEnabled ? "true" : "false"}>
          <Switch color="teal" label="兼容模式" description={<><span>对话一直无回复时可尝试开启。开启后，此实例暂时不能上网查资料。</span><span>不懂就不要开启。</span></>}
            checked={!!p.api.compatibilityEnabled}
            disabled={p.status !== "stopped" || pending(`compatibility:${p.id}`)}
            onChange={event => setCompatibility(p, event.currentTarget.checked)} />
        </div>}
        {p.issue && <Alert className="chatgpt-profile-issue" color="red">{p.issue}</Alert>}
      </div>; })}
      </div>
    </section>}

    {message && <div className="chatgpt-feedback" role="alert"><Text size="sm">{message}</Text><Button size="compact-xs" variant="subtle" aria-label="收起错误提示" onClick={() => setMessage(null)}><IconX size={15} /></Button></div>}
    {!state ? <div className="chatgpt-loading" role="status" aria-live="polite" aria-busy="true">
      <div className="chatgpt-loading-orbit" aria-hidden="true">
        <span className="chatgpt-loading-halo" />
        <span className="chatgpt-loading-track" />
        <span className="chatgpt-loading-core"><IconBrandOpenai size={38} stroke={1.6} /></span>
        <span className="chatgpt-loading-satellite"><i /></span>
      </div>
      <div className="chatgpt-loading-copy">
        <Text className="chatgpt-loading-title">正在准备你的 ChatGPT</Text>
        <Text className="chatgpt-loading-subtitle">正在检测客户端与实例…</Text>
      </div>
      <div className="chatgpt-loading-dots" aria-hidden="true"><i /><i /><i /></div>
      <div className="chatgpt-loading-preview" aria-hidden="true">
        {[0, 1, 2].map(index => <div className="chatgpt-loading-card" key={index}><span /><div><i /><i /></div></div>)}
      </div>
    </div> : profiles.length > 0 && <>
      <ChatGptPanelHistory state={state} active={active} initialCopy={initialCopy} coordinate={coordinate} />
    </>}

    <Modal opened={active && creating} onClose={() => { if (!pending("save")) setCreating(false); }} title="创建 ChatGPT 实例" centered>
      <Stack>
        <TextInput label="实例名称" placeholder="例如：工作账号" value={name} maxLength={40} onChange={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === "Enter" && !e.nativeEvent.isComposing && name.trim()) void save(); }} />
        <Select label="创建后选择本地记录（可选）" clearable value={copySource} onChange={setCopySource} data={[{ value: "default", label: "默认实例的本地 Codex 记录" }, ...profiles.map(p => ({ value: p.id, label: p.name }))]} />
        <Text size="sm" c="dimmed">在官方 ChatGPT 窗口选择账号或 API Key 登录。创建后可选择复制本地 Codex 工作记录。</Text>
        <Group justify="flex-end"><Button leftSection={pending("save") ? <IconLoader2 size={16} className="chatgpt-button-spinner" /> : <IconDeviceFloppy size={16} />} aria-busy={pending("save")} disabled={!name.trim()} onClick={() => void save()}>保存</Button></Group>
      </Stack>
    </Modal>
    <Modal opened={active && apiConfigId !== null} onClose={() => { if (!apiConfigId || !pending(`api:${apiConfigId}`)) closeApiConfig(); }} title={`${profiles.find(p => p.id === apiConfigId)?.name ?? "实例"} · API 配置`} centered>
      <Stack gap="sm">
        <Text size="sm" c="dimmed">为这个实例设置 Responses API。保存后启动实例，即可使用 API 模式进入程序；主 ChatGPT 和其他实例不受影响。</Text>
        <TextInput label="API 基础地址" placeholder="https://api.openai.com/v1" value={apiBaseUrl} onChange={e => { setApiBaseUrl(e.currentTarget.value); setApiModels([]); setApiModel(""); setApiManualModel(false); }} />
        <PasswordInput label="API Key" placeholder={profiles.find(p => p.id === apiConfigId)?.api?.hasKey ? "已保存；留空表示继续使用" : "填写 API Key"} value={apiKey} onChange={e => { setApiKey(e.currentTarget.value); setApiModels([]); setApiModel(""); setApiManualModel(false); }} autoComplete="new-password" />
        {!apiManualModel && <Select label="默认模型" placeholder={apiModelsLoading ? "正在读取网关模型…" : "填写地址和 API Key 后自动读取"} value={apiModel || null} onChange={value => setApiModel(value || "")} data={apiModels.map(model => ({ value: model.id, label: model.name === model.id ? model.id : `${model.name} · ${model.id}` }))} searchable disabled={apiModelsLoading || apiModels.length === 0} />}
        {apiManualModel && <TextInput label="模型 ID" placeholder="填写网关支持的模型 ID" value={apiModel} onChange={e => setApiModel(e.currentTarget.value)} />}
        {apiModelsLoading && <Text size="xs" c="dimmed"><IconLoader2 size={13} className="chatgpt-button-spinner" /> 正在读取网关模型…</Text>}
        {apiModelsError && <Alert color="orange">{apiModelsError}</Alert>}
        {apiModelsError && <Button variant="subtle" size="xs" disabled={apiModelsLoading} onClick={() => setApiRefreshSequence(current => current + 1)}>重新读取模型</Button>}
        {apiModelsError && /HTTP (404|405|501)|格式无效|未返回 OpenAI|没有返回可用模型/.test(apiModelsError) && <Button variant="subtle" size="xs" onClick={() => { if (apiManualModel) setApiRefreshSequence(current => current + 1); setApiManualModel(current => !current); setApiModel(""); }}>{apiManualModel ? "返回自动读取" : "网关不提供模型列表？手动填写"}</Button>}
        {apiModels.length > 0 && <Text size="xs" c="dimmed">已读取 {apiModels.length} 个模型并写入该实例的模型目录。官方客户端的模型菜单可能需要先有 ChatGPT 账号登录记录；若菜单没有显示，可关闭实例后在这里更换默认模型。</Text>}
        <Text size="xs" c="dimmed">地址通常以 /v1 结尾。默认直连需要 Responses API；仅支持 Anthropic Messages 的网关可在保存后开启卡片上的兼容模式。若网关不提供 /models，可手动填写模型 ID。API Key 保存在该实例的本地配置中，不会显示在面板里。</Text>
        {profiles.find(p => p.id === apiConfigId)?.status !== "stopped" && <Alert color="orange">请先关闭实例，再修改登录方式。重新启动后配置才会生效。</Alert>}
        <Group justify="space-between">
          {profiles.find(p => p.id === apiConfigId)?.api?.active ? <Button variant="subtle" disabled={profiles.find(p => p.id === apiConfigId)?.status !== "stopped" || !!apiConfigId && pending(`api:${apiConfigId}`)} onClick={useAccountLogin}>切回账号登录</Button> : <span />}
          <Button leftSection={apiConfigId && pending(`api:${apiConfigId}`) ? <IconLoader2 size={16} className="chatgpt-button-spinner" /> : <IconDeviceFloppy size={16} />} aria-busy={!!apiConfigId && pending(`api:${apiConfigId}`)} disabled={!apiBaseUrl.trim() || !apiModel.trim() || apiModelsLoading || !(apiKey.trim() || profiles.find(p => p.id === apiConfigId)?.api?.hasKey) || profiles.find(p => p.id === apiConfigId)?.status !== "stopped" || !!apiConfigId && pending(`api:${apiConfigId}`)} onClick={saveApiConfig}>保存并启用 API</Button>
        </Group>
      </Stack>
    </Modal>
    <RiskConfirm opened={active && deleting !== null} level="high" title={`删除实例：${deleting?.name ?? ""}`}
      consequences={["仅可删除已关闭的实例。将移除其独立目录：登录数据、本地会话、缓存、日志及未完成的复制队列。", "外部项目文件、其他实例和默认账号的数据不会删除。", "云端账号和云端记录不会被此操作删除。删除失败时会提示残留路径。"]}
      confirmLabel="删除实例数据" busy={!!deleting && pending(`delete:${deleting.id}`)} onCancel={() => { if (!deleting || !pending(`delete:${deleting.id}`)) setDeleting(null); }} onConfirm={() => { if (deleting) void action(deleting, "delete"); }} />
    <Modal opened={active && diagnosticDetailsId !== null} onClose={() => setDiagnosticDetailsId(null)} title={`${profiles.find(p => p.id === diagnosticDetailsId)?.name ?? "实例"} · 检查详情`} centered>
      <Stack gap="sm" className="chatgpt-diagnostic-modal">
        {diagnosticDetailsId && diagnostics[diagnosticDetailsId]?.error && <Text c="red" size="sm">{diagnostics[diagnosticDetailsId].error}</Text>}
        {diagnosticDetailsId && diagnostics[diagnosticDetailsId]?.details.map((line, index) => <div key={`${index}:${line}`}><span>{index < 2 ? <IconCircleCheck size={17} /> : <IconInfoCircle size={17} />}</span><Text size="sm">{line}</Text></div>)}
        <Group justify="flex-end"><Button variant="subtle" onClick={() => { const profile = profiles.find(p => p.id === diagnosticDetailsId); setDiagnosticDetailsId(null); if (profile) void diagnose(profile); }}>重新检查</Button><Button variant="default" onClick={() => setDiagnosticDetailsId(null)}>知道了</Button></Group>
      </Stack>
    </Modal>
  </Stack></div>;
}
