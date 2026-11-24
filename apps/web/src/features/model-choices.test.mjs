import { test } from "node:test";
import assert from "node:assert/strict";
import { filterModels, mergeModels, offeredModels, resolveModel, sessionModel } from "./model-choices.ts";

// An endpoint behind a gateway that serves models under IDs nobody would type.
const endpoint = [
  { id: "claude-fable-5-dd-lacol-b9-5.3newq", name: "qwen3.5-9b-local" },
  { id: "claude-fable-5-dd-3.5-MLG/gro-iaz", name: "zai-org/GLM-5.3" },
  // Two models that share a name cannot be told apart by it.
  { id: "claude-fable-5-dd-a", name: "twin" },
  { id: "claude-fable-5-dd-b", name: "twin" },
];

test("with an endpoint catalog, the menu is the endpoint's models", () => {
  const client = [
    { id: "default", name: "Default (recommended)", isDefault: true },
    // The client guessed this name from the ID's prefix, and knows its levels.
    {
      id: "claude-fable-5-dd-lacol-b9-5.3newq",
      name: "Fable 5",
      description: "Fable 5 · For the hardest problems",
      efforts: ["low", "high"],
    },
    { id: "sonnet", name: "Sonnet" },
  ];
  const merged = mergeModels(client, endpoint);
  assert.deepEqual(
    merged.map(entry => entry.id),
    endpoint.map(entry => entry.id),
    "the client's official entries are left out",
  );
  assert.equal(merged[0].name, "qwen3.5-9b-local", "the endpoint's name wins over the client's guess");
  assert.deepEqual(merged[0].efforts, ["low", "high"], "what the client knows about the same ID is kept");
  assert.equal(merged[0].description, undefined, "the client's description of a model it guessed at is not");
  assert.deepEqual(
    mergeModels(client, endpoint, "sonnet")[0],
    { id: "sonnet", name: "sonnet" },
    "the model in use stays, first, as its ID",
  );
  assert.deepEqual(
    mergeModels(client, endpoint, "my-own-id")[0],
    { id: "my-own-id", name: "my-own-id" },
    "even one neither lists",
  );
  assert.deepEqual(mergeModels(client, []), client, "no endpoint catalog, the client list as it was");
});

test("a profile offers the models chosen in it, or all of them", () => {
  assert.deepEqual(offeredModels(endpoint, []).length, endpoint.length);
  assert.deepEqual(offeredModels(endpoint, undefined).length, endpoint.length);
  assert.deepEqual(offeredModels(endpoint, ["claude-fable-5-dd-3.5-MLG/gro-iaz", "gone-model"]), [
    endpoint[1],
    { id: "gone-model", name: "gone-model" },
  ]);
});

test("a model typed by its name is sent by its ID", () => {
  assert.equal(resolveModel("qwen3.5-9b-local", endpoint), "claude-fable-5-dd-lacol-b9-5.3newq");
  assert.equal(
    resolveModel("  QWEN3.5-9B-LOCAL ", endpoint),
    "claude-fable-5-dd-lacol-b9-5.3newq",
    "case and spaces do not matter",
  );
  assert.equal(
    resolveModel("claude-fable-5-dd-3.5-MLG/gro-iaz", endpoint),
    "claude-fable-5-dd-3.5-MLG/gro-iaz",
    "an ID stays an ID",
  );
  assert.equal(resolveModel("twin", endpoint), "twin", "an ambiguous name is not guessed at");
  assert.equal(resolveModel("claude-opus-5-5", endpoint), "claude-opus-5-5", "anything else is sent as typed");
  assert.equal(resolveModel("qwen3.5-9b-local", []), "qwen3.5-9b-local", "without a catalog nothing can be resolved");
});

test("the menu narrows to models whose name or ID matches", () => {
  assert.deepEqual(
    filterModels(endpoint, "glm").map(entry => entry.name),
    ["zai-org/GLM-5.3"],
  );
  assert.deepEqual(
    filterModels(endpoint, "LACOL").map(entry => entry.name),
    ["qwen3.5-9b-local"],
    "by ID too",
  );
  assert.equal(filterModels(endpoint, "  ").length, endpoint.length);
});

test("reconnecting sessions never borrow the catalog or profile default", () => {
  const catalog = [{ id: "default", name: "Opus 5.5", isDefault: true }];
  assert.equal(sessionModel(undefined, false, true, "opus", catalog), undefined);
  assert.equal(sessionModel(undefined, true, true, "opus", catalog), undefined);
  assert.equal(sessionModel(undefined, true, false, "opus", catalog), undefined);
  assert.equal(sessionModel("claude-fable-5-2", true, true, "opus", catalog), "claude-fable-5-2");
  assert.equal(sessionModel(undefined, false, false, "opus", catalog), "opus");
  assert.equal(sessionModel(undefined, false, false, undefined, catalog), "default");
});
