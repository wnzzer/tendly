import { test } from "node:test";
import assert from "node:assert/strict";
import { addSample, flingStep, releaseVelocity, FLING_WINDOW_MS } from "./terminal-scroll.ts";

test("a quick flick up leaves a velocity that brings later lines into view", () => {
  const samples = [
    { y: 400, at: 0 },
    { y: 360, at: 20 },
    { y: 300, at: 40 },
  ];
  assert.ok(releaseVelocity(samples, 40) > 0);
  // Down is the other way.
  assert.ok(
    releaseVelocity(
      [
        { y: 300, at: 0 },
        { y: 400, at: 40 },
      ],
      40,
    ) < 0,
  );
});

test("a finger that stopped before lifting does not fling", () => {
  const samples = [
    { y: 400, at: 0 },
    { y: 200, at: 40 },
  ];
  assert.equal(releaseVelocity(samples, 40 + FLING_WINDOW_MS + 50), 0);
  // Nor does a slow drag.
  assert.equal(
    releaseVelocity(
      [
        { y: 400, at: 0 },
        { y: 395, at: 80 },
      ],
      80,
    ),
    0,
  );
});

test("old samples are dropped as the drag goes on", () => {
  let samples = [];
  for (let at = 0; at <= 1000; at += 16) samples = addSample(samples, { y: 1000 - at, at });
  assert.ok(samples.length <= Math.ceil(FLING_WINDOW_MS / 16) + 1);
});

test("a flick slows down and comes to rest", () => {
  let velocity = 2,
    travelled = 0,
    frames = 0;
  while (velocity && frames < 1000) {
    const step = flingStep(velocity, 16);
    travelled += step.distance;
    velocity = step.velocity;
    frames++;
  }
  assert.equal(velocity, 0);
  assert.ok(frames < 200);
  // About velocity × time constant, which is well over a screenful for a hard flick.
  assert.ok(travelled > 500 && travelled < 700);
});
