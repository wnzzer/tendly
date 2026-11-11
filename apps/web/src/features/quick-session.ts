import type { EndpointProfile, ProviderKind } from "@tendly/protocol";
import { DEFAULT_CLIENT, PROVIDER_KINDS, providerLabel } from "./clients";
import { PROFILE_CHOICE_REQUIRED, PROFILE_NONE, preferredProfileSelection } from "./profile-preferences";

export type QuickProvider = ProviderKind;
const KEY = "tendly.quick-provider.v1";
type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;

function browserStorage(): PreferenceStorage | undefined {
  try { return typeof window === "undefined" ? undefined : window.localStorage; } catch { return undefined; }
}
const DEFAULT: QuickProvider = DEFAULT_CLIENT;
/** Only covers a storage that is unavailable or throwing; a readable storage is always authoritative. */
const unstored = { value: undefined as QuickProvider | undefined };
const valid = (value: unknown): QuickProvider | undefined => PROVIDER_KINDS.find(provider => provider === value);

/** The provider the plain "+" will create, so its tooltip can say so up front. */
export function lastQuickProvider(storage?: PreferenceStorage): QuickProvider {
  const target = storage ?? browserStorage();
  if (!target) return unstored.value ?? DEFAULT;
  try { return valid(target.getItem(KEY)) ?? DEFAULT; }
  catch { return unstored.value ?? DEFAULT; }
}
export function rememberQuickProvider(provider: QuickProvider, storage?: PreferenceStorage): void {
  if (!valid(provider)) return;
  const target = storage ?? browserStorage();
  try {
    if (!target) { unstored.value = provider; return; }
    target.setItem(KEY, provider);
  } catch { unstored.value = provider; }
}

export interface QuickPlan {
  /** Body for POST /api/workspaces/:id/sessions, or undefined when a choice is genuinely required. */
  body?: Record<string, unknown>;
  /** True when the full dialog must be opened instead, because an account cannot be guessed. */
  needsDialog: boolean;
}
export interface QuickOptions {
  ephemeral?: boolean;
  structuredChat?: boolean;
  ephemeralSupported?: boolean;
  /** Existing titles in the workspace, so a repeated quick create does not produce duplicates. */
  existingTitles?: readonly string[];
  /** The profile Preferences names for this provider, if any. */
  preferredProfileId?: string | null;
}

/**
 * Build a create request without asking anything. An account is only assumed
 * when the choice is unambiguous: with several host accounts available we would
 * be picking one silently, and with none there is nothing to start with, so the
 * dialog opens instead.
 */
export function planQuickSession(provider: QuickProvider, profiles: EndpointProfile[], options: QuickOptions = {}): QuickPlan {
  const profileId = provider === "terminal" ? "" : preferredProfileSelection(provider, profiles, undefined, options.preferredProfileId);
  if (profileId === PROFILE_CHOICE_REQUIRED || profileId === PROFILE_NONE) return { needsDialog: true };
  return {
    needsDialog: false,
    body: {
      title: quickTitle(provider, options.ephemeral === true, options.existingTitles ?? []),
      provider,
      endpoint_profile_id: profileId || null,
      ...(options.structuredChat && provider !== "terminal" ? { interaction_mode: "structured" } : {}),
      ...(options.ephemeralSupported && options.ephemeral ? { ephemeral: true } : {}),
    },
  };
}

/** A numbered name keeps a row of quick sessions distinguishable in the sidebar. */
export function quickTitle(provider: QuickProvider, ephemeral: boolean, existing: readonly string[]): string {
  const base = ephemeral ? `${providerLabel(provider)} · scratch` : providerLabel(provider);
  if (!existing.includes(base)) return base;
  for (let suffix = 2; suffix < 1000; suffix++) {
    const candidate = `${base} ${suffix}`;
    if (!existing.includes(candidate)) return candidate;
  }
  return base;
}
