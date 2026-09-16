import { invoke } from "@tauri-apps/api/core";

// 与 src-tauri 里的 #[tauri::command] 及 serde 结构一一对应

/** Profile（serde rename_all = camelCase） */
export interface Profile {
  name: string;
  type: "router" | "account";
  baseUrl: string;
  tokenEnc?: string;
  hasToken: boolean;
  opusModel: string;
  sonnetModel: string;
  haikuModel: string;
}

/** EnvInfo（serde 未重命名，保持 snake_case） */
export interface EnvInfo {
  platform: string;
  claude_found: boolean;
  claude_detection: ClaudeDetection;
  integrated: boolean;
  cert_imported: boolean;
  cert_count: number;
}

export type ClaudeDetectionStatus = "ready" | "notFound" | "unusable";
export type ClaudeDetectionSource =
  | "cache"
  | "processPath"
  | "loginShell"
  | "packageManager"
  | "fallback"
  | "manual";

export interface ClaudeDetection {
  found: boolean;
  status: ClaudeDetectionStatus;
  path?: string;
  version?: string;
  source?: ClaudeDetectionSource;
  detail: string;
  checkedPaths: string[];
  shellWarning?: string;
}

export interface ProfileRuntimeInfo {
  name: string;
  configDir: string;
  settingsExists: boolean;
  hasProjectData: boolean;
  lastUsed?: number;
  authenticated: boolean;
  sharedDirsOk: boolean;
}

export type ResourceKind = "skills" | "agents";
export type ResourceTargetStatus = "inherited" | "override" | "excluded" | "missing" | "local" | "unavailable" | "target";

export interface ResourceTargetState {
  target: string;
  label: string;
  state: ResourceTargetStatus;
  reason: string;
  issues: string[];
}

export interface ResourceItem {
  name: string;
  inShared: boolean;
  inDefaultClaude: boolean;
  targets: ResourceTargetState[];
}

export interface ResourceOverview {
  kind: ResourceKind;
  label: string;
  sharedPath: string;
  items: ResourceItem[];
  targets: ResourceTargetState[];
  autoImportEnabled: boolean;
  lastAutoImportAt?: number;
  lastAutoImportAdded: number;
  lastAutoImportSkipped: number;
  lastAutoImportFailures: string[];
}

// ---------------- MCP 服务管理 ----------------
// 与 src-tauri/src/mcp/mod.rs 的 serde 结构一一对应（camelCase）。

export type McpScope = "user" | "local" | "project";
export type McpTransport = "stdio" | "http" | "sse" | "ws" | "unknown";
export type McpEffectiveState =
  | "effective"
  | "partially-shadowed"
  | "shadowed"
  | "disabled";

export interface McpLocator {
  scope: McpScope;
  name: string;
  instanceId?: string;
  projectPath?: string;
}

export interface McpSaveItem {
  target: McpLocator;
  config: Record<string, unknown>;
  overwrite: boolean;
}

export type McpChangeAction =
  | {
      op: "save";
      original?: McpLocator;
      target: McpLocator;
      config: Record<string, unknown>;
      overwrite?: boolean;
    }
  | { op: "batchSave"; items: McpSaveItem[] }
  | { op: "setEnabled"; target: McpLocator; enabled: boolean }
  | { op: "delete"; target: McpLocator };

export interface McpInstanceRef {
  id: string;
  label: string;
}

export interface McpProjectRef {
  path: string;
  label: string;
  discovered: boolean;
}

export interface McpShadowRef {
  scope: McpScope;
  name: string;
  instanceId?: string;
  projectPath?: string;
}

export interface McpService {
  locator: McpLocator;
  transport: McpTransport;
  rawTransport?: string;
  config: Record<string, unknown>;
  enabled: boolean;
  effectiveState: McpEffectiveState;
  shadowedBy: McpShadowRef[];
  shadowedContextCount: number;
  sourceId: string;
  revision: string;
  sensitivePaths: string[];
  warnings: string[];
}

export interface McpSourceIssue {
  sourceId: string;
  path: string;
  detail: string;
}

/** 一条"死条目"：记录存在、目录已确认不存在，可被一键清理 */
export interface McpDeadEntry {
  /** user:<环境>（.claude.json 的 projects 键）| manager:projects（登记表条目） */
  sourceId: string;
  /** 记录中的原始路径/键 */
  rawPath: string;
  /** 所在文件（.claude.json 或 mcp-projects.json） */
  filePath: string;
}

/** 一键清理的结构化结果：前端据此区分成功/无事可做/部分失败 */
export interface McpCleanupResult {
  removedCount: number;
  blockedCount: number;
  writeErrorCount: number;
  /** true = 无写失败（可关闭确认弹窗）；部分失败保持弹窗供重试 */
  complete: boolean;
  /** 人类可读报告 */
  message: string;
}

