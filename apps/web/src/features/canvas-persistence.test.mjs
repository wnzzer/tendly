import { afterEach, test } from "node:test";
import assert from "node:assert/strict";
import { nextTick, ref } from "vue";
import { initialPane, isSharedCanvas, layoutInTrouble, useCanvasPersistence } from "./canvas-persistence.ts";
import { setBackendCapabilities } from "./backend-capabilities.ts";
import { readCanvasCache, writeCanvasCache } from "./canvas-cache.ts";

const pane = (id, kind = "agent_chat") => ({ type: "pane", id, kind, title: id });
const doc = root => ({ version: 1, root });
const split = (first, second) => ({ type: "split", id: "root", direction: "horizontal", ratio: 0.5, first, second });
const stack = (panes, activePaneId) => ({ type: "stack", id: "stack", panes, activePaneId });
const workspace = { id: "w", name: "w", root_path: "/w", created_at: "2026-01-01T00:00:00Z" };
const KEY = "tendly:shared-canvas:test";

function memoryStorage() {
  const values = new Map();
  return { values, getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
}

/** A persistence over plain refs, with the server answering from `respond` and every request recorded. */
function harness(
  t,
  {
    layout = doc(pane("a")),
    respond = () => {
      throw Error("no request expected");
    },
    storage = memoryStorage(),
  } = {},
) {
  const requests = [],
    reports = [],
    ready = [];
  t.mock.method(globalThis, "fetch", async (url, init = {}) => {
    const request = {
      url: String(url),
      method: init.method ?? "GET",
      body: init.body === undefined ? undefined : JSON.parse(init.body),
    };
    requests.push(request);
    return respond(request);
  });
  const state = { layout: ref(layout), selectedPaneId: ref(), sessions: ref([]) };
  const persistence = useCanvasPersistence({
    ...state,
    storage,
    storageKey: ref(KEY),
    report: cause => reports.push(cause),
    onReady: preferred => ready.push(preferred.id),
  });
  t.after(() => persistence.dispose());
  return { ...state, ...persistence, requests, reports, ready, storage };
}

afterEach(() => setBackendCapabilities({ ok: false }));

test("a shared canvas answer is trusted only with a whole revision and a valid or absent layout", () => {
  assert.equal(isSharedCanvas({ revision: 0, layout: null }), true);
  assert.equal(isSharedCanvas({ revision: 3, layout: doc(pane("a")) }), true);
  for (const shared of [
    { revision: -1, layout: null },
    { revision: 1.5, layout: null },
    { revision: "2", layout: null },
    { revision: 1, layout: { version: 1 } },
  ])
    assert.equal(isSharedCanvas(shared), false, JSON.stringify(shared));
});

test("a loaded layout focuses its first pane, or the tab a stack had in front", () => {
  assert.equal(initialPane(pane("a")).id, "a");
  assert.equal(initialPane(split(stack([pane("b"), pane("c")], "c"), pane("d"))).id, "c");
  assert.equal(initialPane(stack([pane("b"), pane("c")], "gone")).id, "b");
});

test("the status bar warns only while the layout is not safely stored", () => {
  for (const status of ["Save failed", "Save conflict", "Memory only"])
    assert.equal(layoutInTrouble(status), true, status);
  for (const status of ["Saved", "Saved in this browser", "Unsaved", "Saving…"])
    assert.equal(layoutInTrouble(status), false, status);
});

test("without a shared canvas the layout comes from this browser and every change is kept there", async t => {
  const storage = memoryStorage();
  writeCanvasCache(storage, KEY, doc(split(pane("a"), pane("b"))), false);
  const canvas = harness(t, { storage });
  await canvas.initializeCanvas(workspace);
  assert.deepEqual(canvas.requests, [], "nothing is asked of the server");
  assert.equal(canvas.canvasReady.value, true);
  assert.equal(canvas.selectedPaneId.value, "a");
  assert.deepEqual(canvas.ready, ["w"]);
  assert.equal(canvas.layoutStatus.value, "Saved in this browser");
  canvas.layout.value = doc(pane("b"));
  await nextTick();
  assert.equal(canvas.selectedPaneId.value, "b", "a focus whose pane closed moves to one still open");
  assert.equal(readCanvasCache(storage, KEY).layout.root.id, "b");
  assert.equal(canvas.hasPendingLayout(), false);
});

test("a shared canvas saves against the revision it read, and a conflict stops saving until resolved", async t => {
  setBackendCapabilities({ ok: true, api_version: 2, capabilities: ["shared_canvas"] });
  let conflict = false;
  const canvas = harness(t, {
    respond: request => {
      if (request.method === "GET") return Response.json({ revision: 4, layout: doc(pane("a")) });
      return conflict ? Response.json({ error: "stale" }, { status: 409 }) : Response.json({ revision: 5 });
    },
  });
  await canvas.initializeCanvas(workspace);
  assert.equal(canvas.layoutStatus.value, "Saved");
  canvas.layout.value = doc(split(pane("a"), pane("b")));
  await nextTick();
  assert.equal(canvas.layoutStatus.value, "Unsaved");
  assert.equal(canvas.hasPendingLayout(), true);
  await canvas.flushLayout();
  assert.deepEqual(canvas.requests.at(-1), {
    url: "/api/canvas/layout",
    method: "PUT",
    body: { layout: doc(split(pane("a"), pane("b"))), expected_revision: 4 },
  });
  assert.equal(canvas.canvasRevision.value, 5);
  assert.equal(canvas.layoutStatus.value, "Saved");
  conflict = true;
  canvas.layout.value = doc(pane("b"));
  await nextTick();
  await canvas.flushLayout();
  assert.equal(canvas.canvasConflict.value, true);
  assert.equal(canvas.layoutStatus.value, "Save conflict");
  assert.deepEqual(canvas.reports, [], "a conflict is asked about, not reported");
  const sent = canvas.requests.length;
  canvas.layout.value = doc(pane("a"));
  await nextTick();
  await canvas.flushLayout();
  assert.equal(canvas.requests.length, sent, "nothing more is sent while in conflict");
});

test("an unsaved change that lost to the server is kept to restore", async t => {
  setBackendCapabilities({ ok: true, api_version: 2, capabilities: ["shared_canvas"] });
  const storage = memoryStorage();
  writeCanvasCache(storage, KEY, doc(pane("mine")), true);
  const canvas = harness(t, { storage, respond: () => Response.json({ revision: 1, layout: doc(pane("theirs")) }) });
  await canvas.initializeCanvas(workspace);
  assert.equal(canvas.layout.value.root.id, "theirs");
  assert.equal(canvas.localRecovery.value.root.id, "mine");
  assert.equal(readCanvasCache(storage, KEY + ":recovery").layout.root.id, "mine");
  canvas.restoreLocalRecovery();
  assert.equal(canvas.layout.value.root.id, "mine");
  assert.equal(canvas.selectedPaneId.value, "mine");
  assert.equal(canvas.localRecovery.value, undefined);
});

test("an agent's change to the canvas loads, unless this window has one of its own waiting", async t => {
  setBackendCapabilities({ ok: true, api_version: 2, capabilities: ["shared_canvas"] });
  let served = { revision: 1, layout: doc(split(pane("a"), pane("b"))) };
  const canvas = harness(t, {
    respond: request => (request.method === "GET" ? Response.json(served) : Response.json({ revision: 9 })),
  });
  await canvas.initializeCanvas(workspace);
  canvas.selectedPaneId.value = "b";
  await canvas.agentCanvas(1);
  assert.equal(canvas.requests.length, 1, "a revision already seen is ignored");
  served = { revision: 2, layout: doc(split(pane("b"), pane("c"))) };
  await canvas.agentCanvas(2);
  assert.equal(canvas.layout.value.root.second.id, "c");
  assert.equal(canvas.selectedPaneId.value, "b", "the focus stays on a pane still there");
  assert.equal(canvas.layoutStatus.value, "Saved");
  await nextTick();
  assert.equal(canvas.hasPendingLayout(), false, "a layout loaded from the server is not saved back");
  canvas.layout.value = doc(pane("b"));
  await nextTick();
  await canvas.agentCanvas(3);
  assert.equal(canvas.canvasConflict.value, true);
  assert.equal(canvas.layout.value.root.id, "b", "the window's own change is not overwritten");
});
