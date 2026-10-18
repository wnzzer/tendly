import { test } from 'node:test';
import assert from 'node:assert/strict';
import { claimPrune, PRUNE_INTERVAL_MS } from './branch-prune.ts';

test('the first open for a workspace always fetches and records the time', () => {
  const last = new Map();
  assert.equal(claimPrune(last, 'w1', 1000), true);
  assert.equal(last.get('w1'), 1000);
});

test('a second open within the interval does not fetch and keeps the first time', () => {
  const last = new Map([['w1', 1000]]);
  assert.equal(claimPrune(last, 'w1', 1000 + PRUNE_INTERVAL_MS - 1), false);
  assert.equal(last.get('w1'), 1000);
});

test('an open after the interval fetches again and records the new time', () => {
  const last = new Map([['w1', 1000]]);
  const now = 1000 + PRUNE_INTERVAL_MS;
  assert.equal(claimPrune(last, 'w1', now), true);
  assert.equal(last.get('w1'), now);
});

test('each workspace is throttled on its own clock', () => {
  const last = new Map([['w1', 1000]]);
  assert.equal(claimPrune(last, 'w2', 1000), true);
  assert.equal(last.get('w1'), 1000);
  assert.equal(last.get('w2'), 1000);
});