export interface McpSummary {
  total: number;
  enabled: number;
  disabled: number;
  warnings: number;
  shadowed: number;
}

export type McpSyncStatus =
  | "not-synced"
  | "synced"
  | "source-updated"
  | "target-modified"
  | "conflict"
  | "incompatible";

export interface McpSyncTargetInfo {
  locator: McpLocator;
  targetId: string;
  targetLabel: string;
  status: McpSyncStatus;
  connected: boolean;
  targetPath: string;
  targetRevision: string;
  detail: string;
}

export interface McpState {
  services: McpService[];
  instances: McpInstanceRef[];
  projects: McpProjectRef[];
  revisions: Record<string, string>;
  issues: McpSourceIssue[];
  /** 可被「一键清理」安全删除的死条目（目录确认不存在）；与 issues 有意冗余 */
  deadEntries: McpDeadEntry[];
  summary: McpSummary;
  operationWarnings: string[];
  syncTargets: McpSyncTargetInfo[];
  syncTargetRevisions: Record<string, string>;
  /** 哪些环境的哪些条目覆盖了共享配置（决策 7.2：必须显示，不能静默以环境为准） */
  sharedOverrides: McpSharedOverride[];
}

/** 某个环境的插件继承状态 */
export interface PluginEnvState {
  env: string;
  value?: boolean | null;
  /** true = 与共享值一致（继承）；false = 该环境独立覆盖 */
  inherited: boolean;
  /** 覆盖原因（继承时为空） */
  reason: string;
  installed?: boolean;
  version?: string;
  storageIndependent?: boolean;
  excluded?: boolean;
}

/** 插件启用状态总览的一行 */
export interface PluginRow {
  name: string;
  /** 共享库里的值（null = 共享库里没有这一条） */
  shared?: boolean | null;
  /** 默认 Claude 的值 —— 只展示，不可编辑 */
  defaultClaude?: boolean | null;
  defaultInstalled?: boolean;
  defaultVersion?: string;
  envs: PluginEnvState[];
}

export type PluginAction = "install" | "update" | "uninstall" | "enable" | "disable";
export interface PluginActionOutcome {
  env: string;
  ok: boolean;
  detail: string;
}
export interface PluginActionReport {
  action: PluginAction;
  plugin: string;
  results: PluginActionOutcome[];
  reloadHint: string;
  policyWarning?: string;
}

export interface McpSharedOverride {
  env: string;
  name: string;
  /** 人类可读的原因，如「已覆盖共享配置（你修改过它）」 */
  reason: string;
  /** 共享库里的值；共享库没有该条时为 undefined */
  sharedValue?: Record<string, unknown> | null;
  /** 环境里的值；该环境已删除这条时为 undefined */
  envValue?: Record<string, unknown> | null;
}

export interface McpSyncPreview {
  locator: McpLocator;
  targetId: string;
  targetLabel: string;
  actionLabel: string;
  targetPath: string;
  redactedBefore?: Record<string, unknown>;
  redactedAfter: Record<string, unknown>;
  warnings: string[];
  expectedSourceRevision: string;
  expectedTargetRevision: string;
  expectedRegistryRevision: string;
  restartHint: string;
  preservedFieldsNote: string;
}

export interface McpSyncApplyRequest {
  locator: McpLocator;
  targetId: string;
  expectedSourceRevision: string;
  expectedTargetRevision: string;
  expectedRegistryRevision: string;
}

export interface McpTargetDisableRequest {
  locator: McpLocator;
  targetId: string;
  expectedTargetRevision: string;
  expectedRegistryRevision: string;
}

export interface McpChangeRequest {
  action: McpChangeAction;
  expectedRevisions: Record<string, string>;
}

export interface McpAffectedSource {
  sourceId: string;
  path: string;
  scope: McpScope;
}

export interface McpBatchItem {
  name: string;
  scope: McpScope;
  sourceId: string;
  redactedBefore?: Record<string, unknown>;
  redactedAfter?: Record<string, unknown>;
}

export interface McpChangePreview {
  actionLabel: string;
  affectedSources: McpAffectedSource[];
  affectedInstances: string[];
  redactedBefore?: Record<string, unknown>;
  redactedAfter?: Record<string, unknown>;
  batchItems: McpBatchItem[];
  userSyncNote?: string;
  warnings: string[];
  expectedRevisions: Record<string, string>;
}

export interface McpTestRequest {
  locator?: McpLocator;
  name: string;
  config: Record<string, unknown>;
}

