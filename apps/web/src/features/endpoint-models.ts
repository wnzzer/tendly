/** A one-step display alias -> actual provider model ID map. */
export function parseModelAliases(text: string): Record<string, string> {
  const result: Record<string, string> = Object.create(null);
  for (const line of text.split(/\r?\n/).filter(line => line.trim())) {
    const split = line.indexOf("=");
    if (split < 1) throw new Error("Use one alias = model-id mapping per line.");
    const alias = line.slice(0, split).trim(),
      model = line.slice(split + 1).trim();
    if (!alias || !model || alias.length > 120 || model.length > 200)
      throw new Error("Alias names and model IDs must be non-empty and short.");
    if (Object.hasOwn(result, alias)) throw new Error("Alias names must be unique.");
    result[alias] = model;
  }
  if (Object.keys(result).length > 100) throw new Error("At most 100 model aliases are allowed.");
  return result;
}
export const formatModelAliases = (aliases?: Record<string, string>) =>
  Object.entries(aliases ?? {})
    .map(([key, value]) => `${key} = ${value}`)
    .join("\n");
