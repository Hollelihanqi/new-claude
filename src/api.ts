import { invoke } from "@tauri-apps/api/core";

// 与 src-tauri 里的 #[tauri::command] 及 serde 结构一一对应

/** Profile（serde rename_all = camelCase） */
export interface Profile {
  name: string;
  type: "router" | "account";
  baseUrl: string;
  tokenEnc?: string;
  hasToken: boolean;
  shareSkills: boolean;
  sharePlugins: boolean;
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
  syncLinks: (name: string, skills: boolean, plugins: boolean): Promise<string> =>
    invoke("sync_links", { name, skills, plugins }),
  environment: (): Promise<EnvInfo> => invoke("environment"),
  importCert: (path: string): Promise<string> => invoke("import_cert", { path }),
  clearCerts: (): Promise<string> => invoke("clear_certs"),
  detectModels: (baseUrl: string, token: string): Promise<string[]> =>
    invoke("detect_models", { baseUrl, token }),
  detectModelsFor: (name: string): Promise<string[]> =>
    invoke("detect_models_for", { name }),
  usageStats: (): Promise<UsageStats> => invoke("usage_stats"),
};
