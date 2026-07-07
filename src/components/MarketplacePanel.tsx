import { useEffect, useMemo, useState } from "react";
import {
  Card,
  Stack,
  Group,
  Button,
  TextInput,
  Select,
  SegmentedControl,
  Switch,
  Text,
  Title,
  Badge,
  SimpleGrid,
  ActionIcon,
  Tooltip,
} from "@mantine/core";
import {
  IconSearch,
  IconRefresh,
  IconTrash,
  IconPlus,
  IconDownload,
  IconBuildingStore,
  IconWorld,
  IconExternalLink,
  IconGitBranch,
} from "@tabler/icons-react";
import { notifications } from "@mantine/notifications";
import { api } from "../api";
import type { MarketplaceEntry, PluginListResult, PluginSource } from "../api";

type Tab = "all" | "installed" | "available";

interface CardItem {
  key: string; // pluginId，如 name@marketplace
  name: string;
  marketplaceName: string;
  description?: string;
  installCount?: number;
  installed: boolean;
  enabled?: boolean;
  source?: PluginSource;
  sourceUrl?: string;
  sourceLabel?: string;
  marketUrl?: string;
}

const githubRepoUrl = (repo?: string | null) =>
  repo ? `https://github.com/${repo.replace(/^github:/, "").replace(/\.git$/, "")}` : "";

const normalizeGitUrl = (url?: string) => (url ? url.replace(/\.git$/, "") : "");

const joinGitPath = (repoUrl: string, ref?: string, path?: string) => {
  const clean = normalizeGitUrl(repoUrl);
  if (!clean || !path) return clean;
  return `${clean}/tree/${ref || "main"}/${path.replace(/^\.?\//, "")}`;
};

