import { useEffect, useRef, useState } from "react";
import { usePageActivation } from "./PersistentPage";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Alert,
  Badge,
  Button,
  Card,
  Checkbox,
  Code,
  Group,
  Modal,
  NavLink,
  PasswordInput,
  SimpleGrid,
  Stack,
  Text,
  TextInput,
  Title,
} from "@mantine/core";
import {
  IconAlertTriangle,
  IconBrandOpenai,
  IconBuilding,
  IconCertificate,
  IconDeviceFloppy,
  IconExternalLink,
  IconFolderOpen,
  IconPencil,
  IconPlus,
  IconRefresh,
  IconTrash,
} from "@tabler/icons-react";
import { api } from "../api";
import RiskConfirm from "./RiskConfirm";
import type {
  WorkBuddyCertificateStatus,
  WorkBuddyOrganization,
  WorkBuddyState,
} from "../api";

// 只作输入框的 placeholder 示例用。**不再作为表单预填值** ——
// 原先预填的是一个公司内网地址，公开用户打开页面时地址栏已被填好，
// 不动它直接保存就会指向不可达内网（既是信息泄漏，也是功能问题）。
const EXAMPLE_ENDPOINT = "https://gateway.example.com:8080";

interface OrganizationForm {
  name: string;
  modelPrefix: string;
  url: string;
  apiKey: string;
}

const emptyOrganization = (): OrganizationForm => ({
  name: "",
  modelPrefix: "",
  url: "",
  apiKey: "",
});

const formFromOrganization = (organization: WorkBuddyOrganization): OrganizationForm => ({
  name: organization.name,
  modelPrefix: organization.modelPrefix,
  url: organization.url,
  apiKey: "",
});

const migrateSelectedModels = (selected: string[], catalog: string[]) => {
  const exact = new Set(catalog);
  const byLowerCase = new Map(catalog.map((id) => [id.toLowerCase(), id]));
  return Array.from(
    new Set(
      selected.flatMap((id) => {
        if (exact.has(id)) return [id];
        const openAiAlias = byLowerCase.get(`o${id.toLowerCase()}`);
        return openAiAlias ? [openAiAlias] : [];
      })
    )
  ).sort();
};

