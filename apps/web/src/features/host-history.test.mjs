import { test } from "node:test";
import assert from "node:assert/strict";
import {
  bin,
  busiestSessions,
  chartPaths,
  gigahertz,
  measures,
  nearest,
  niceCeiling,
  niceRange,
  samples,
  segments,
  watts,
} from "./host-history.ts";

const host = (ts, extra = {}) => ({
  ts,
  cpu_avg: 10,
  cpu_max: 20,
  memory_used_avg: 4,
  memory_used_max: 6,
  memory_total: 8,
  fan_rpm_avg: null,
  fan_rpm_max: null,
  temperature_avg: null,
  temperature_max: null,
  ...extra,
});

test("a measure keeps only the points that read it, in time order", () => {
  const points = [
    host(20, { temperature_avg: 50, temperature_max: 60 }),
    host(10),
    host(30, { temperature_avg: 40, temperature_max: 45 }),
  ];
  assert.deepEqual(samples(points, measures.temperature), [
    { ts: 20, avg: 50, max: 60 },
    { ts: 30, avg: 40, max: 45 },
  ]);
  assert.deepEqual(
    samples(points, measures.cpu).map(p => p.ts),
    [10, 20, 30],
  );
  // Memory is a share of the machine, and a point without a total has none.
  assert.deepEqual(samples([host(1), host(2, { memory_total: 0 })], measures.memory), [{ ts: 1, avg: 50, max: 75 }]);
  // The clock is absent from an older backend's points, and then not charted.
  assert.deepEqual(samples([host(1), host(2, { cpu_mhz_avg: 3204, cpu_mhz_max: 3504 })], measures.clock), [
    { ts: 2, avg: 3204, max: 3504 },
  ]);
  // A peak below its own average (a rounding artefact) never inverts the band.
  assert.deepEqual(samples([host(1, { cpu_avg: 5, cpu_max: 4 })], measures.cpu), [{ ts: 1, avg: 5, max: 5 }]);
});

test("axis tops are round numbers at or above the data", () => {
  assert.equal(niceCeiling(57), 100);
  assert.equal(niceCeiling(7199), 10000);
  assert.equal(niceCeiling(1800), 2000);
  assert.equal(niceCeiling(41), 50);
  assert.equal(niceCeiling(0), 1);
  assert.equal(niceCeiling(3, 50), 50);
});

test("a range that need not start at zero is widened to whole steps", () => {
  const points = values => values.map((value, ts) => ({ ts, avg: value, max: value }));
  // Readings between 52 and 60 °C fit a 50–60 axis, not a 0–100 one.
  assert.deepEqual(niceRange(points([52, 60]), 10, 10), { bottom: 50, top: 60 });
  // A steady reading still gets the minimum span, centred where it can be.
  assert.deepEqual(niceRange(points([52, 52]), 10, 20), { bottom: 40, top: 60 });
  // Never below zero.
  assert.deepEqual(niceRange(points([3, 4]), 10, 20), { bottom: 0, top: 20 });
  assert.deepEqual(niceRange([], 10, 20), { bottom: 0, top: 20 });
});

test("dense points are gathered into buckets that keep the average, the peak and the gaps", () => {
  const raw = [0, 5, 10, 15, 60, 65].map(ts => ({ ts, avg: ts, max: ts + 100 }));
  const { points, resolution } = bin(raw, 0, 100, 5, 5);
  assert.equal(resolution, 20);
  assert.deepEqual(points, [
    { ts: 10, avg: 7.5, max: 115 },
    // Nothing between 20 and 60: that bucket stays empty rather than zero.
    { ts: 70, avg: 62.5, max: 165 },
  ]);
  // Points already sparser than a bucket are left alone.
  assert.deepEqual(bin(raw, 0, 100, 50, 5), { points: raw, resolution: 5 });
});

test("a raised axis puts its bottom at the baseline", () => {
  const { line } = chartPaths(
    [
      { ts: 0, avg: 50, max: 50 },
      { ts: 10, avg: 60, max: 60 },
    ],
    10,
    { from: 0, to: 10, width: 10, height: 10, top: 60, bottom: 50 },
  );
  assert.equal(line, "M0 10L10 0");
});

test("power is charted where the backend records it, and reads in watts", () => {
  assert.deepEqual(samples([host(1), host(2, { power_avg: 8.25, power_max: 21 })], measures.power), [
    { ts: 2, avg: 8.25, max: 21 },
  ]);
  assert.equal(watts(8.25), "8.3 W");
  assert.deepEqual(samples([host(1), host(2, { gpu_avg: 12, gpu_max: 40 })], measures.gpu), [
    { ts: 2, avg: 12, max: 40 },
  ]);
  assert.equal(watts(21.4), "21 W");
});

test("clocks read in gigahertz", () => {
  assert.equal(gigahertz(3504), "3.5 GHz");
  assert.equal(gigahertz(912), "0.9 GHz");
});

test("a missing stretch splits the line instead of bridging it", () => {
  const points = [0, 5, 10, 60, 65].map(ts => ({ ts, avg: 1, max: 2 }));
  assert.deepEqual(
    segments(points, 5).map(run => run.map(p => p.ts)),
    [
      [0, 5, 10],
      [60, 65],
    ],
  );
  const { line, band } = chartPaths(points, 5, { from: 0, to: 100, width: 100, height: 10, top: 10 });
  assert.equal(line.match(/M/g).length, 2, line);
  assert.equal(band.match(/Z/g).length, 2, band);
  assert.ok(line.startsWith("M0 9L5 9L10 9M60 9"), line);
});

test("values are clamped to the frame and a lone point is still drawn", () => {
  const { line } = chartPaths([{ ts: 50, avg: 150, max: 200 }], 5, {
    from: 0,
    to: 100,
    width: 100,
    height: 10,
    top: 100,
  });
  assert.equal(line, "M47.5 0L52.5 0");
  const { line: below } = chartPaths(
    [
      { ts: 0, avg: -5, max: 0 },
      { ts: 5, avg: 0, max: 0 },
    ],
    5,
    { from: 0, to: 100, width: 100, height: 10, top: 100 },
  );
  assert.equal(below, "M0 10L5 10");
});

test("hover finds the nearest point but not across a gap", () => {
  const points = [0, 5, 60].map(ts => ({ ts, avg: ts, max: ts }));
  assert.equal(nearest(points, 6, 5)?.ts, 5);
  assert.equal(nearest(points, 58, 5)?.ts, 60);
  assert.equal(nearest(points, 30, 5), undefined);
  assert.equal(nearest([], 30, 5), undefined);
});

test("sessions rank by CPU share over the buckets they ran in", () => {
  const point = (session_id, ts, cpu, memory = 1) => ({
    ts,
    session_id,
    cpu_avg: cpu,
    cpu_max: cpu * 2,
    memory_avg: memory,
    memory_max: memory,
  });
  const ranked = busiestSessions(
    [
      point("steady", 0, 40),
      point("steady", 5, 40),
      point("steady", 10, 40),
      // Ran for one bucket, but hard.
      point("burst", 5, 160, 9),
      point("idle", 0, 0, 50),
      point("ended", 0, 0, 2),
    ],
    4,
    3,
  );
  assert.deepEqual(
    ranked.map(row => row.session_id),
    ["burst", "steady", "idle"],
  );
  assert.deepEqual(ranked[0], { session_id: "burst", cpu_avg: 40, cpu_max: 80, memory_max: 9 });
  assert.equal(ranked[1].cpu_avg, 10);
  assert.deepEqual(busiestSessions([], 8), []);
});
