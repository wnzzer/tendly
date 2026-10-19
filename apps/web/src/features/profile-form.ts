import type { AgentProviderKind, EndpointProfile, KeyHeader } from "@tendly/protocol";
import { sharesHostConfig } from "./native-profiles";

/**
 * The endpoint profile form, as plain functions: what a save sends, and the
 * one-line summary each profile shows in the list.
 */

export interface ProfileForm {
  name: string; provider: AgentProviderKind; endpoint_url: string; api: string; model: string; effort: string;
  permission_mode: EndpointProfile["permission_mode"]; secret_ref: string; proxy_url: string;
  /** Where the key goes: the protocol's own header, one of the two any client sends, or a header of the endpoint's. */
  key_mode?: KeyMode; key_name?: string; key_value?: string;
}

export type KeyMode = "" | "x-api-key" | "bearer" | "custom";
export const KEY_HEADERS = {
  "x-api-key": { name: "x-api-key", value: "{key}" },
  bearer: { name: "Authorization", value: "Bearer {key}" },
} as const satisfies Record<string, KeyHeader>;

/** The header the form names, or null for the protocol's own. */
export function keyHeader(form: Pick<ProfileForm, "key_mode" | "key_name" | "key_value">): KeyHeader | null {
  if (form.key_mode === "custom") return { name: (form.key_name ?? "").trim(), value: (form.key_value ?? "").trim() };
  return form.key_mode ? { ...KEY_HEADERS[form.key_mode] } : null;
}

/** A saved header as the form shows it: a preset where it is one, otherwise its name and value. */
export function keyMode(header: KeyHeader | null | undefined): Required<Pick<ProfileForm, "key_mode" | "key_name" | "key_value">> {
  if (!header) return { key_mode: "", key_name: "", key_value: "" };
  for (const mode of ["x-api-key", "bearer"] as const) {
    const preset = KEY_HEADERS[mode];
    if (header.name.toLowerCase() === preset.name.toLowerCase() && header.value === preset.value) return { key_mode: mode, key_name: "", key_value: "" };
  }
  return { key_mode: "custom", key_name: header.name, key_value: header.value };
}

/** Why the form's own header cannot be saved, or "" when it can. */
export function keyHeaderProblem(form: Pick<ProfileForm, "key_mode" | "key_name" | "key_value">): string {
  const header = keyHeader(form);
  if (form.key_mode !== "custom" || !header) return "";
  if (!/^[A-Za-z0-9!#$%&'*+.^_`|~-]{1,64}$/.test(header.name)) return "Use a valid HTTP header name for the key";
  if (header.value.split("{key}").length !== 2 || header.value.includes("$") || header.value.startsWith("!")) return "The header value must contain {key} exactly once, on one line, without $ or a leading !";
  return "";
}

/**
 * What saving an ordinary profile sends. Text is trimmed and an empty field
 * is null. An optional setting -- proxy, aliases, effort, chosen models, API
 * -- is sent when it has a value or the profile already had one, so clearing
 * it clears it while a profile that never had it is not given an empty one.
 * Only a client with a choice of API takes one.
 */
export function profilePayload(form: ProfileForm, previous: EndpointProfile | undefined, aliases: Record<string, string>, models: readonly string[], offersApi: boolean) {
  return {
    name: form.name.trim(), provider: form.provider, endpoint_url: form.endpoint_url.trim() || null, model: form.model.trim() || null,
    permission_mode: form.permission_mode, secret_ref: form.secret_ref.trim() || null,
    ...(form.proxy_url.trim() || previous?.proxy_url !== undefined ? { proxy_url: form.proxy_url.trim() || null } : {}),
    ...(Object.keys(aliases).length || previous?.model_aliases !== undefined ? { model_aliases: aliases } : {}),
    ...(form.effort.trim() || previous?.effort !== undefined ? { effort: form.effort.trim() || null } : {}),
    ...(models.length || previous?.models !== undefined ? { models: [...models] } : {}),
    ...(offersApi && (form.api || previous?.api) ? { api: form.api || null } : {}),
    ...(form.key_mode || previous?.key_header ? { key_header: keyHeader(form) } : {}),
  };
}

/** A profile in one line: its model, the endpoint's host and a proxy if any; or, for a client's own configuration, whose sign-in it uses. */
export function profileSummary(profile: EndpointProfile, t: (text: string) => string): string {
  if (profile.native_config) return sharesHostConfig(profile.native_config) ? t("Host configuration · shared sign-in") : t("Isolated configuration · this account only");
  let host = "";
  try { host = profile.endpoint_url ? new URL(profile.endpoint_url).host : ""; } catch { host = profile.endpoint_url ?? ""; }
  return [profile.model || t("Native client default"), host || t("Official endpoint"), profile.proxy_url ? t("Proxy configured") : ""].filter(Boolean).join(" · ");
}
