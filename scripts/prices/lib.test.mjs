import { test } from "node:test";
import assert from "node:assert/strict";
import { build, convert, nextFile, problems, SCHEMA_VERSION } from "./lib.mjs";

const claude = { litellm_provider: "anthropic", mode: "chat", max_input_tokens: 1000000, max_output_tokens: 128000, input_cost_per_token: 4e-6, output_cost_per_token: 2e-5, cache_read_input_token_cost: 2e-7, cache_creation_input_token_cost: 5e-6, cache_creation_input_token_cost_above_1hr: 8e-6, input_cost_per_token_above_200k_tokens: 8e-6 };
const gpt = { litellm_provider: "openai", mode: "responses", max_tokens: 400000, input_cost_per_token: 5e-6, output_cost_per_token: 3e-5, cache_read_input_token_cost: 5e-7, input_cost_per_token_flex: 2.5e-6 };

test("an entry becomes dollars per million at its base tier, with cache writes only where charged", () => {
  assert.deepEqual(convert(claude), { input: 4, output: 20, cacheRead: 0.2, contextWindow: 1000000, maxOutput: 128000, cacheWrite: 5, cacheWrite1h: 8 });
  // Only max_tokens: it stands for the window.
  assert.deepEqual(convert(gpt), { input: 5, output: 30, cacheRead: 0.5, contextWindow: 400000 });
  // No cache price: a cached token costs what an input token does.
  assert.deepEqual(convert({ input_cost_per_token: 1.5e-4, output_cost_per_token: 6e-4 }), { input: 150, output: 600, cacheRead: 150 });
  assert.equal(convert({ output_cost_per_token: 1e-5 }), undefined, "no input price, no entry");
});

test("only the clients' models are kept, without dated copies, and overrides win", () => {
  const models = build({
    "claude-opus-5-5": claude,
    "claude-haiku-4-5": claude,
    "claude-haiku-4-5-20251001": claude,
    "gpt-5.5": gpt,
    "gpt-5.5-2026-04-23": gpt,
    // A dated name with no undated twin stays.
    "gpt-4o-2024-05-13": gpt,
    "o3": gpt,
    "ft:gpt-4o-mini": gpt,
    "gpt-image-1": { ...gpt, mode: "image_generation" },
    "text-embedding-3-small": { ...gpt, mode: "embedding" },
    "anthropic.claude-opus-5-5": { ...claude, litellm_provider: "bedrock" },
    "sample_spec": { mode: "chat" },
  }, { models: { "gpt-5.5": { input: 1, output: 2, cacheRead: 0.1 } } });
  assert.deepEqual(Object.keys(models), ["claude-haiku-4-5", "claude-opus-5-5", "gpt-4o-2024-05-13", "gpt-5.5", "o3"]);
  assert.deepEqual(models["gpt-5.5"], { input: 1, output: 2, cacheRead: 0.1 });
  assert.equal(models["o3"].contextWindow, 400000);
});

test("other makers' models are kept by their bare names, at the maker's own price", () => {
  const priced = (provider, input) => ({ litellm_provider: provider, mode: "chat", input_cost_per_token: input, output_cost_per_token: 1e-6 });
  const models = build({
    "deepseek-v4-flash": priced("deepseek", 3e-7),
    "deepseek/deepseek-v4-flash": priced("deepseek", 9e-7),
    "zai/glm-5.3-flash": priced("zai", 1.5e-7),
    "gemini/gemini-3.8-flash": priced("gemini", 7.5e-7),
    "moonshot/kimi-k3": priced("moonshot", 3e-6),
    "dashscope/qwen3.8-max": priced("dashscope", 2e-6),
    // A reseller's copy, and a maker's model filed deeper than its prefix.
    "dashscope/deepseek-v4-flash": priced("dashscope", 2e-7),
    "azure_ai/glm-5.1": priced("azure_ai", 1e-6),
    "zai/org/glm-5": priced("zai", 1e-6),
  });
  assert.deepEqual(Object.keys(models), ["deepseek-v4-flash", "gemini-3.8-flash", "glm-5.3-flash", "kimi-k3", "qwen3.8-max"]);
  assert.equal(models["deepseek-v4-flash"].input, 0.3, "the bare entry wins over the prefixed one");
});

test("a list that lost a required model, a sane price or most of its models is not published", () => {
  const good = { "gpt-5.5": { input: 5, output: 30, cacheRead: 0.5 } };
  assert.deepEqual(problems(good, { required: ["gpt-5.5"] }), []);
  assert.match(problems(good, { required: ["claude-opus-5-5"] })[0], /required model missing: claude-opus-5-5/);
  assert.match(problems({ x: { input: 5e-6, output: 30e6, cacheRead: 0 } })[0], /x.output is not a sane price/);
  assert.match(problems({ x: { input: -1, output: 1, cacheRead: 0 } })[0], /x.input/);
  assert.match(problems({ x: { input: 1, output: 1, cacheRead: 0, cacheWrite: Number.NaN } })[0], /x.cacheWrite/);
  assert.match(problems({ x: { input: 1, output: 1, cacheRead: 0, contextWindow: 1.5 } })[0], /x.contextWindow is not a sane token count/);
  assert.match(problems({ x: { input: 1, output: 1, cacheRead: 0, maxOutput: 2e9 } })[0], /x.maxOutput/);
  const previous = Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`m${i}`, good["gpt-5.5"]]));
  assert.match(problems(good, { previous })[0], /only 1 models, down from 10/);
});

test("the file is rewritten only when a price changed", () => {
  const models = { "gpt-5.5": { input: 5, output: 30, cacheRead: 0.5 } };
  const now = new Date("2026-10-08T03:17:09.123Z");
  const file = nextFile(models, undefined, now);
  assert.equal(file.schemaVersion, SCHEMA_VERSION);
  assert.equal(file.updatedAt, "2026-10-08T03:17:09Z");
  assert.equal(nextFile(models, file, new Date()), undefined, "same prices, same file and date");
  assert.ok(nextFile({ "gpt-5.5": { ...models["gpt-5.5"], output: 31 } }, file, new Date()));
});
