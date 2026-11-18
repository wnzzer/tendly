/**
 * Names for keys Tendly stores itself (server `secrets.rs`).
 *
 * A profile refers to its key as `env:TENDLY_SECRET_NAME`, whether the
 * server's environment or Tendly's own store holds it. A key pasted into
 * the profile form is stored under a name made from the profile's name, and
 * never under one another profile already uses: that would silently change
 * the key behind someone else's endpoint.
 */
/** `stored`: Tendly keeps a value under the name (absent from an older server, where `source` says it). */
export interface SecretName {
  name: string;
  source: "tendly" | "environment";
  stored?: boolean;
}

/** Whether Tendly itself keeps the key `name`, whichever source wins now. */
export function isStored(entries: readonly SecretName[], name: string): boolean {
  return entries.some(entry => entry.name === name && (entry.stored ?? entry.source === "tendly"));
}

export const SECRET_PREFIX = "TENDLY_SECRET_";
const REFERENCE = /^env:(TENDLY_SECRET_[A-Z0-9_]+)$/;

/** The secret a reference names, if it is a well-formed one. */
export function referencedSecret(reference: string | null | undefined): string | undefined {
  return reference?.trim().match(REFERENCE)?.[1];
}

/** `Work proxy` → `TENDLY_SECRET_WORK_PROXY`; `_2`, `_3`… when taken. */
export function secretNameFor(label: string, taken: Iterable<string>): string {
  const slug =
    label
      .toUpperCase()
      .replace(/[^A-Z0-9]+/g, "_")
      .replace(/^_+|_+$/g, "") || "KEY";
  const used = new Set(taken),
    base = SECRET_PREFIX + slug;
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) if (!used.has(`${base}_${n}`)) return `${base}_${n}`;
}

/**
 * The stored key a profile can reuse when a new one is pasted: its own, when
 * it already points at one Tendly holds and no other profile shares it.
 */
export function ownStoredSecret(
  reference: string | null | undefined,
  stored: SecretName[],
  otherReferences: Array<string | null | undefined>,
): string | undefined {
  const name = referencedSecret(reference);
  if (!name || !isStored(stored, name)) return undefined;
  return otherReferences.some(other => referencedSecret(other) === name) ? undefined : name;
}

/**
 * Where a profile's key stands. `stored`: Tendly keeps the key the profile
 * references. `adoptable`: the server sets that variable but Tendly has no
 * copy, so it can be kept here. `shadowed`: Tendly keeps it, but the server
 * also sets a variable of that name, which wins while it does.
 */
export function secretStatus(
  reference: string | null | undefined,
  entries: readonly SecretName[],
): { stored?: string; adoptable?: string; shadowed: boolean } {
  const name = referencedSecret(reference);
  if (!name) return { shadowed: false };
  const fromEnvironment = entries.some(entry => entry.name === name && entry.source === "environment");
  if (isStored(entries, name)) return { stored: name, shadowed: fromEnvironment };
  return fromEnvironment ? { adoptable: name, shadowed: false } : { shadowed: false };
}
