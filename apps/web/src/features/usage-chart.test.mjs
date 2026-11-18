import { test } from "node:test";
import assert from "node:assert/strict";
import {
  allowanceCards,
  busiestHour,
  changeLabel,
  dateLabel,
  dayParts,
  heatShade,
  shade,
  stackedBar,
  usageInsights,
  usageTiles,
  windowLabel,
} from "./usage-chart.ts";
import { usageBackoffSeconds } from "./account-ui.ts";

const bucket = (overrides = {}) => ({
  input: 0,
  cache_read: 0,
  cache_write_5m: 0,
  cache_write_1h: 0,
  output: 0,
  reasoning: 0,
  total: 0,
  cost: 0,
  unpriced: 0,
  calls: 0,
  user_messages: 0,
  active_minutes: 0,
  ...overrides,
});
const window = (account_id, window_minutes) => ({
  account_id,
  account: account_id.toUpperCase(),
  provider: "claude",
  plan: "max",
  window_minutes,
});
const t = (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key]));

test("each account gets one card with its windows shortest first", () => {
  const cards = allowanceCards([window("a", 10080), window("b", 300), window("a", 300)]);
  assert.deepEqual(
    cards.map(card => [card.id, card.name, card.windows.map(w => w.window_minutes)]),
    [
      ["a", "A", [300, 10080]],
      ["b", "B", [300]],
    ],
  );
  assert.deepEqual(allowanceCards([]), []);
});

test("windows are named by length, with some slack either way", () => {
  assert.equal(windowLabel(300), "5h");
  assert.equal(windowLabel(330), "5h");
  assert.equal(windowLabel(331), "6h");
  assert.equal(windowLabel(10080 - 60), "7d");
  assert.equal(windowLabel(1440), "24h");
});

test("changes are signed to one decimal, and absent when there is nothing to compare", () => {
  assert.equal(changeLabel(12.345), "+12.3%");
  assert.equal(changeLabel(0), "+0.0%");
  assert.equal(changeLabel(-4), "-4.0%");
  assert.equal(changeLabel(undefined), "");
});

test("a day stacks the token kinds shown, or cost or time alone", () => {
  const day = bucket({ output: 5, input: 3, cache_read: 10, cache_write_5m: 1, cost: 2.5, active_minutes: 40 });
  const all = { output: true, input: true, cache: true };
  assert.deepEqual(
    dayParts(day, "tokens", all).map(part => [part.key, part.value]),
    [
      ["output", 5],
      ["input", 3],
      ["cache", 11],
    ],
  );
  assert.deepEqual(
    dayParts(day, "tokens", { ...all, cache: false }).map(part => part.key),
    ["output", "input"],
  );
  assert.deepEqual(
    dayParts(day, "cost", all).map(part => [part.key, part.value]),
    [["cost", 2.5]],
  );
  assert.deepEqual(
    dayParts(day, "time", all).map(part => [part.key, part.value]),
    [["time", 40]],
  );
});

test("a bar is centred in its slot, stacked from the bottom and scaled to the tallest day", () => {
  const parts = [
    { key: "a", label: "A", value: 50, color: "x" },
    { key: "b", label: "B", value: 50, color: "y" },
    { key: "c", label: "C", value: 0, color: "z" },
  ];
  const rects = stackedBar(parts, 1, 4, 100, 400, 200);
  // Four slots of 100: the bar is at most 22 wide, centred in the second slot.
  assert.deepEqual(
    rects.map(r => [r.key, r.x, r.width, r.y, r.height]),
    [
      ["a", 139, 22, 100, 98.5],
      ["b", 139, 22, 0, 98.5],
    ],
  );
  // A narrow slot still gets a visible bar.
  assert.equal(stackedBar(parts, 0, 400, 100, 400, 200)[0].width, 3);
});

test("dates read as month/day and the heat map fades by square root", () => {
  assert.equal(dateLabel("2026-10-07"), "10/7");
  assert.equal(heatShade(0, 10), shade(5));
  assert.equal(heatShade(10, 10), shade(96));
  assert.equal(heatShade(2.5, 10), shade(55), "a quarter of the busiest hour is half the way");
});

test("the busiest hour is found, or nothing when nothing happened", () => {
  assert.deepEqual(
    busiestHour([
      [0, 1],
      [0, 5, 2],
    ]),
    { day: 1, hour: 1 },
  );
  assert.equal(busiestHour([[0, 0], [0]]), undefined);
});

test("tiles and insights read from the report", () => {
  const report = {
    totals: bucket({ total: 1000, input: 400, output: 600, cost: 1.5, calls: 6, user_messages: 3 }),
    previous: bucket({ total: 500, input: 400, output: 100, cost: 0 }),
    days: [
      { ...bucket({ total: 300 }), date: "2026-10-01" },
      { ...bucket({ total: 700 }), date: "2026-10-02" },
    ],
    models: [{ ...bucket({ total: 800 }), model: "opus", provider: "claude", price: null, custom_price: false }],
    heatmap_active: [
      [0, 0],
      [0, 0, 9],
    ],
    duration_seconds: 3600,
    previous_duration_seconds: 0,
    conversation_count: 2,
  };
  const tiles = Object.fromEntries(usageTiles(report, t).map(tile => [tile.key, tile]));
  assert.equal(tiles.total.change, 100);
  assert.equal(tiles.cost.change, undefined, "no cost before, so no change to show");
  assert.equal(tiles.messages.value, "9");
  assert.equal(tiles.sessions.value, "2");
  assert.deepEqual(usageInsights(report, t, ["Mon", "Tue"]), [
    "opus did 80% of the work.",
    "The busiest day was 2026-10-02, with 700 tokens.",
    "You work hardest on Tue around 2:00.",
    "Each of your messages led to 2.0 model calls on average.",
  ]);
  assert.deepEqual(usageInsights({ ...report, totals: bucket() }, t, []), [], "nothing used, nothing to say");
});

test("a usage check waits a minute after an answer and doubles after each refusal", () => {
  assert.equal(usageBackoffSeconds(0), 60);
  assert.equal(usageBackoffSeconds(1), 300);
  assert.equal(usageBackoffSeconds(2), 600);
  assert.equal(usageBackoffSeconds(3), 1200);
  assert.equal(usageBackoffSeconds(4), 1800, "capped at half an hour");
  assert.equal(usageBackoffSeconds(9), 1800);
  assert.equal(usageBackoffSeconds(1, 900), 900, "longer when the endpoint asks");
  assert.equal(usageBackoffSeconds(1, null), 300);
});
