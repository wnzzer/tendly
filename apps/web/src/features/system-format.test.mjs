import { test } from "node:test";
import assert from "node:assert/strict";
import {
  formatBytes,
  formatDuration,
  formatRate,
  HISTORY_LENGTH,
  levelFor,
  pushSample,
  sparkPath,
} from "./system-format.ts";

test("sizes, rates and durations read the way the system page shows them", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(6.8 * 1024 ** 3), "6.8 GB");
  assert.equal(formatBytes(568 * 1024 ** 3), "568 GB");
  assert.equal(formatRate(33.8 * 1024), "33.8 KB/s");
  assert.equal(formatDuration(59), "0m");
  assert.equal(formatDuration(3 * 3600 + 120), "3h 2m");
  assert.equal(formatDuration(27 * 3600), "1d 3h");
});

test("levels and the short history behind each trend line", () => {
  assert.deepEqual([levelFor(10), levelFor(70), levelFor(95)], ["ok", "warn", "danger"]);
  let series = [];
  for (let index = 0; index < HISTORY_LENGTH + 5; index++) series = pushSample(series, index);
  assert.equal(series.length, HISTORY_LENGTH);
  assert.equal(series[0], 5, "the oldest readings fall off");
  assert.equal(sparkPath([1]), "", "one point is not a line");
  const path = sparkPath([0, 50, 100], 100);
  assert.match(path, /^M[\d.]+ 27\.00 L[\d.]+ 14\.00 L100\.00 1\.00$/);
});