export interface McpTestStage {
  id: "schema" | "command" | "url" | "endpoint";
  status: "ok" | "warn" | "fail" | "skipped";
  detail: string;
}

export interface McpTestResult {
  ok: boolean;
  transport: McpTransport;
  stages: McpTestStage[];
  sanitizedDetail: string;
}

export interface UsageRow {
  datetime: string; // UTC，如 "2026-06-22T04"
  model: string;
  profile: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheCreate: number;
  requests: number;
}

export interface ConvRow {
  datetime: string;
  profile: string;
}

export interface UsageStats {
  daily: UsageRow[];
  conversations: ConvRow[];
  totalInput: number;
  totalOutput: number;
  totalRequests: number;
  totalConversations: number;
}

/** ModelPinWarning（serde rename_all = camelCase）：/model 钉死具体型号、绕过档位映射 */
export interface ModelPinWarning {
  profile: string; // 环境名；默认 Claude为 __main__
  model: string;
  settingsPath: string;
}

/** InstanceSettings（serde rename_all = camelCase）：某个独立环境的 settings.json */
export interface InstanceSettings {
  path: string;
  exists: boolean;
  content: string;
  /** 文件 mtime（毫秒）。保存时原样回传，用于检测后台 --sync 的并发改写 */
  revision: string;
  bypassEnabled: boolean;
  /** 有更高优先级的配置也设了 defaultMode 时，给出那个文件的路径 */
  overriddenBy?: string;
}

/** HealthItem（serde rename_all = camelCase） */
export interface HealthItem {
  id: string;
  label: string;
  status: "ok" | "warn" | "fail";
  detail: string;
}

export interface WorkBuddyEnvironment {
  found: boolean;
  platform: "windows" | "macos" | "other";
  executablePath?: string;
  version?: string;
  configPath: string;
  configExists: boolean;
  configValid: boolean;
  detail: string;
}

export interface WorkBuddyModel {
  id: string;
  name: string;
  vendor: string;
  url: string;
  hasApiKey: boolean;
  maxInputTokens: number;
  maxOutputTokens: number;
  supportsToolCall: boolean;
  supportsImages: boolean;
  supportsReasoning: boolean;
  useCustomProtocol: boolean;
  visible: boolean;
  usesGlobalKey: boolean;
}

export interface WorkBuddyGatewayConfig {
  url: string;
  hasApiKey: boolean;
}

export interface WorkBuddyOrganization {
  id: string;
  name: string;
  modelPrefix: string;
  url: string;
  selectedModels: string[];
  hasApiKey: boolean;
}

export interface WorkBuddyState {
  environment: WorkBuddyEnvironment;
  gateway: WorkBuddyGatewayConfig;
  organizations: WorkBuddyOrganization[];
  models: WorkBuddyModel[];
  /** `models.json` 的修订号 */
  revision: string;
  /** `cc-manager-gateway.json` 的修订号 */
  gatewayRevision: string;
  /** `cc-manager-organizations.json` 的修订号 */
  organizationsRevision: string;
  warnings: string[];
}

export interface WorkBuddyModelInput extends Omit<WorkBuddyModel, "hasApiKey" | "usesGlobalKey"> {
  apiKey?: string;
  useGlobalKey: boolean;
}

export interface WorkBuddyTestResult {
  ok: boolean;
  statusCode: number;
  detail: string;
}

export interface WorkBuddyCertificateStatus {
  state: "checking" | "trusted" | "untrusted" | "notRequired" | "unreachable";
  detail: string;
}

