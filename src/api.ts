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
  integrated: boolean;
  cert_imported: boolean;
  cert_count: number;
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
  kind: "skills" | "plugins" | "agents" | "commands" | "mcp";
  label: string;
  path: string;
  items: string[];
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

/** HealthItem（serde rename_all = camelCase） */
export interface HealthItem {
  id: string;
  label: string;
  status: "ok" | "warn" | "fail";
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
  profileRuntimeInfo: (): Promise<ProfileRuntimeInfo[]> => invoke("profile_runtime_info"),
  extensionOverview: (): Promise<ExtensionGroup[]> => invoke("extension_overview"),
  backupConfig: (): Promise<string> => invoke("backup_config"),
  recentSyncLog: (): Promise<string[]> => invoke("recent_sync_log"),
  importCert: (path: string): Promise<string> => invoke("import_cert", { path }),
  clearCerts: (): Promise<string> => invoke("clear_certs"),
  detectModels: (baseUrl: string, token: string): Promise<string[]> =>
    invoke("detect_models", { baseUrl, token }),
  detectModelsFor: (name: string): Promise<string[]> =>
    invoke("detect_models_for", { name }),
  usageStats: (): Promise<UsageStats> => invoke("usage_stats"),
  // 健康与诊断
  modelPinWarnings: (): Promise<ModelPinWarning[]> =>
    invoke("model_pin_warnings"),
  fixModelPin: (profile: string): Promise<string> =>
    invoke("fix_model_pin", { profile }),
  healthCheck: (): Promise<HealthItem[]> => invoke("health_check"),
  exportDiagnostics: (): Promise<string> => invoke("export_diagnostics"),
};
