import { useEffect, useState } from "react";
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
import type {
  WorkBuddyCertificateStatus,
  WorkBuddyOrganization,
  WorkBuddyState,
} from "../api";

const DEFAULT_ENDPOINT = "https://10.0.147.128:8080";

interface OrganizationForm {
  name: string;
  modelPrefix: string;
  url: string;
  apiKey: string;
}

const emptyOrganization = (): OrganizationForm => ({
  name: "",
  modelPrefix: "",
  url: DEFAULT_ENDPOINT,
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

export default function WorkBuddyPanel() {
  const [state, setState] = useState<WorkBuddyState | null>(null);
  const [selectedOrganizationId, setSelectedOrganizationId] = useState<string | null>(null);
  const [organizationForm, setOrganizationForm] = useState<OrganizationForm>(emptyOrganization);
  const [editingOrganization, setEditingOrganization] = useState(false);
  const [catalog, setCatalog] = useState<string[]>([]);
  const [selectedModels, setSelectedModels] = useState<string[]>([]);
  const [busy, setBusy] = useState("");
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

  const refreshCertificate = async (url: string) => {
    setCertificateStatus({ state: "checking", detail: "正在检测网关证书…" });
    try {
      setCertificateStatus(await api.checkWorkBuddyCertificate(url));
    } catch (error) {
      setCertificateStatus({ state: "unreachable", detail: String(error) });
    }
  };

  const fetchModels = async (organizationId: string, quiet = false) => {
    setBusy("models");
    try {
      const models = await api.listWorkBuddyOrganizationModels(organizationId);
      setCatalog(models);
      setSelectedModels((current) => migrateSelectedModels(current, models));
      if (!quiet) {
        setMessage({
          ok: true,
          text: `已从该组织网关实时获取 ${models.length} 个 WorkBuddy 可用模型，请勾选需要使用的模型。`,
        });
      }
    } catch (error) {
      setCatalog([]);
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy("");
    }
  };

  const activateOrganization = (organization: WorkBuddyOrganization, fetch = true) => {
    setSelectedOrganizationId(organization.id);
    setOrganizationForm(formFromOrganization(organization));
    setEditingOrganization(false);
    setCatalog([]);
    setSelectedModels(organization.selectedModels);
    setMessage({ ok: true, text: "" });
    void refreshCertificate(organization.url);
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
        void refreshCertificate(DEFAULT_ENDPOINT);
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

  const startNewOrganization = () => {
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
    try {
      const next = await api.saveWorkBuddyOrganization(
        selectedOrganizationId || undefined,
        organizationForm.name.trim(),
        organizationForm.modelPrefix.trim(),
        organizationForm.url.trim(),
        organizationForm.apiKey.trim() || undefined
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
      await refreshCertificate(selectedOrganization?.url || DEFAULT_ENDPOINT);
      setMessage({ ok: true, text: "已保存 WorkBuddy 安装位置；后续启动和证书同步都会使用该路径。" });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
      await load();
    } finally {
      setBusy("");
    }
  };

  const importCertificate = async () => {
    if (!certificatePath) return;
    setBusy("certificate");
    try {
      const result = await api.importWorkBuddyCa(certificatePath);
      setCertificatePath(null);
      await refreshCertificate(selectedOrganization?.url || DEFAULT_ENDPOINT);
      setMessage({ ok: true, text: result });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy("");
    }
  };

  const openWorkBuddy = async () => {
    try {
      await api.launchWorkBuddy();
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
        opened={deleteOpen}
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

      <Modal
        opened={Boolean(certificatePath)}
        onClose={() => setCertificatePath(null)}
        title="确认导入网关证书"
        centered
      >
        <Stack>
          <Alert color="orange" icon={<IconCertificate size={16} />}>
            {isMacos
              ? "将所选 CA 同步到 WorkBuddy.app 的内置 CLI 证书文件。不会修改 macOS 系统钥匙串；WorkBuddy 更新后，管理中心会在启动时自动补写。请仅导入公司网关管理员提供的证书。"
              : "将所选 CA 加入当前 Windows 用户的“受信任根证书”存储，并同步到本机共享的 WorkBuddy 安装目录。此后，这台电脑上使用该 WorkBuddy 安装的所有用户都会在自定义模型请求中信任该 CA。请仅导入公司网关管理员提供的证书；WorkBuddy 更新后，管理中心会在启动时自动补写。"}
          </Alert>
          <Text size="sm" c="dimmed" lineClamp={2}>{certificatePath}</Text>
          <Group justify="flex-end">
            <Button variant="default" onClick={() => setCertificatePath(null)}>取消</Button>
            <Button loading={busy === "certificate"} onClick={() => void importCertificate()}>
              确认并信任
            </Button>
          </Group>
        </Stack>
      </Modal>

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
        MaaS Gateway 会收集与模型交互的请求和响应用于公司审计，请勿提交个人隐私或非公司事务信息。组织 Key 仅保存在本机 WorkBuddy 配置中，不会写入 Claude Code。
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
              <Button size="xs" variant="light" leftSection={<IconPlus size={14} />} onClick={startNewOrganization}>
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
                        placeholder={DEFAULT_ENDPOINT}
                        value={organizationForm.url}
                        onChange={(event) => {
                          const url = event.currentTarget.value;
                          setOrganizationForm((current) => ({ ...current, url }));
                        }}
                        onBlur={() => void refreshCertificate(organizationForm.url.trim() || DEFAULT_ENDPOINT)}
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
                        loading={busy === "models"}
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
