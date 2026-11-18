import type { EndpointProfile, ProviderKind } from "@tendly/protocol";
import { parseEnvironmentRows, type EnvironmentRow } from "./environment-model";

export function profileEnvironmentDraftChanged(rows: EnvironmentRow[], initial: EnvironmentRow[]): boolean {
  const signature = (values: EnvironmentRow[]) =>
    JSON.stringify(values.map(({ name, kind, value }) => ({ name, kind, value })));
  return signature(rows) !== signature(initial);
}

/** Unsupported backends may receive a rename, never a silently discarded env edit. */
export function profileEnvironmentPayload(rows: EnvironmentRow[], supported: boolean, initial: EnvironmentRow[] = []) {
  if (!supported) {
    if (profileEnvironmentDraftChanged(rows, initial))
      throw new Error("Environment changes are not saved by this backend. Upgrade it or keep your draft.");
    return {};
  }
  const parsed = parseEnvironmentRows(rows);
  if (parsed.errors.length) throw new Error(parsed.errors[0]);
  // An empty object is intentional: clearing a template replaces it with {}.
  return { environment: parsed.environment };
}

export function nativeProfileUpdatePayload(
  name: string,
  rows: EnvironmentRow[],
  supported: boolean,
  initial: EnvironmentRow[] = [],
) {
  return { ...nativeProfileRenamePayload(name), ...profileEnvironmentPayload(rows, supported, initial) };
}

/**
 * Whether a native configuration directory is one Tendly created and owns.
 *
 * A profile having a `native_config` only says it runs against a client's own
 * configuration directory — not whose directory that is. An account created
 * here gets its own under `.tendly/accounts/<id>` and is the only thing
 * signing in through it; an imported one points at a directory the host already
 * had, where a profile switcher or a native re-login changes this account too.
 *
 * `source_id` separates them, by the same `account:` convention the server uses
 * to refuse unlinking a managed profile. Reading `native_config` itself as
 * "shared" is what labelled a hand-made account as a shared host sign-in.
 */
export function ownsNativeConfig(native?: { source_id: string } | null): boolean {
  return !!native && native.source_id.startsWith("account:");
}
/** True only for a directory the host already had, which other tools also edit. */
export function sharesHostConfig(native?: { source_id: string } | null): boolean {
  return !!native && !ownsNativeConfig(native);
}
/** Renaming a shared reference must not send even null endpoint/configuration overrides. */
export function nativeProfileRenamePayload(name: string): { name: string } {
  return { name: name.trim() };
}

/** A hidden/stale model input must never override settings in a shared native directory. */
export function sessionModelOverride(
  provider: ProviderKind,
  profile: EndpointProfile | undefined,
  model: string,
): { model?: string } {
  const selected = model.trim();
  return provider !== "terminal" && !profile?.native_config && selected ? { model: selected } : {};
}

export function sessionEffortOverride(
  provider: ProviderKind,
  profile: EndpointProfile | undefined,
  effort: string,
): { effort?: string } {
  const selected = effort.trim();
  return provider !== "terminal" && !profile?.native_config && selected ? { effort: selected } : {};
}