// 与 src-tauri 里的 #[tauri::command] 一一对应
export const api = {
  listProfiles: (): Promise<Profile[]> => invoke("list_profiles"),
  saveProfile: (
    profile: Omit<Profile, "hasToken" | "tokenEnc">,
    token: string | null
  ): Promise<string> =>
    invoke("save_profile", { profile, token: token || null }),
  deleteProfile: (name: string): Promise<string> =>
    invoke("delete_profile", { name }),
  // 刷新终端集成，并把共享 Skills / Agents / MCP 单向分发到受管理环境
  syncAll: (): Promise<string> => invoke("sync_all"),
  environment: (): Promise<EnvInfo> => invoke("environment"),
  setClaudeExecutable: (path: string): Promise<ClaudeDetection> =>
    invoke("set_claude_executable", { path }),
  profileRuntimeInfo: (): Promise<ProfileRuntimeInfo[]> => invoke("profile_runtime_info"),
  resourceOverview: (kind: ResourceKind): Promise<ResourceOverview> =>
    invoke("resource_overview", { kind }),
  importDefaultResource: (kind: ResourceKind, name?: string, replace = false): Promise<string> =>
    invoke("import_default_resource", { kind, name: name ?? null, replace }),
  installResourceFromPath: (kind: ResourceKind, path: string, targetId?: string): Promise<string> =>
    invoke("install_resource_from_path", { kind, path, targetId: targetId ?? null }),
  setResourceExcluded: (
    kind: ResourceKind,
    targetId: string,
    name: string,
    excluded: boolean
  ): Promise<string> => invoke("set_resource_excluded", { kind, targetId, name, excluded }),
  restoreResourceInheritance: (
    kind: ResourceKind,
    targetId: string,
    name: string
  ): Promise<string> => invoke("restore_resource_inheritance", { kind, targetId, name }),
  deleteSharedResource: (kind: ResourceKind, name: string): Promise<string> =>
    invoke("delete_shared_resource", { kind, name }),
  syncExtensionResources: (): Promise<string> => invoke("sync_extension_resources"),
  setResourceAutoImport: (enabled: boolean): Promise<string> =>
    invoke("set_resource_auto_import", { enabled }),
  pluginTargets: (): Promise<string[]> => invoke("plugin_targets"),
  setPluginExcluded: (env: string, plugin: string, excluded: boolean): Promise<string> =>
    invoke("set_plugin_excluded", { env, plugin, excluded }),
  // MCP 服务管理
  listMcpServices: (): Promise<McpState> => invoke("list_mcp_services"),
  registerMcpProject: (path: string): Promise<McpState> =>
    invoke("register_mcp_project", { path }),
  unregisterMcpProject: (path: string): Promise<McpState> =>
    invoke("unregister_mcp_project", { path }),
  previewMcpChange: (request: McpChangeRequest): Promise<McpChangePreview> =>
    invoke("preview_mcp_change", { request }),
  applyMcpChange: (request: McpChangeRequest): Promise<McpState> =>
    invoke("apply_mcp_change", { request }),
  /** 撤销某个环境对被分发条目的覆盖，改回共享值（决策 7.2：必须用户显式触发） */
  restoreSharedMcpEntry: (env: string, name: string): Promise<string> =>
    invoke("restore_shared_mcp_entry", { env, name }),
  /** 一键清理死条目（目录确认不存在的项目键/登记表条目）的结果 */
  cleanupDeadProjectEntries: (): Promise<McpCleanupResult> =>
    invoke("cleanup_dead_project_entries"),
  /** 插件启用状态总览（扩展 → Plugins） */
  pluginsOverview: (): Promise<PluginRow[]> => invoke("plugins_overview"),
  /** 通过 Claude Code 官方命令撤销该环境的独立设置，再恢复共享策略 */
  restorePluginInheritance: (env: string, name: string): Promise<string> =>
    invoke("restore_plugin_inheritance", { env, name }),
  managePlugin: (action: PluginAction, plugin: string, envs: string[], sharedScope: boolean): Promise<PluginActionReport> =>
    invoke("manage_plugin", { action, plugin, envs, sharedScope }),
  testMcpServer: (request: McpTestRequest): Promise<McpTestResult> =>
    invoke("test_mcp_server", { request }),
  previewMcpTargetSync: (
    targetId: string,
    locator: McpLocator
  ): Promise<McpSyncPreview> =>
    invoke("preview_mcp_target_sync", { targetId, locator }),
  applyMcpTargetSync: (request: McpSyncApplyRequest): Promise<McpState> =>
    invoke("apply_mcp_target_sync", { request }),
  disableMcpTarget: (request: McpTargetDisableRequest): Promise<McpState> =>
    invoke("disable_mcp_target", { request }),
  backupConfig: (): Promise<string> => invoke("backup_config"),
  recentSyncLog: (): Promise<string[]> => invoke("recent_sync_log"),
  // 兼容入口：作用于**全部网关环境**（保留给不指定目标的调用方）
  importCert: (path: string): Promise<string> => invoke("import_cert", { path }),
  clearCerts: (): Promise<string> => invoke("clear_certs"),
  // 按网关注入/清空：CA 是**每个网关各自一份**，不再全局共用（约束 5）
  importCertFor: (envs: string[], path: string): Promise<string> =>
    invoke("import_cert_for", { envs, path }),
  clearCertsFor: (envs: string[]): Promise<string> =>
    invoke("clear_certs_for", { envs }),
  detectModels: (baseUrl: string, token: string, env = ""): Promise<string[]> =>
    invoke("detect_models", { env, baseUrl, token }),
  detectModelsFor: (name: string): Promise<string[]> =>
    invoke("detect_models_for", { name }),
  usageStats: (): Promise<UsageStats> => invoke("usage_stats"),
  // 独立环境的 settings.json：开关与手动编辑共用同一份读写
  readInstanceSettings: (name: string): Promise<InstanceSettings> =>
    invoke("read_instance_settings", { name }),
  writeInstanceSettings: (
    name: string,
    content: string,
    revision: string
  ): Promise<InstanceSettings> =>
    invoke("write_instance_settings", { name, content, revision }),
  setBypassPermissions: (
    name: string,
    enabled: boolean
  ): Promise<InstanceSettings> =>
    invoke("set_bypass_permissions", { name, enabled }),
  // 健康与诊断
  modelPinWarnings: (): Promise<ModelPinWarning[]> =>
    invoke("model_pin_warnings"),
  fixModelPin: (profile: string): Promise<string> =>
    invoke("fix_model_pin", { profile }),
  healthCheck: (): Promise<HealthItem[]> => invoke("health_check"),
  /** 最近一次完整健康检查。从未检测过 / 记录损坏 / 时间戳来自未来都返回 null */
  lastVerification: (): Promise<
    { at: number; problems: number; gatewayFails?: string[]; appVersion?: string | null } | null
  > => invoke("last_verification"),
  /** 单环境网关连通复测：只探测该环境并同步修正最近验证记录 */
  probeGateway: (env: string): Promise<string> =>
    invoke("probe_gateway", { env }),
  exportDiagnostics: (): Promise<string> => invoke("export_diagnostics"),
  // WorkBuddy 独立模型配置
  workBuddyState: (): Promise<WorkBuddyState> => invoke("workbuddy_state"),
  setWorkBuddyExecutable: (path: string): Promise<WorkBuddyState> =>
    invoke("set_workbuddy_executable", { path }),
  saveWorkBuddyGateway: (
    url: string,
    apiKey: string | undefined,
    expectedRevision: string
  ): Promise<WorkBuddyState> =>
    invoke("save_workbuddy_gateway", {
      request: { url, apiKey: apiKey || null, expectedRevision },
    }),
  saveWorkBuddyOrganization: (
    id: string | undefined,
    name: string,
    modelPrefix: string,
    url: string,
    apiKey: string | undefined,
    /** 来自 state.organizationsRevision；不一致后端会拒绝覆盖 */
    expectedRevision: string
  ): Promise<WorkBuddyState> =>
    invoke("save_workbuddy_organization", {
      request: {
        id: id || null,
        name,
        modelPrefix,
        url,
        apiKey: apiKey || null,
        expectedRevision,
      },
    }),
  deleteWorkBuddyOrganization: (id: string): Promise<WorkBuddyState> =>
    invoke("delete_workbuddy_organization", { id }),
  applyWorkBuddyOrganizationModels: (
    organizationId: string,
    models: string[]
  ): Promise<WorkBuddyState> =>
    invoke("apply_workbuddy_organization_models", {
      request: { organizationId, models },
    }),
  importWorkBuddyCa: (path: string): Promise<string> => invoke("import_workbuddy_ca", { path }),
  listWorkBuddyModels: (id: string | undefined, url: string, apiKey: string | undefined): Promise<string[]> =>
    invoke("list_workbuddy_models", { request: { id: id || null, url, apiKey: apiKey || null } }),
  listWorkBuddyOrganizationModels: (id: string): Promise<string[]> =>
    invoke("list_workbuddy_organization_models", { id }),
  checkWorkBuddyCertificate: (url: string): Promise<WorkBuddyCertificateStatus> =>
    invoke("check_workbuddy_certificate", { url }),
  saveWorkBuddyModel: (
    model: WorkBuddyModelInput,
    previousId: string | undefined,
    expectedRevision: string
  ): Promise<WorkBuddyState> =>
    invoke("save_workbuddy_model", {
      request: { model, previousId, expectedRevision },
    }),
  deleteWorkBuddyModel: (
    id: string,
    expectedRevision: string
  ): Promise<WorkBuddyState> =>
    invoke("delete_workbuddy_model", {
      request: { id, expectedRevision },
    }),
  testWorkBuddyModel: (
    id: string,
    url: string,
    apiKey: string | undefined,
    useCustomProtocol: boolean,
    useGlobalKey: boolean
  ): Promise<WorkBuddyTestResult> =>
    invoke("test_workbuddy_model", {
      request: { id, url, apiKey: apiKey || null, useCustomProtocol, useGlobalKey },
    }),
  // 返回启动结果说明：CA 若未能写进 WorkBuddy 安装目录（权限不足），会在这里带出提示
  launchWorkBuddy: (): Promise<string> => invoke("launch_workbuddy"),
};
