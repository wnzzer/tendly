import { reactive } from "vue";

export interface BackendHealth { ok: boolean; platform?: string; instance_label?: string; version?: string; api_version?: number; capabilities?: string[] }
export function capabilitiesFor(health: BackendHealth) {
  const modern = typeof health.api_version === "number" && health.api_version >= 2;
  const has = (name: string, legacy = modern) => Array.isArray(health.capabilities) ? health.capabilities.includes(name) : legacy;
  return { sharedCanvas: has("shared_canvas", false), nativeConfig: has("native_configurations"), nativeHistory: has("native_history"), directories: has("host_directories"), models: has("endpoint_models"), environment: has("session_environment", false), structuredChat: has("structured_chat",false), accounts: has("official_accounts",false), sessionConfiguration: has("session_configuration",false), sessionArchive: has("session_archive", false), accountImportNative: has("account_import_native", false),
    clients: has("agent_clients", false), ephemeralSessions: has("ephemeral_sessions", false), sessionTerminalEscape: has("session_terminal_escape", false),
    customBalances: has("custom_balances", false), storedSecrets: has("stored_secrets", false), agentTools: has("agent_tools", false), selfUpdate: has("self_update", false),
    modelProbe: has("model_probe", false), modelProbePlan: has("model_probe_plan", false),
    resourceMonitoring: has("resource_monitoring", false), tailnet: has("tailnet", false), peers: has("peers", false), hostRoots: has("host_roots", false) };
}
export const backendCapabilities = reactive(capabilitiesFor({ ok: false }));
export function setBackendCapabilities(health: BackendHealth) { Object.assign(backendCapabilities, capabilitiesFor(health)); }
