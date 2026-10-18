import { ref } from "vue";
import type { AgentProviderKind, ProviderKind } from "@tendly/protocol";
import { json, request } from "./api";
import { AGENT_CLIENTS, isAgentClient } from "./clients";

/**
 * What a new session starts with, per provider. Kept by the server, so every
 * device opens sessions the same way; each value is only a default the
 * new-session dialog and the session's own chips can still change.
 */
export interface ProviderPreference { endpoint_profile_id?: string | null; effort?: string | null; permission?: string | null }
/**
 * `agent_tools`: whether sessions get Tendly's own tools.
 * `resource_monitoring`: whether the host's resources are read at all, for
 * the status bar, the system page and the recorded history. Absent means on
 * for both.
 * `appearance`, `terminal_appearance`: `light` or `dark`; absent follows the
 * system and the interface (theme.ts).
 */
export type Preferences = Record<AgentProviderKind, ProviderPreference> & { hidden_account_quotas?: string[]; agent_tools?: boolean; resource_monitoring?: boolean; appearance?: string; terminal_appearance?: string };
export type PreferenceProvider = AgentProviderKind;

/** The one copy the page reads; loaded once and replaced on save. */
export const preferences = ref<Preferences>(Object.fromEntries(AGENT_CLIENTS.map(id => [id, {}])) as Preferences);
let loading: Promise<Preferences> | undefined;

export function loadPreferences(force = false): Promise<Preferences> {
  if (!loading || force) {
    loading = request<Preferences>("/preferences")
      .then(value => (preferences.value = value))
      // An older server has no preferences; that is the same as none set.
      .catch(() => preferences.value);
  }
  return loading;
}

export async function savePreferences(next: Preferences): Promise<Preferences> {
  const saved = await request<Preferences>("/preferences", json("PUT", next));
  preferences.value = saved; loading = Promise.resolve(saved);
  return saved;
}

export function preferenceFor(provider: ProviderKind, source: Preferences = preferences.value): ProviderPreference | undefined {
  return isAgentClient(provider) ? source[provider] : undefined;
}