export default function WorkBuddyPanel({ active = true }: { active?: boolean }) {
  const [state, setState] = useState<WorkBuddyState | null>(null);
  const [selectedOrganizationId, setSelectedOrganizationId] = useState<string | null>(null);
  const [organizationForm, setOrganizationForm] = useState<OrganizationForm>(emptyOrganization);
  const [editingOrganization, setEditingOrganization] = useState(false);
  const [catalog, setCatalog] = useState<string[]>([]);
  const [selectedModels, setSelectedModels] = useState<string[]>([]);
  const [busy, setBusy] = useState("");
  const [modelsBusy, setModelsBusy] = useState(false);
  const modelRequest = useRef(0);
  const certificateRequest = useRef(0);
  const invalidateRequests = () => {
    modelRequest.current += 1;
    certificateRequest.current += 1;
    setModelsBusy(false);
  };
  useEffect(() => () => {
    modelRequest.current += 1;
    certificateRequest.current += 1;
  }, []);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [certificatePath, setCertificatePath] = useState<string | null>(null);
  const [certificateStatus, setCertificateStatus] = useState<WorkBuddyCertificateStatus>({
    state: "checking",
    detail: "正在检测网关证书…",
  });
  const [message, setMessage] = useState<{ ok: boolean; text: string }>({ ok: true, text: "" });

  const selectedOrganization = state?.organizations.find(
    (organization) => organization.id === selectedOrganizationId
  );

  const refreshCertificate = async (url: string, quiet = false) => {
    const request = ++certificateRequest.current;
    if (!quiet) setCertificateStatus({ state: "checking", detail: "正在检测网关证书…" });
    try {
      const result = await api.checkWorkBuddyCertificate(url);
      if (request === certificateRequest.current) setCertificateStatus(result);
    } catch (error) {
      if (request === certificateRequest.current) setCertificateStatus({ state: "unreachable", detail: String(error) });
    }
  };

  // 没有地址就不探测证书：否则会立刻报一个假的「证书检测失败」。
  // 换成中性的灰徽章，提示用户填完地址会自动检测。
  //
  // 必须返回 Promise<void>：调用方有 `await probeCertificate(...)` 与
  // `Promise.all([...])` 两种用法，返回 void 时 await 会被当成"立刻完成"，
  // 导致后台 in-flight 提前释放、证书检测结果晚于后续文案出现。
  const probeCertificate = async (url: string, quiet = false): Promise<void> => {
    const target = url.trim();
    if (!target) {
      setCertificateStatus({
        state: "notRequired",
        detail: "填写网关地址后将自动检测证书。",
      });
      return;
    }
    await refreshCertificate(target, quiet);
  };

  const fetchModels = async (organizationId: string, quiet = false, background = false) => {
    const request = ++modelRequest.current;
    if (!background) setModelsBusy(true);
    try {
      const models = await api.listWorkBuddyOrganizationModels(organizationId);
      if (request !== modelRequest.current) return;
      setCatalog(models);
      if (!background) setSelectedModels((current) => migrateSelectedModels(current, models));
      if (!quiet) {
        setMessage({
          ok: true,
          text: `已从该组织网关实时获取 ${models.length} 个 WorkBuddy 可用模型，请勾选需要使用的模型。`,
        });
      }
    } catch (error) {
      if (request !== modelRequest.current) return;
      if (!background) setCatalog([]);
      setMessage({ ok: false, text: String(error) });
    } finally {
      if (request === modelRequest.current) setModelsBusy(false);
    }
  };

  const activateOrganization = (organization: WorkBuddyOrganization, fetch = true) => {
    invalidateRequests();
    setSelectedOrganizationId(organization.id);
    setOrganizationForm(formFromOrganization(organization));
    setEditingOrganization(false);
    setCatalog([]);
    setSelectedModels(organization.selectedModels);
    setMessage({ ok: true, text: "" });
    void probeCertificate(organization.url);
    if (fetch) void fetchModels(organization.id, true);
  };

  const load = async () => {
    setBusy("load");
    try {
      const next = await api.workBuddyState();
      setState(next);
      const current = next.organizations.find(
        (organization) => organization.id === selectedOrganizationId
      );
      const organization = current || next.organizations[0];
      if (organization) {
        activateOrganization(organization);
      } else {
        setSelectedOrganizationId(null);
        setOrganizationForm(emptyOrganization());
        setEditingOrganization(true);
        setCatalog([]);
        setSelectedModels([]);
        void probeCertificate("");
      }
      setMessage({ ok: true, text: "" });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy("");
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const backgroundInFlight = useRef(false);
  usePageActivation(() => {
    if (backgroundInFlight.current || busy || modelsBusy) return;
    backgroundInFlight.current = true;
    const generation = modelRequest.current;
    void api.workBuddyState().then(async (next) => {
      if (generation !== modelRequest.current) return;
      setState(next);
      const current = next.organizations.find((item) => item.id === selectedOrganizationId);
      if (current && !editingOrganization) {
        await Promise.all([
          fetchModels(current.id, true, true),
          probeCertificate(current.url, true),
        ]);
      }
    }).catch((error) => {
      if (generation === modelRequest.current) setMessage({ ok: false, text: `后台更新失败，已保留当前内容：${String(error)}` });
    }).finally(() => { backgroundInFlight.current = false; });
  }, active);

  const startNewOrganization = () => {
    invalidateRequests();
    probeCertificate("");
    setSelectedOrganizationId(null);
    setOrganizationForm(emptyOrganization());
    setEditingOrganization(true);
    setCatalog([]);
    setSelectedModels([]);
    setMessage({ ok: true, text: "新增组织只需配置一次网关地址和系统 Key。" });
  };

  const saveOrganization = async () => {
    if (!organizationForm.name.trim()) {
      setMessage({ ok: false, text: "请填写组织名称。" });
      return;
    }
    if (!/^https?:\/\/\S+$/i.test(organizationForm.url.trim())) {
      setMessage({ ok: false, text: "请填写有效的网关地址。" });
      return;
    }
    if (!organizationForm.apiKey.trim() && !selectedOrganization?.hasApiKey) {
      setMessage({ ok: false, text: "首次保存组织时必须填写系统 Key。" });
      return;
    }
    setBusy("organization");
    invalidateRequests();
    try {
      const next = await api.saveWorkBuddyOrganization(
        selectedOrganizationId || undefined,
        organizationForm.name.trim(),
        organizationForm.modelPrefix.trim(),
        organizationForm.url.trim(),
        organizationForm.apiKey.trim() || undefined,
        // 与 model 写入同级：文件被别的程序改过时后端会拒绝覆盖
        state?.organizationsRevision ?? ""
      );
      setState(next);
      const organization = selectedOrganizationId
        ? next.organizations.find((item) => item.id === selectedOrganizationId)
        : [...next.organizations]
            .reverse()
            .find(
              (item) =>
                item.name === organizationForm.name.trim() &&
                item.url === organizationForm.url.trim().replace(/\/$/, "")
            );
      if (organization) {
        activateOrganization(organization);
      }
      setMessage({ ok: true, text: "组织网关已保存。以后只需进入该组织勾选模型。" });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy("");
    }
  };

  const applyModels = async () => {
    if (!selectedOrganizationId) return;
    setBusy("apply");
    invalidateRequests();
    try {
      const next = await api.applyWorkBuddyOrganizationModels(
        selectedOrganizationId,
        selectedModels
      );
      setState(next);
      const organization = next.organizations.find(
        (item) => item.id === selectedOrganizationId
      );
      if (organization) setSelectedModels(organization.selectedModels);
      setMessage({
        ok: true,
        text: `已将 ${selectedModels.length} 个模型同步到 WorkBuddy，通常会在 1 秒内出现在模型列表中。`,
      });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy("");
    }
  };

  const deleteOrganization = async () => {
    if (!selectedOrganizationId) return;
    setBusy("delete");
    invalidateRequests();
    try {
      const next = await api.deleteWorkBuddyOrganization(selectedOrganizationId);
      setState(next);
      setDeleteOpen(false);
      const organization = next.organizations[0];
      if (organization) activateOrganization(organization);
      else startNewOrganization();
      setMessage({ ok: true, text: "组织及其在 WorkBuddy 中管理的模型已移除。" });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy("");
    }
  };

  const chooseCertificate = async () => {
    try {
      const selected = await open({
        title: "选择 MaaS Gateway CA 根证书",
        directory: false,
        multiple: false,
        filters: [{ name: "CA 证书", extensions: ["pem", "crt", "cer"] }],
      });
      if (!selected || Array.isArray(selected)) return;
      setCertificatePath(selected);
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    }
  };

  const chooseWorkBuddyExecutable = async () => {
    try {
      const macos = state?.environment.platform === "macos";
      const selected = await open({
        title: macos ? "选择 WorkBuddy.app" : "选择 WorkBuddy.exe",
        directory: false,
        multiple: false,
        filters: [{ name: "WorkBuddy 应用程序", extensions: [macos ? "app" : "exe"] }],
      });
      if (!selected || Array.isArray(selected)) return;
      setBusy("executable");
      const next = await api.setWorkBuddyExecutable(selected);
      setState(next);
      await probeCertificate(selectedOrganization?.url || "");
      setMessage({ ok: true, text: "已保存 WorkBuddy 安装位置；后续启动和证书同步都会使用该路径。" });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
      await load();
    } finally {
      setBusy("");
    }
  };

  // 返回**是否成功**。原先把错误吞进 message 后正常 resolve，外层无从判断，
  // 于是导入失败也照样关窗、清掉已选文件 —— 用户要重新挑一次文件才能重试。
  const importCertificate = async (): Promise<boolean> => {
    if (!certificatePath) return false;
    setBusy("certificate");
    try {
      const result = await api.importWorkBuddyCa(certificatePath);
      await probeCertificate(selectedOrganization?.url || "");
      setMessage({ ok: true, text: result });
      return true;
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
      return false;
    } finally {
      setBusy("");
    }
  };

  const openWorkBuddy = async () => {
    try {
      // 后端可能带回"CA 未能写进安装目录，已改用环境变量注入"这类提示，要显示出来
      setMessage({ ok: true, text: await api.launchWorkBuddy() });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    }
  };

  const environment = state?.environment;
  const isMacos = environment?.platform === "macos";
  const locked = environment?.configValid === false;
  const allModelIds = Array.from(new Set([...catalog, ...selectedModels])).sort();
  const certificateBadge = {
    trusted: { color: "teal", label: "证书已信任" },
    untrusted: { color: "orange", label: "证书未信任" },
    notRequired: { color: "gray", label: "无需证书" },
    unreachable: { color: "red", label: "证书检测失败" },
    checking: { color: "gray", label: "证书检测中" },
  }[certificateStatus.state];

  return (
    <div className="workbuddy-page">
      <Modal
        opened={active && deleteOpen}
        onClose={() => setDeleteOpen(false)}
        title="删除组织"
        centered
      >
        <Stack>
          <Alert color="red" icon={<IconAlertTriangle size={16} />}>
            将删除“{selectedOrganization?.name}”以及由该组织写入 WorkBuddy 的模型，不影响其他组织。
          </Alert>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setDeleteOpen(false)}>取消</Button>
            <Button color="red" loading={busy === "delete"} onClick={() => void deleteOrganization()}>
              确认删除
            </Button>
          </Group>
        </Stack>
      </Modal>

      <RiskConfirm
        opened={active && Boolean(certificatePath)}
        // 两个平台都会**改变信任边界**，所以都定 critical：
        //   - 该 CA 会经 import_cert 进入应用信任库 ⇒ 全部托管 Claude 环境都信任它；
        //   - Windows 还会写当前用户 Root 库 + WorkBuddy 共享 ca.pem。
        // macOS 只动 WorkBuddy 内置 CLI 的证书文件（不碰系统钥匙串），但"所有环境"那一条同样成立。
        level="critical"
        title="导入网关 CA 证书"
        consequences={
          isMacos
            ? [
                "该 CA 会同时加入应用信任库 —— 此后全部托管 Claude 环境都会信任它签发的任意证书。",
                "同时同步到 WorkBuddy.app 内置 CLI 的证书文件。",
                "不会修改 macOS 系统钥匙串，也不影响系统层面的信任设置。",
                "WorkBuddy 更新后，管理中心会在下次启动时自动补写。",
                "请只导入公司网关管理员提供的证书。",
              ]
            : [
                // 后端用的是 certutil -user，作用域是当前登录用户，不是全机。
                // 另有共享面（WorkBuddy 安装目录里的 ca.pem）与"所有托管 Claude 环境"，三者要分开说。
                "该 CA 会同时加入应用信任库 —— 此后所有托管 Claude 环境都会信任它签发的任意证书。",
                "还会加入「当前 Windows 用户」的受信任根证书库：只对你自己生效，不改动其他用户账户。",
                "并写入 WorkBuddy 安装目录下的共享 ca.pem，那个文件是该安装的所有用户共用的。",
                "WorkBuddy 更新后，管理中心会在下次启动时自动补写。",
                "请只导入公司网关管理员提供的证书。",
              ]
        }
        detail={certificatePath}
        confirmLabel="确认并信任"
        busy={busy === "certificate"}
        onCancel={() => setCertificatePath(null)}
        onConfirm={async () => {
          // 只有成功才关窗并清掉路径；失败保留上下文供直接重试
          if (await importCertificate()) setCertificatePath(null);
        }}
      />

      <Card withBorder padding="md" radius="lg" className="workbuddy-environment-card">
        <Group justify="space-between" align="flex-start">
          <Group align="flex-start" wrap="nowrap">
            <div className={`workbuddy-status-icon ${environment?.found ? "ready" : "warn"}`}>
              <IconBrandOpenai size={22} />
            </div>
            <div>
              <Group gap="xs">
                <Text fw={700}>WorkBuddy 环境</Text>
                <Badge color={environment?.found ? "teal" : "orange"} variant="light">
                  {environment?.found
                    ? `已安装${environment.version ? ` · v${environment.version}` : ""}`
                    : "未检测到"}
                </Badge>
              </Group>
              <Text size="xs" c="dimmed" mt={4}>{environment?.detail || "正在检测…"}</Text>
              {environment && (
                <Stack gap={2} mt={4}>
                  {environment.executablePath && (
                    <Text size="xs" c="dimmed">
                      安装位置：<Code>{environment.executablePath}</Code>
                    </Text>
                  )}
                  <Text size="xs" c="dimmed">
                    配置文件：<Code>{environment.configPath}</Code>
                  </Text>
                </Stack>
              )}
            </div>
          </Group>
          <Group gap="xs">
            <Button variant="default" leftSection={<IconRefresh size={15} />} loading={busy === "load"} onClick={() => void load()}>
              重新检测
            </Button>
            <Button
              variant="default"
              leftSection={<IconFolderOpen size={15} />}
              loading={busy === "executable"}
              disabled={!environment}
              onClick={() => void chooseWorkBuddyExecutable()}
            >
              选择安装位置
            </Button>
            <Button variant="light" leftSection={<IconExternalLink size={15} />} disabled={!environment?.found} onClick={() => void openWorkBuddy()}>
              打开 WorkBuddy
            </Button>
          </Group>
        </Group>
      </Card>

      <Alert color="orange" variant="light" icon={<IconAlertTriangle size={16} />}>
        MaaS Gateway 会收集与模型交互的请求和响应用于公司审计，请勿提交个人隐私或非公司事务信息。组织 Key 仅保存在本机 WorkBuddy 配置中（未加密的明文文件；macOS 显式限本人可读，Windows 依赖用户目录的继承权限），不会写入 Claude Code。
      </Alert>

      {(state?.warnings.length || 0) > 0 && (
        <Alert color="red" title="WorkBuddy 配置需要处理">{state?.warnings.join("；")}</Alert>
      )}
      {message.text && <Alert color={message.ok ? "teal" : "red"}>{message.text}</Alert>}

      <div className="workbuddy-grid">
        <div className="workbuddy-list-pane">
          <Card withBorder padding="sm" radius="lg" className="workbuddy-list-card">
            <Group justify="space-between" mb="sm" px={4}>
              <div>
                <Title order={5}>组织与网关</Title>
                <Text size="xs" c="dimmed">{state?.organizations.length || 0} 个组织</Text>
              </div>
              <Button size="xs" variant="light" leftSection={<IconPlus size={14} />} disabled={busy !== ""} onClick={startNewOrganization}>
                新增组织
              </Button>
            </Group>
            <Stack gap={4}>
              {state?.organizations.map((organization) => (
                <NavLink
                  key={organization.id}
                  active={selectedOrganizationId === organization.id}
                  label={<Text size="sm" fw={650}>{organization.name}</Text>}
                  description={`${organization.selectedModels.length} 个模型`}
                  leftSection={<IconBuilding size={16} />}
                  rightSection={<span className={`instance-health-dot ${organization.hasApiKey ? "ok" : "warn"}`} />}
                  disabled={busy !== ""}
                  onClick={() => activateOrganization(organization)}
                />
              ))}
              {!state?.organizations.length && (
                <Text size="sm" c="dimmed" p="xs">先添加一个组织网关，之后只需勾选模型。</Text>
              )}
            </Stack>
          </Card>
        </div>

        <div className="workbuddy-editor-scroll">
          <Card withBorder padding="lg" radius="lg" className="workbuddy-editor-card">
            <Stack gap="md">
              <Group justify="space-between" className="editor-toolbar">
                <div>
                  <Title order={4}>{selectedOrganization?.name || "新增组织网关"}</Title>
                </div>
                {selectedOrganization && !editingOrganization && (
                  <Group gap="xs">
                    <Button variant="subtle" color="red" leftSection={<IconTrash size={15} />} onClick={() => setDeleteOpen(true)}>
                      删除组织
                    </Button>
                    <Button variant="default" leftSection={<IconPencil size={15} />} onClick={() => setEditingOrganization(true)}>
                      编辑组织
                    </Button>
                  </Group>
                )}
              </Group>

              {editingOrganization ? (
                <Card withBorder radius="lg" padding="xl" className="workbuddy-setup-card">
                  <Stack gap="sm">
                    <SimpleGrid cols={{ base: 1, sm: 2 }}>
                      <TextInput
                        label="组织名称"
                        placeholder="例如 北京研发网关"
                        value={organizationForm.name}
                        onChange={(event) => {
                          const name = event.currentTarget.value;
                          setOrganizationForm((current) => ({ ...current, name }));
                        }}
                      />
                      <TextInput
                        label="模型前缀"
                        description="可选；填写后显示为“前缀:模型 ID”，留空则只显示模型 ID。"
                        placeholder="例如 company"
                        value={organizationForm.modelPrefix}
                        onChange={(event) => {
                          const modelPrefix = event.currentTarget.value;
                          setOrganizationForm((current) => ({ ...current, modelPrefix }));
                        }}
                      />
                    </SimpleGrid>
                    <TextInput
                        label="网关地址"
                        placeholder={EXAMPLE_ENDPOINT}
                        value={organizationForm.url}
                        onChange={(event) => {
                          const url = event.currentTarget.value;
                          certificateRequest.current += 1;
                          setCertificateStatus({ state: "checking", detail: "地址已修改，离开输入框后重新检测。" });
                          setOrganizationForm((current) => ({ ...current, url }));
                        }}
                        onBlur={() => void probeCertificate(organizationForm.url)}
                    />
                    <PasswordInput
                      label="系统 Key"
                      description={selectedOrganization?.hasApiKey ? "系统 Key 已保存；留空保持不变。" : "填写管理员分配的 gw-sk-..."}
                      placeholder={selectedOrganization?.hasApiKey ? "已保存，留空不修改" : "gw-sk-..."}
                      value={organizationForm.apiKey}
                      onChange={(event) => {
                        const apiKey = event.currentTarget.value;
                        setOrganizationForm((current) => ({ ...current, apiKey }));
                      }}
                    />
                    <Group justify="space-between">
                      <Group gap="xs">
                        <Badge
                          color={certificateBadge.color}
                          variant="light"
                          leftSection={<IconCertificate size={12} />}
                          title={certificateStatus.detail}
                        >
                          {certificateBadge.label}
                        </Badge>
                        <Button
                          size="xs"
                          variant="default"
                          leftSection={<IconCertificate size={14} />}
                          onClick={() => void chooseCertificate()}
                        >
                          导入证书
                        </Button>
                      </Group>
                      <Group gap="xs">
                      {selectedOrganization && (
                        <Button variant="default" onClick={() => {
                          setOrganizationForm(formFromOrganization(selectedOrganization));
                          setEditingOrganization(false);
                        }}>
                          取消
                        </Button>
                      )}
                      <Button leftSection={<IconDeviceFloppy size={15} />} loading={busy === "organization"} onClick={() => void saveOrganization()}>
                        保存组织
                      </Button>
                      </Group>
                    </Group>
                  </Stack>
                </Card>
              ) : selectedOrganization ? (
                <>
                  <Card withBorder radius="md" padding="md" className="workbuddy-organization-summary">
                    <Group justify="space-between">
                      <div>
                        <Text size="xs" c="dimmed">当前网关</Text>
                        <Text fw={650}>{selectedOrganization.url}</Text>
                      </div>
                      <Group gap="xs">
                        {selectedOrganization.modelPrefix && (
                          <Badge color="blue" variant="light">
                            模型前缀 {selectedOrganization.modelPrefix}
                          </Badge>
                        )}
                        <Badge color="teal" variant="light">系统 Key 已配置</Badge>
                        <Badge
                          color={certificateBadge.color}
                          variant="light"
                          leftSection={<IconCertificate size={12} />}
                          title={certificateStatus.detail}
                        >
                          {certificateBadge.label}
                        </Badge>
                        <Button
                          size="xs"
                          variant="default"
                          leftSection={<IconCertificate size={14} />}
                          onClick={() => void chooseCertificate()}
                        >
                          导入证书
                        </Button>
                      </Group>
                    </Group>
                  </Card>

                  <Group justify="space-between" align="flex-end">
                    <div>
                      <Title order={5}>选择 WorkBuddy 模型</Title>
                      <Text size="xs" c="dimmed">
                        已选择 {selectedModels.length} 个；同一模型 ID 不能同时属于两个组织。
                      </Text>
                    </div>
                    <Group gap="xs">
                      <Button variant="default" size="xs" onClick={() => setSelectedModels(allModelIds)}>全选</Button>
                      <Button variant="default" size="xs" onClick={() => setSelectedModels([])}>清空</Button>
                      <Button
                        variant="light"
                        size="xs"
                        leftSection={<IconRefresh size={14} />}
                        loading={modelsBusy}
                        onClick={() => void fetchModels(selectedOrganization.id)}
                      >
                        刷新模型
                      </Button>
                    </Group>
                  </Group>

                  {allModelIds.length ? (
                    <Checkbox.Group value={selectedModels} onChange={setSelectedModels}>
                      <SimpleGrid cols={{ base: 1, sm: 2, lg: 3 }} className="workbuddy-model-checklist">
                        {allModelIds.map((model) => (
                          <Card key={model} withBorder radius="md" padding="sm">
                            <Checkbox value={model} label={model} />
                          </Card>
                        ))}
                      </SimpleGrid>
                    </Checkbox.Group>
                  ) : (
                    <Alert color="blue">
                      尚未获取模型列表。点击“刷新模型”，系统会使用该组织保存的网关和 Key 自动读取。
                    </Alert>
                  )}

                  <Group justify="flex-end">
                    <Button
                      leftSection={<IconDeviceFloppy size={15} />}
                      loading={busy === "apply"}
                      disabled={locked}
                      onClick={() => void applyModels()}
                    >
                      保存已选模型
                    </Button>
                  </Group>
                </>
              ) : null}
            </Stack>
          </Card>
        </div>
      </div>
    </div>
  );
}
