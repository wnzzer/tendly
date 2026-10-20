/**
 * Context windows the user set, by model (server model_limits.rs), and the
 * drafts a form holds before saving them. Keys are model IDs lowercased, as
 * the server keeps them.
 */
export type Limits = Record<string, number>;

export const FEWEST_TOKENS = 1_000, MOST_TOKENS = 100_000_000;

/** A window known for a model the user set none for: what the endpoint reported, or the published one. Shown, never applied. */
export interface KnownWindow { tokens: number; source: "endpoint" | "published" }

export function limitKey(model: string): string { return model.trim().toLowerCase(); }

/** A typed window as a number: separators allowed, empty for none. */
export function windowValue(text: string): number | undefined {
  const plain = text.replace(/[,_\s]/g, "");
  return plain ? Number(plain) : undefined;
}

export function validWindow(text: string): boolean {
  const value = windowValue(text);
  return value === undefined || Number.isInteger(value) && value >= FEWEST_TOKENS && value <= MOST_TOKENS;
}

/** The saved limits with the drafts applied, or undefined when nothing changes. */
export function nextLimits(saved: Limits, drafts: Record<string, string>): Limits | undefined {
  const next = { ...saved };
  for (const [model, text] of Object.entries(drafts)) {
    const value = windowValue(text);
    if (value === undefined) delete next[model]; else next[model] = value;
  }
  const keys = new Set([...Object.keys(saved), ...Object.keys(next)]);
  return [...keys].some(key => saved[key] !== next[key]) ? next : undefined;
}
