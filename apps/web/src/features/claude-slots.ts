import { newEnvironmentRow, type EnvironmentRow } from "./environment-model";

/**
 * Claude Code's model slots, kept as the environment variables Claude Code
 * reads for them, so what a slot runs is always shown in the profile's
 * environment rather than kept somewhere of its own. A slot is either left
 * to the client (no variable: Anthropic's model for it), following the main
 * model (`main_model`, resolved at launch), or a model of its own (a value).
 */
export const SLOTS = [
  { key: "opus", label: "Opus", variable: "ANTHROPIC_DEFAULT_OPUS_MODEL" },
  { key: "sonnet", label: "Sonnet", variable: "ANTHROPIC_DEFAULT_SONNET_MODEL" },
  { key: "haiku", label: "Haiku", variable: "ANTHROPIC_DEFAULT_HAIKU_MODEL" },
] as const;
export type SlotVariable = (typeof SLOTS)[number]["variable"];
export type SlotState = { mode: "main" } | { mode: "client" } | { mode: "model"; model: string };

export function slotState(rows: readonly EnvironmentRow[], variable: SlotVariable): SlotState {
  const row = rows.find(entry => entry.name.trim() === variable);
  if (!row || row.kind === "unset") return { mode: "client" };
  if (row.kind === "main_model") return { mode: "main" };
  return { mode: "model", model: row.kind === "literal" ? row.value : "" };
}

/** The rows with one slot set to `state`: its variable replaced, added or removed. */
export function setSlot(rows: readonly EnvironmentRow[], variable: SlotVariable, state: SlotState): EnvironmentRow[] {
  const index = rows.findIndex(entry => entry.name.trim() === variable);
  const others = rows.filter(entry => entry.name.trim() !== variable);
  if (state.mode === "client") return others;
  const row: EnvironmentRow = {
    ...(index >= 0 ? rows[index] : newEnvironmentRow()),
    name: variable,
    kind: state.mode === "main" ? "main_model" : "literal",
    value: state.mode === "model" ? state.model : "",
  };
  // Kept where it was, so editing a slot does not reorder the list.
  return index >= 0
    ? rows
        .map((entry, at) => (at === index ? row : entry))
        .filter(entry => entry === row || entry.name.trim() !== variable)
    : [...others, row];
}

/** Every slot following the main model: what a new Claude Code endpoint starts with. */
export function followingRows(rows: readonly EnvironmentRow[] = []): EnvironmentRow[] {
  return SLOTS.reduce<EnvironmentRow[]>((next, slot) => setSlot(next, slot.variable, { mode: "main" }), [...rows]);
}

/** One word for all three slots, or `mixed` when they differ. */
export function slotSummary(rows: readonly EnvironmentRow[]): "main" | "client" | "mixed" {
  const modes = new Set(SLOTS.map(slot => slotState(rows, slot.variable).mode));
  return modes.size === 1 && !modes.has("model") ? ([...modes][0] as "main" | "client") : "mixed";
}