const sourceUrl = (source: PluginSource | undefined, market?: MarketplaceEntry) => {
  if (!source) return "";
  if (typeof source === "string") {
    if (/^https?:\/\//i.test(source)) return normalizeGitUrl(source);
    const marketRoot = market?.url || githubRepoUrl(market?.repo);
    if (source.startsWith("./") && marketRoot) return joinGitPath(marketRoot, "main", source);
    return marketRoot;
  }
  if (source.url) return joinGitPath(source.url, source.ref, source.path);
  return market?.url || githubRepoUrl(market?.repo);
};

const sourceLabel = (source: PluginSource | undefined) => {
  if (!source) return "market source";
  if (typeof source === "string") {
    if (source.startsWith("./")) return source.replace(/^\.\//, "");
    return source;
  }
  if (source.path) return source.path;
  return source.source || "source";
};

export default function MarketplacePanel() {
  const [markets, setMarkets] = useState<MarketplaceEntry[]>([]);
  const [marketErr, setMarketErr] = useState("");
  const [marketInput, setMarketInput] = useState("");

  const [pluginData, setPluginData] = useState<PluginListResult>({
    installed: [],
    available: [],
  });
  const [pluginErr, setPluginErr] = useState("");
  const [loading, setLoading] = useState(false);

  const [search, setSearch] = useState("");
  const [marketFilter, setMarketFilter] = useState<string | null>("");
  const [tab, setTab] = useState<Tab>("all");
  const [busyId, setBusyId] = useState("");

  const loadMarkets = () => {
    api
      .pluginMarketplaceList()
      .then(setMarkets)
      .catch((e) => setMarketErr(String(e)));
  };

  const loadPlugins = () => {
    setLoading(true);
    setPluginErr("");
    api
      .pluginList()
      .then(setPluginData)
      .catch((e) => setPluginErr(String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    loadMarkets();
    loadPlugins();
  }, []);

  const items: CardItem[] = useMemo(() => {
    const marketByName = new Map(markets.map((m) => [m.name, m]));
    const installedIds = new Set(pluginData.installed.map((p) => p.id));
    const availableById = new Map(pluginData.available.map((p) => [p.pluginId, p]));
    const installed: CardItem[] = pluginData.installed.map((p) => {
      const at = p.id.lastIndexOf("@");
      const name = at >= 0 ? p.id.slice(0, at) : p.id;
      const marketplaceName = at >= 0 ? p.id.slice(at + 1) : p.scope;
      const meta = availableById.get(p.id);
      const market = marketByName.get(meta?.marketplaceName || marketplaceName);
      const src = meta?.source;
      return {
        key: p.id,
        name: meta?.name || name,
        marketplaceName: meta?.marketplaceName || marketplaceName,
        description: meta?.description,
        installCount: meta?.installCount,
        installed: true,
        enabled: p.enabled,
        source: src,
        sourceUrl: sourceUrl(src, market),
        sourceLabel: sourceLabel(src),
        marketUrl: market?.url || githubRepoUrl(market?.repo),
      };
    });
    const available: CardItem[] = pluginData.available
      .filter((p) => !installedIds.has(p.pluginId))
      .map((p) => {
        const market = marketByName.get(p.marketplaceName);
        return {
          key: p.pluginId,
          name: p.name,
          marketplaceName: p.marketplaceName,
          description: p.description,
          installCount: p.installCount,
          installed: false,
          source: p.source,
          sourceUrl: sourceUrl(p.source, market),
          sourceLabel: sourceLabel(p.source),
          marketUrl: market?.url || githubRepoUrl(market?.repo),
        };
      });
    if (tab === "installed") return installed;
    if (tab === "available") return available;
    return [...installed, ...available];
  }, [markets, pluginData, tab]);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return items.filter((it) => {
      if (marketFilter && it.marketplaceName !== marketFilter) return false;
      if (!q) return true;
      return (
        it.name.toLowerCase().includes(q) ||
        (it.description || "").toLowerCase().includes(q)
      );
    });
  }, [items, search, marketFilter]);

  const doInstall = (pluginId: string) => {
    setBusyId(`install:${pluginId}`);
    api
      .pluginInstall(pluginId)
      .then(() => {
        notifications.show({
          color: "teal",
          title: "安装成功",
          message: pluginId,
          autoClose: 3000,
        });
        loadPlugins();
      })
      .catch((e) =>
        notifications.show({
          color: "red",
          title: "安装失败",
          message: String(e),
          autoClose: 6000,
        })
      )
      .finally(() => setBusyId(""));
  };

  const doUninstall = (pluginId: string) => {
    setBusyId(`uninstall:${pluginId}`);
    api
      .pluginUninstall(pluginId)
      .then(() => {
        notifications.show({
          color: "teal",
          title: "已卸载",
          message: pluginId,
          autoClose: 3000,
        });
        loadPlugins();
      })
      .catch((e) =>
        notifications.show({
          color: "red",
          title: "卸载失败",
          message: String(e),
          autoClose: 6000,
        })
      )
      .finally(() => setBusyId(""));
  };

  const doToggle = (pluginId: string, enabled: boolean) => {
    setBusyId(`toggle:${pluginId}`);
    api
      .pluginSetEnabled(pluginId, enabled)
      .then(() => loadPlugins())
      .catch((e) =>
        notifications.show({
          color: "red",
          title: enabled ? "启用失败" : "停用失败",
          message: String(e),
          autoClose: 6000,
        })
      )
      .finally(() => setBusyId(""));
  };

  const doAddMarket = () => {
    const src = marketInput.trim();
    if (!src) return;
    setBusyId("market:add");
    api
      .pluginMarketplaceAdd(src)
      .then(() => {
        setMarketInput("");
        notifications.show({
          color: "teal",
          title: "市场已添加",
          message: src,
          autoClose: 3000,
        });
        loadMarkets();
        loadPlugins();
      })
      .catch((e) =>
        notifications.show({
          color: "red",
          title: "添加市场失败",
          message: String(e),
          autoClose: 6000,
        })
      )
      .finally(() => setBusyId(""));
  };

  const doRemoveMarket = (name: string) => {
    setBusyId(`market:remove:${name}`);
    api
      .pluginMarketplaceRemove(name)
      .then(() => {
        loadMarkets();
        loadPlugins();
      })
      .catch((e) =>
        notifications.show({
          color: "red",
          title: "移除市场失败",
          message: String(e),
          autoClose: 6000,
        })
      )
      .finally(() => setBusyId(""));
  };

  const openLink = (url?: string) => {
    if (!url) return;
    api.openExternalUrl(url).catch((e) =>
      notifications.show({
        color: "red",
        title: "打开链接失败",
        message: String(e),
        autoClose: 5000,
      })
    );
  };

  const marketOptions = [
    { value: "", label: "全部市场" },
    ...markets.map((m) => ({ value: m.name, label: m.name })),
  ];

  return (
    <div style={{ display: "flex", gap: 16, alignItems: "stretch", height: "100%" }}>
      {/* 左栏：市场管理，独立滚动 */}
      <div style={{ flex: "0 0 30%", minWidth: 240, overflowY: "auto" }}>
        <Card withBorder padding="sm" radius="md">
          <Group justify="space-between" mb="xs">
            <Group gap={6}>
              <IconBuildingStore size={16} />
              <Title order={5}>市场</Title>
            </Group>
          </Group>
          <Stack gap={4} mb="sm">
            {markets.length === 0 && (
              <Text size="sm" c="dimmed" p="xs">
                还没有市场。
              </Text>
            )}
            {markets.map((m) => (
              <Group key={m.name} justify="space-between" wrap="nowrap" gap={4}>
                <Group gap={6} wrap="nowrap" style={{ minWidth: 0 }}>
                  <IconWorld size={14} />
                  <Stack gap={0} style={{ minWidth: 0 }}>
                    <Text size="sm" truncate>
                      {m.name}
                    </Text>
                    <Text size="xs" c="dimmed" truncate>
                      {m.repo || m.url || m.source}
                    </Text>
                  </Stack>
                </Group>
                {(m.url || m.repo) && (
                  <Tooltip label="打开市场仓库">
                    <ActionIcon
                      size="sm"
                      variant="subtle"
                      onClick={() => openLink(m.url || githubRepoUrl(m.repo))}
                    >
                      <IconExternalLink size={14} />
                    </ActionIcon>
                  </Tooltip>
                )}
                <Tooltip label="移除该市场">
                  <ActionIcon
                    size="sm"
                    variant="subtle"
                    color="red"
                    loading={busyId === `market:remove:${m.name}`}
                    onClick={() => doRemoveMarket(m.name)}
                  >
                    <IconTrash size={14} />
                  </ActionIcon>
                </Tooltip>
              </Group>
            ))}
          </Stack>
          <Text size="xs" c="dimmed" mb={4}>
            填 owner/repo 或 git 地址，添加自定义市场。
          </Text>
          <Group gap={6}>
            <TextInput
              size="xs"
              placeholder="owner/repo"
              value={marketInput}
              onChange={(e) => setMarketInput(e.currentTarget.value)}
              style={{ flex: 1 }}
            />
            <Button
              size="xs"
              leftSection={<IconPlus size={14} />}
              loading={busyId === "market:add"}
              onClick={doAddMarket}
            >
              添加
            </Button>
          </Group>
          {marketErr && (
            <Text size="xs" c="red" mt="xs">
              {marketErr}
            </Text>
          )}
        </Card>
      </div>

      {/* 右栏：插件检索与安装，独立滚动 */}
      <div style={{ flex: 1, minWidth: 0, overflowY: "auto" }}>
        <Card withBorder padding="lg" radius="md">
          <Stack gap="sm">
            <Group justify="space-between" wrap="nowrap">
              <Title order={5}>Skill / Plugin 市场</Title>
              <Tooltip label="刷新">
                <ActionIcon variant="light" loading={loading} onClick={loadPlugins}>
                  <IconRefresh size={16} />
                </ActionIcon>
              </Tooltip>
            </Group>

            <Group grow>
              <TextInput
                leftSection={<IconSearch size={14} />}
                placeholder="按名称或描述搜索"
                value={search}
                onChange={(e) => setSearch(e.currentTarget.value)}
              />
              <Select
                data={marketOptions}
                value={marketFilter}
                onChange={setMarketFilter}
                allowDeselect={false}
              />
            </Group>
            <SegmentedControl
              value={tab}
              onChange={(v) => setTab(v as Tab)}
              data={[
                { value: "all", label: "全部" },
                { value: "installed", label: `已装 (${pluginData.installed.length})` },
                { value: "available", label: "未装" },
              ]}
            />

            {pluginErr && (
              <Text size="sm" c="red">
                {pluginErr}
              </Text>
            )}

            {!pluginErr && filtered.length === 0 && (
              <Text size="sm" c="dimmed" p="xs">
                没有匹配的插件。
              </Text>
            )}

            <SimpleGrid cols={{ base: 1, md: 2 }} spacing="sm">
              {filtered.map((it) => (
                <Card key={it.key} withBorder padding="sm" radius="md">
                  <Stack gap={4}>
                    <Group justify="space-between" wrap="nowrap">
                      <Group gap={6} wrap="nowrap" style={{ minWidth: 0 }}>
                        <Text fw={600} size="sm" truncate>
                          {it.name}
                        </Text>
                        {it.sourceUrl && (
                          <Tooltip label="打开仓库或插件目录">
                            <ActionIcon
                              size="sm"
                              variant="subtle"
                              onClick={() => openLink(it.sourceUrl)}
                            >
                              <IconExternalLink size={14} />
                            </ActionIcon>
                          </Tooltip>
                        )}
                      </Group>
                      {it.installed ? (
                        <Badge size="sm" color="teal" variant="light">
                          已安装
                        </Badge>
                      ) : (
                        <Badge size="sm" color="gray" variant="light">
                          {it.marketplaceName}
                        </Badge>
                      )}
                    </Group>
                    {it.description && (
                      <Text size="xs" c="dimmed" lineClamp={4}>
                        {it.description}
                      </Text>
                    )}
                    <Group gap={6} wrap="nowrap" mt={2}>
                      <IconGitBranch size={13} opacity={0.55} />
                      <Text size="xs" c="dimmed" truncate>
                        {it.sourceLabel || it.marketplaceName}
                      </Text>
                    </Group>
                    <Group justify="space-between" mt={4}>
                      <Text size="xs" c="dimmed">
                        {it.installed
                          ? it.marketplaceName
                          : it.installCount != null
                          ? `${it.installCount} 次安装`
                          : ""}
                      </Text>
                      <Group gap={6}>
                        {it.installed && (
                          <Switch
                            size="xs"
                            checked={!!it.enabled}
                            onChange={(e) =>
                              doToggle(it.key, e.currentTarget.checked)
                            }
                            disabled={busyId === `toggle:${it.key}`}
                          />
                        )}
                        {it.installed ? (
                          <Button
                            size="xs"
                            variant="subtle"
                            color="red"
                            leftSection={<IconTrash size={14} />}
                            loading={busyId === `uninstall:${it.key}`}
                            onClick={() => doUninstall(it.key)}
                          >
                            卸载
                          </Button>
                        ) : (
                          <Button
                            size="xs"
                            leftSection={<IconDownload size={14} />}
                            loading={busyId === `install:${it.key}`}
                            onClick={() => doInstall(it.key)}
                          >
                            安装
                          </Button>
                        )}
                      </Group>
                    </Group>
                  </Stack>
                </Card>
              ))}
            </SimpleGrid>

            <Text size="xs" c="dimmed" mt="xs">
              安装/卸载都作用于主账户（--scope user），完成后自动同步给所有实例。
            </Text>
          </Stack>
        </Card>
      </div>
    </div>
  );
}
