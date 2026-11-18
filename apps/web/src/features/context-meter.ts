/**
 * Share of the model's context window currently in use.
 *
 * The window size is only ever what the native client reported for the model it
 * actually ran. When the client did not state one, there is no share to show:
 * this returns `undefined` rather than dividing by an assumed window, so the
 * meter never turns a token count into a confident-looking percentage.
 */
export interface ContextUsage {
  context_tokens?: number;
  context_window?: number;
}
export interface ContextMeter {
  /** 0–1, clamped. A model may exceed its own reported window. */
  fraction: number;
  percent: number;
  used: number;
  window: number;
  level: "normal" | "high" | "full";
}

const usable = (value?: number) => typeof value === "number" && Number.isFinite(value) && value >= 0;

export function contextMeter(usage?: ContextUsage): ContextMeter | undefined {
  if (!usage || !usable(usage.context_tokens) || !usable(usage.context_window) || !usage.context_window)
    return undefined;
  const used = usage.context_tokens as number;
  const window = usage.context_window as number;
  const fraction = Math.min(1, used / window);
  return {
    fraction,
    // Rounded for display only; a nonzero share never rounds away to 0%.
    percent: used > 0 && fraction < 0.01 ? 1 : Math.round(fraction * 100),
    used,
    window,
    level: fraction >= 0.95 ? "full" : fraction >= 0.8 ? "high" : "normal",
  };
}

/** Compact count for a meter label: 1400 -> "1.4k", 200000 -> "200k". */
export function compactTokens(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "0";
  if (value < 1000) return String(Math.round(value));
  if (value < 1_000_000) {
    const thousands = value / 1000;
    return `${thousands < 10 ? thousands.toFixed(1).replace(/\.0$/, "") : Math.round(thousands)}k`;
  }
  const millions = value / 1_000_000;
  return `${millions < 10 ? millions.toFixed(1).replace(/\.0$/, "") : Math.round(millions)}M`;
}

/** The server's answer for a model's window: one the user set, or the published one. */
export interface KnownWindow {
  context_window: number | null;
  source: "yours" | "published" | null;
}

/**
 * The usage to measure, against the window the user set for this model, else
 * what the client reports, else the published one. A client behind a gateway
 * can report a window its model does not have.
 */
export function measuredUsage<T extends ContextUsage>(
  usage: T | undefined,
  known: KnownWindow | undefined,
): T | undefined {
  if (!usage) return usage;
  const window =
    known?.source === "yours"
      ? (known.context_window ?? undefined)
      : (usage.context_window ?? known?.context_window ?? undefined);
  return { ...usage, context_window: window };
}
