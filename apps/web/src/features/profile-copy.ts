import type { AgentProviderKind, EndpointProfile, EnvironmentOverrides } from "@tendly/protocol";
import { clientInfo, type ClientInfo } from "./clients";
import { SLOTS } from "./claude-slots";

/** The few fields a copy is asked for; everything else comes from the profile it copies. */
export interface CopyDraft {
  name: string;
  provider: AgentProviderKind;
  endpoint_url: string;
  model: string;
  secret_ref: string | null;
}

/**
 * What a profile started from `source` is created with. The proxy, permission
 * intent and environment carry over; settings that only mean something to the
 * source's client (its API choice, Claude Code's model slots) or to its endpoint
 * (the models chosen from its list, aliases for them) are kept only while the
 * copy still runs on that client and endpoint.
 */
export function copiedProfilePayload(source: EndpointProfile, draft: CopyDraft, withEnvironment = true) {
  const client = clientInfo(draft.provider);
  const sameClient = draft.provider === source.provider;
  const endpointUrl = draft.endpoint_url.trim() || null;
  const sameEndpoint = sameClient && endpointUrl === (source.endpoint_url ?? null);
  const model = draft.model.trim() || null;
  const aliases = source.model_aliases ?? {};
  return {
    name: draft.name.trim(),
    provider: draft.provider,
    endpoint_url: endpointUrl,
    model,
    permission_mode: permissionFor(source.permission_mode, client),
    secret_ref: draft.secret_ref,
    ...(source.proxy_url ? { proxy_url: source.proxy_url } : {}),
    // A depth is offered per model, so a different model starts on the default.
    ...(sameClient && model === source.model && source.effort ? { effort: source.effort } : {}),
    ...(sameClient && client.apis?.length && source.api ? { api: source.api } : {}),
    ...(sameEndpoint && source.models?.length ? { models: [...source.models] } : {}),
    ...(sameEndpoint && Object.keys(aliases).length ? { model_aliases: { ...aliases } } : {}),
    ...(withEnvironment ? { environment: copiedEnvironment(source, draft.provider) } : {}),
  };
}

/** A permission intent the new client cannot take falls back to its own defaults. */
function permissionFor(
  mode: EndpointProfile["permission_mode"],
  client: ClientInfo,
): EndpointProfile["permission_mode"] {
  if (mode === "plan") return client.profilePlan ? mode : "native";
  if (mode === "interactive" || mode === "trusted") return client.approvals ? mode : "native";
  return mode;
}

/**
 * The source's environment, for a copy on `provider`. On another client the
 * variables only the source's client reads (model slots, context window) and
 * any the new client reserves are dropped, and a client with model slots
 * starts with them following the main model, as a new profile does. What the
 * person added themselves stays.
 */
export function copiedEnvironment(source: EndpointProfile, provider: AgentProviderKind): EnvironmentOverrides {
  const environment: EnvironmentOverrides = { ...(source.environment ?? {}) };
  if (provider === source.provider) return environment;
  const from = clientInfo(source.provider as AgentProviderKind),
    to = clientInfo(provider);
  const dropped = new Set<string>([
    ...(from.modelSlots ? SLOTS.map(slot => slot.variable) : []),
    ...(from.contextWindow.variable ? [from.contextWindow.variable] : []),
    ...to.reservedEnvironment,
  ]);
  for (const name of Object.keys(environment)) if (dropped.has(name)) delete environment[name];
  if (to.modelSlots) for (const slot of SLOTS) environment[slot.variable] ??= { kind: "main_model" };
  return environment;
}

/** The first of `label(1)`, `label(2)`… ("Name copy", "Name copy 2") not already taken. */
export function copyName(taken: Iterable<string>, label: (n: number) => string): string {
  const names = new Set(taken);
  for (let n = 1; ; n++) {
    const candidate = label(n);
    if (!names.has(candidate)) return candidate;
  }
}
