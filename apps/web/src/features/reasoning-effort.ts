import type { AgentProviderKind, ModelCatalog } from "@tendly/protocol";
import { agentClient } from "./clients";

/** The values accepted by both native clients' configuration surfaces. */
export const REASONING_EFFORTS = ["low", "medium", "high", "xhigh", "max"] as const;

/**
 * Claude Code exposes --effort as a client option, so it gets every level
 * (`effortLadder` in clients.ts). Codex is model-specific and must use the
 * levels advertised by its model/list response; an empty catalog
 * deliberately produces no guessed choices. Custom endpoints may omit capability
 * metadata, so allow a manual choice using the client vocabulary.
 */
export function modelEfforts(
  provider: AgentProviderKind,
  model: string | undefined,
  catalog: ModelCatalog | undefined,
  current?: string | null,
  customEndpoint = false,
): string[] {
  const advertised = model ? (catalog?.models.find(entry => entry.id === model)?.efforts ?? []) : [];
  const values =
    (customEndpoint || agentClient(provider)?.effortLadder) && !advertised.length
      ? provider === "pi"
        ? ["low", "medium", "high"]
        : [...REASONING_EFFORTS]
      : advertised.filter(value => (REASONING_EFFORTS as readonly string[]).includes(value));
  if (current && (REASONING_EFFORTS as readonly string[]).includes(current) && !values.includes(current))
    values.push(current);
  return [...new Set(values)];
}

export function effortLabel(value: string): string {
  return value === "xhigh" ? "X-high" : value[0].toUpperCase() + value.slice(1);
}

/**
 * The thinking-depth slider. Stop 0 leaves the depth to the client; the rest
 * are the levels it offers, so dragging never invents one it did not.
 */
export const effortStops = (choices: readonly string[]) => ["", ...choices];

/** Where the slider rests for the level in use; one it does not offer rests at 0. */
export const restingStop = (stops: readonly string[], live: string) => Math.max(0, stops.indexOf(live));

/** How much of the track, as a fraction, is coloured left of the thumb. */
export const effortFill = (stops: readonly string[], index: number) =>
  stops.length < 2 ? 0 : index / (stops.length - 1);

/** The top third of whatever the client offers, so the flourish marks the
 * genuinely expensive end rather than a level name hardcoded here. */
export const effortVivid = (stops: readonly string[], index: number, stop: string) =>
  !!stop && stops.length > 2 && index >= stops.length - 2;
