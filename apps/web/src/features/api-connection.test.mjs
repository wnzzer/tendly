import { afterEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { ApiConnectionError, ApiError, request, json } from "./api.ts";

let fetchMock;
afterEach(() => {
  mock.restoreAll();
  fetchMock = undefined;
});

test("a failed read gets a useful connection error instead of raw Failed to fetch", async () => {
  fetchMock = mock.method(globalThis, "fetch", async () => {
    throw new TypeError("Failed to fetch");
  });
  await assert.rejects(
    request("/sessions"),
    error =>
      error instanceof ApiConnectionError &&
      error.status === 0 &&
      !error.outcomeUnknown &&
      !error.message.includes("Failed to fetch"),
  );
  assert.equal(fetchMock.mock.calls.length, 1);
});
test("mutations report an unknown outcome and are never automatically replayed", async () => {
  for (const method of ["POST", "PUT", "PATCH", "DELETE"]) {
    mock.restoreAll();
    fetchMock = mock.method(globalThis, "fetch", async () => {
      throw new TypeError("Failed to fetch private-sentinel");
    });
    await assert.rejects(
      request("/fixture", json(method, { value: "private-sentinel" })),
      error =>
        error instanceof ApiConnectionError && error.outcomeUnknown && !error.message.includes("private-sentinel"),
    );
    assert.equal(fetchMock.mock.calls.length, 1);
  }
});
test("intentional request cancellation stays cancellation, not a service-offline error", async () => {
  const controller = new AbortController(),
    reason = new DOMException("Cancelled", "AbortError");
  controller.abort(reason);
  fetchMock = mock.method(globalThis, "fetch", async () => {
    throw reason;
  });
  await assert.rejects(request("/fixture", { signal: controller.signal }), error => error === reason);
});
test("a response-body disconnect after a mutation also preserves outcome uncertainty", async () => {
  fetchMock = mock.method(globalThis, "fetch", async () => ({
    ok: true,
    status: 200,
    json: async () => {
      throw new TypeError("body disconnected");
    },
  }));
  await assert.rejects(
    request("/fixture", json("POST", {})),
    error => error instanceof ApiConnectionError && error.outcomeUnknown,
  );
  assert.equal(fetchMock.mock.calls.length, 1);
});
test("unreadable successful responses do not expose raw HTML or masquerade as network loss", async () => {
  fetchMock = mock.method(
    globalThis,
    "fetch",
    async () => new Response("<html>private-sentinel</html>", { status: 200 }),
  );
  await assert.rejects(
    request("/fixture"),
    error =>
      error instanceof ApiError &&
      !(error instanceof ApiConnectionError) &&
      !error.message.includes("private-sentinel"),
  );
});
test("a later successful refresh remains usable after a network failure", async () => {
  let calls = 0;
  fetchMock = mock.method(globalThis, "fetch", async () => {
    if (calls++ === 0) throw new TypeError("Failed to fetch");
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  });
  await assert.rejects(request("/health"), ApiConnectionError);
  assert.deepEqual(await request("/health"), { ok: true });
  assert.equal(fetchMock.mock.calls.length, 2);
});
