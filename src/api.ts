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

export interface ExtensionGroup {
  kind: "skills" | "plugins" | "agents" | "commands";
  label: string;
  path: string;
  items: string[];
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
  summary: McpSummary;
  operationWarnings: string[];
  syncTargets: McpSyncTargetInfo[];
  syncTargetRevisions: Record<string, string>;
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
  profile: string; // 实例名；主账户为 __main__
  model: string;
  settingsPath: string;
}

/** InstanceSettings（serde rename_all = camelCase）：某个独立空间的 settings.json */
export interface InstanceSettings {
  path: string;
  exists: boolean;
  content: string;
  /** 文件 mtime（毫秒）。保存时原样回传，用于检测后台 --sync 的并发改写 */
  revision: number;
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
  revision: string;
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
  deleteProfile: (name: string, purgeData = false): Promise<string> =>
    invoke("delete_profile", { name, purgeData }),
  // 刷新集成脚本 + 建齐共享链接 + 合并同步 MCP/插件启用状态
  syncAll: (): Promise<string> => invoke("sync_all"),
  environment: (): Promise<EnvInfo> => invoke("environment"),
  setClaudeExecutable: (path: string): Promise<ClaudeDetection> =>
    invoke("set_claude_executable", { path }),
  profileRuntimeInfo: (): Promise<ProfileRuntimeInfo[]> => invoke("profile_runtime_info"),
  extensionOverview: (): Promise<ExtensionGroup[]> => invoke("extension_overview"),
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
  importCert: (path: string): Promise<string> => invoke("import_cert", { path }),
  clearCerts: (): Promise<string> => invoke("clear_certs"),
  detectModels: (baseUrl: string, token: string): Promise<string[]> =>
    invoke("detect_models", { baseUrl, token }),
  detectModelsFor: (name: string): Promise<string[]> =>
    invoke("detect_models_for", { name }),
  usageStats: (): Promise<UsageStats> => invoke("usage_stats"),
  // 独立空间的 settings.json：开关与手动编辑共用同一份读写
  readInstanceSettings: (name: string): Promise<InstanceSettings> =>
    invoke("read_instance_settings", { name }),
  writeInstanceSettings: (
    name: string,
    content: string,
    revision: number
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
  exportDiagnostics: (): Promise<string> => invoke("export_diagnostics"),
  // WorkBuddy 独立模型配置
  workBuddyState: (): Promise<WorkBuddyState> => invoke("workbuddy_state"),
  setWorkBuddyExecutable: (path: string): Promise<WorkBuddyState> =>
    invoke("set_workbuddy_executable", { path }),
  saveWorkBuddyGateway: (url: string, apiKey: string | undefined): Promise<WorkBuddyState> =>
    invoke("save_workbuddy_gateway", { request: { url, apiKey: apiKey || null } }),
  saveWorkBuddyOrganization: (
    id: string | undefined,
    name: string,
    modelPrefix: string,
    url: string,
    apiKey: string | undefined
  ): Promise<WorkBuddyState> =>
    invoke("save_workbuddy_organization", {
      request: { id: id || null, name, modelPrefix, url, apiKey: apiKey || null },
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
  launchWorkBuddy: (): Promise<void> => invoke("launch_workbuddy"),
};
