import { test } from "node:test";
import assert from "node:assert/strict";
import { compactTokens, contextMeter, measuredUsage } from "./context-meter.ts";

test("a share of context needs a window the client actually reported", () => {
  // No window means no denominator, so no percentage is invented from a
  // guessed model size.
  for (const usage of [
    undefined,
    {},
    { context_tokens: 1000 },
    { context_window: 200000 },
    { context_tokens: 1000, context_window: 0 },
    { context_tokens: 1000, context_window: -5 },
    { context_tokens: -1, context_window: 200000 },
    { context_tokens: Number.NaN, context_window: 200000 },
    { context_tokens: 1000, context_window: Number.POSITIVE_INFINITY },
  ]) {
    assert.equal(contextMeter(usage), undefined, JSON.stringify(usage));
  }
  assert.deepEqual(contextMeter({ context_tokens: 50000, context_window: 200000 }), {
    fraction: 0.25,
    percent: 25,
    used: 50000,
    window: 200000,
    level: "normal",
  });
});

test("the meter stays truthful at the edges instead of rounding a real usage away", () => {
  // Real but tiny usage must not display as an empty 0%.
  const tiny = contextMeter({ context_tokens: 12, context_window: 200000 });
  assert.equal(tiny.percent, 1);
  assert.ok(tiny.fraction > 0 && tiny.fraction < 0.001);
  // Genuinely zero stays zero.
  assert.equal(contextMeter({ context_tokens: 0, context_window: 200000 }).percent, 0);
  // A client may report more than its own stated window; the ring cannot
  // overdraw, but the underlying counts stay exact.
  const over = contextMeter({ context_tokens: 260000, context_window: 200000 });
  assert.equal(over.fraction, 1);
  assert.equal(over.percent, 100);
  assert.equal(over.used, 260000);
  assert.equal(over.level, "full");
  assert.equal(contextMeter({ context_tokens: 160000, context_window: 200000 }).level, "high");
  assert.equal(contextMeter({ context_tokens: 159000, context_window: 200000 }).level, "normal");
});

test("compact token labels stay readable without misstating magnitude", () => {
  assert.equal(compactTokens(0), "0");
  assert.equal(compactTokens(999), "999");
  assert.equal(compactTokens(1400), "1.4k");
  assert.equal(compactTokens(12000), "12k");
  assert.equal(compactTokens(200000), "200k");
  assert.equal(compactTokens(1_000_000), "1M");
  assert.equal(compactTokens(1_500_000), "1.5M");
  assert.equal(compactTokens(-1), "0");
  assert.equal(compactTokens(Number.NaN), "0");
});

test("a window the user set wins over the client and the published one", () => {
  const usage = { context_tokens: 10, context_window: 200_000 };
  assert.equal(measuredUsage(usage, { context_window: 1_000_000, source: "yours" }).context_window, 1_000_000);
  assert.equal(
    measuredUsage(usage, { context_window: null, source: "yours" }).context_window,
    undefined,
    "a cleared setting means no window, not the client's",
  );
});

test("otherwise the client's window stands, and the published one fills in for it", () => {
  assert.equal(
    measuredUsage({ context_tokens: 10, context_window: 200_000 }, { context_window: 400_000, source: "published" })
      .context_window,
    200_000,
  );
  assert.equal(
    measuredUsage({ context_tokens: 10 }, { context_window: 400_000, source: "published" }).context_window,
    400_000,
  );
  assert.equal(measuredUsage({ context_tokens: 10 }, undefined).context_window, undefined);
});

test("with no usage there is nothing to measure", () => {
  assert.equal(measuredUsage(undefined, { context_window: 1, source: "yours" }), undefined);
});
