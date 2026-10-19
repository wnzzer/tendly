// Turns LiteLLM's community price list into data/prices.json: the models
// Claude Code, Codex and Pi call, at their makers' own prices, in dollars per
// million tokens. Pure functions only, so the transform and the checks are
// tested without the network.

export const SCHEMA_VERSION = 1;

const PER_MILLION = 1_000_000;
/** No published API price is anywhere near this; a larger one is a unit mistake. */
const MOST_PER_MILLION = 5_000;

/** A dated snapshot repeats its undated name: gpt-5-2025-08-07, claude-haiku-4-5-20251001. */
const DATED = /-(\d{4}-\d{2}-\d{2}|\d{8})$/;

/**
 * Each maker's own API and the model names it sells. A reseller's copy of a
 * model (dashscope's DeepSeek, azure_ai's GLM) has the reseller's price, so
 * only the maker's is kept; dashscope is Qwen's maker.
 */
const MAKERS = {
  anthropic: name => name.startsWith("claude-"),
  openai: name => /^(gpt-|o\d|codex-)/.test(name) && !name.includes("ft:"),
  gemini: name => name.startsWith("gemini-"),
  deepseek: name => name.startsWith("deepseek-"),
  zai: name => name.startsWith("glm-"),
  moonshot: name => name.startsWith("kimi-"),
  dashscope: name => name.startsWith("qwen"),
  xai: name => name.startsWith("grok-"),
  minimax: name => name.startsWith("minimax-"),
};

/**
 * The name a client logs for a wanted entry, or undefined. LiteLLM files some
 * makers' models under their own prefix (`zai/glm-5.3`); clients log the
 * bare name.
 */
function wanted(key, entry) {
  if (!entry || typeof entry !== "object") return undefined;
  if (!["chat", "responses"].includes(entry.mode)) return undefined;
  const maker = entry.litellm_provider;
  const keep = Object.hasOwn(MAKERS, maker) ? MAKERS[maker] : undefined;
  if (!keep) return undefined;
  const lower = key.toLowerCase();
  const name = lower.startsWith(`${maker}/`) ? lower.slice(maker.length + 1) : lower;
  return !name.includes("/") && keep(name) ? name : undefined;
}

const perMillion = value => (typeof value === "number" && Number.isFinite(value) ? +(value * PER_MILLION).toFixed(6) : undefined);
const tokens = value => (Number.isInteger(value) && value > 0 ? value : undefined);
/** No model takes more than this; a larger figure is a unit mistake. */
const MOST_TOKENS = 100_000_000;

/**
 * The prices Tendly uses from one LiteLLM entry. Only the base tier: the
 * long-context, batch, flex and priority tiers are not told apart in the
 * clients' logs. A cache write is priced only where the provider charges for
 * one; absent, Tendly bills it at the input price.
 */
export function convert(entry) {
  const price = {
    input: perMillion(entry.input_cost_per_token),
    output: perMillion(entry.output_cost_per_token),
    cacheRead: perMillion(entry.cache_read_input_token_cost) ?? perMillion(entry.input_cost_per_token),
  };
  if (price.input === undefined || price.output === undefined) return undefined;
  // How much a model takes in, and writes out, at most: a fallback for
  // showing how full a context is, never pushed to a client on its own.
  const context = tokens(entry.max_input_tokens) ?? tokens(entry.max_tokens);
  const output = tokens(entry.max_output_tokens);
  if (context !== undefined) price.contextWindow = context;
  if (output !== undefined) price.maxOutput = output;
  const write = perMillion(entry.cache_creation_input_token_cost);
  const writeHour = perMillion(entry.cache_creation_input_token_cost_above_1hr);
  if (write !== undefined) price.cacheWrite = write;
  if (writeHour !== undefined) price.cacheWrite1h = writeHour;
  return price;
}

/**
 * Every wanted model from LiteLLM, without dated copies of a name already
 * present, then the overrides on top. Where a model is listed both bare and
 * under its maker's prefix, the bare entry wins.
 */
export function build(litellm, overrides = {}) {
  const models = {};
  const bare = new Set();
  for (const [key, entry] of Object.entries(litellm)) {
    const name = wanted(key, entry);
    if (!name || bare.has(name)) continue;
    const price = convert(entry);
    if (!price) continue;
    models[name] = price;
    if (!key.includes("/")) bare.add(name);
  }
  for (const name of Object.keys(models)) {
    const base = name.replace(DATED, "");
    if (base !== name && models[base]) delete models[name];
  }
  for (const [name, price] of Object.entries(overrides.models ?? {})) models[name.toLowerCase()] = price;
  return Object.fromEntries(Object.entries(models).sort(([a], [b]) => a.localeCompare(b)));
}

/**
 * Problems that keep a list from being published; empty when it may be.
 * `previous` is the list it would replace, so a source that suddenly lost
 * most of its models is caught rather than shipped.
 */
export function problems(models, { required = [], previous } = {}) {
  const found = [];
  const names = Object.keys(models);
  for (const name of required) if (!models[name]) found.push(`required model missing: ${name}`);
  for (const [name, price] of Object.entries(models)) {
    for (const field of ["contextWindow", "maxOutput"]) {
      const value = price[field];
      if (value !== undefined && !(Number.isInteger(value) && value > 0 && value <= MOST_TOKENS)) found.push(`${name}.${field} is not a sane token count: ${value}`);
    }
    for (const field of ["input", "output", "cacheRead", "cacheWrite", "cacheWrite1h"]) {
      const value = price[field];
      if (value === undefined && ["cacheWrite", "cacheWrite1h"].includes(field)) continue;
      if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > MOST_PER_MILLION) found.push(`${name}.${field} is not a sane price: ${value}`);
    }
  }
  const before = previous ? Object.keys(previous).length : 0;
  if (before && names.length < before / 2) found.push(`only ${names.length} models, down from ${before}`);
  return found;
}

/** The file to write, or undefined when the models are unchanged and the file should be left alone. */
export function nextFile(models, current, now) {
  if (current?.schemaVersion === SCHEMA_VERSION && JSON.stringify(current.models) === JSON.stringify(models)) return undefined;
  return { schemaVersion: SCHEMA_VERSION, updatedAt: now.toISOString().replace(/\.\d{3}Z$/, "Z"), source: "LiteLLM model_prices_and_context_window.json, with data/prices-overrides.json", models };
}
