/**
 * Translating what a client says about its own models.
 *
 * These lines are written by Claude Code and Codex rather than by Tendly:
 * they name models, quote prices, and change with the client's version, so
 * there is no fixed set of them to put in a dictionary. What is fixed are the
 * phrases between the separators — "Efficient for routine tasks" is the same
 * sentence whichever model it describes — so each segment is translated on its
 * own and anything unrecognised passes through exactly as the client wrote it.
 *
 * A model's name is never touched. It is the thing's name.
 */
type Translate = (source: string, values?: Record<string, string>) => string;

/**
 * Segments that carry a value the client filled in. The value is lifted out so
 * the sentence around it can be translated, and put back untranslated.
 */
const FILLED: { pattern: RegExp; template: string; name: string }[] = [
  {
    pattern: /^Use the default model \(currently (.+)\)$/,
    template: "Use the default model (currently {model})",
    name: "model",
  },
  { pattern: /^(\$[\d.]+\/\$[\d.]+) per Mtok$/, template: "{price} per Mtok", name: "price" },
];

export function describeModel(description: string, t: Translate): string {
  return description
    .split("·")
    .map(segment => segment.trim())
    .map(segment => translateSegment(segment, t))
    .join(" · ");
}

function translateSegment(segment: string, t: Translate): string {
  if (!segment) return segment;
  for (const { pattern, template, name } of FILLED) {
    const match = pattern.exec(segment);
    if (match) return t(template, { [name]: match[1] });
  }
  return t(segment);
}

/**
 * The model a "default" entry stands for, when its description names it.
 *
 * Claude Code lists its default as "Default (recommended)" and says which model
 * that is only in the description ("Opus 5.5 · …", or "Use the default model
 * (currently Opus 5.5)" in older versions). A chip reading "Default" does not
 * say what is answering, so the name is lifted out — only when it is the
 * name of another model in the same list, or the client said "currently".
 */
export function resolvedDefaultName(
  entry: { id: string; description?: string },
  models: readonly { id: string; name: string }[],
): string | undefined {
  if (entry.id !== "default" || !entry.description) return undefined;
  for (const segment of entry.description.split("·").map(part => part.trim())) {
    const current = /^Use the default model \(currently (.+)\)$/.exec(segment)?.[1];
    if (current) return current;
    if (models.some(model => model.id !== entry.id && model.name === segment)) return segment;
  }
  return undefined;
}
