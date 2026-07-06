import { invoke } from "@tauri-apps/api/core";

// 与 src-tauri 里的 #[tauri::command] 一一对应
export const api = {
  listProfiles: () => invoke("list_profiles"),
  saveProfile: (profile, token) =>
    invoke("save_profile", { profile, token: token || null }),
  deleteProfile: (name) => invoke("delete_profile", { name }),
  syncLinks: (name, skills, plugins) =>
    invoke("sync_links", { name, skills, plugins }),
  environment: () => invoke("environment"),
  importCert: (path) => invoke("import_cert", { path }),
  clearCerts: () => invoke("clear_certs"),
  detectModels: (baseUrl, token) =>
    invoke("detect_models", { baseUrl, token }),
  detectModelsFor: (name) => invoke("detect_models_for", { name }),
  usageStats: () => invoke("usage_stats"),
};
