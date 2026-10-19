import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cacheHitRate, cacheSavings, failureRate, formatCost, formatTokens, outputSpeed, rangeStart, trend } from './usage-model.ts';

test('tokens and cost read at a glance', () => {
  assert.equal(formatTokens(0), '0');
  assert.equal(formatTokens(950), '950');
  assert.equal(formatTokens(13_300_000), '13.3M');
  assert.equal(formatTokens(858_000_000), '858M');
  assert.equal(formatTokens(1_200_000_000), '1.2B');
  assert.equal(formatCost(0.4061), '$0.406');
  assert.equal(formatCost(88.2), '$88.20');
  assert.equal(formatCost(2367.4), '$2,367');
});

test('cache hit rate is cache reads over all input, and savings use the price gap', () => {
  assert.equal(cacheHitRate({ input: 10, cache_read: 970, cache_write_5m: 15, cache_write_1h: 5 }), 97);
  assert.equal(cacheHitRate({ input: 0, cache_read: 0, cache_write_5m: 0, cache_write_1h: 0 }), 0);
  const saved = cacheSavings([{ cache_read: 1_000_000, price: { input: 4, output: 20, cache_read: 0.2 } }, { cache_read: 5_000_000, price: null }]);
  assert.ok(Math.abs(saved - 3.8) < 1e-9, 'an unpriced model saves nothing it can be credited with');
});

test('a trend needs a previous period, and a range starts at local midnight', () => {
  assert.equal(trend(124, 100), 24);
  assert.equal(trend(5, 0), undefined);
  const start = new Date(rangeStart(7, new Date(2026, 9, 5, 15, 30)) * 1000);
  assert.deepEqual([start.getDate(), start.getHours(), start.getMinutes()], [29, 0, 0]);
});

test('speed and failure rate only where a client logs them', () => {
  assert.equal(outputSpeed({ timed_output: 650, timed_ms: 10_000 }), 65);
  assert.equal(outputSpeed({ timed_output: 0, timed_ms: 0 }), undefined, 'nothing timed');
  assert.equal(outputSpeed({}), undefined, 'an older backend');
  assert.equal(failureRate({ answered: 98, failed: 2 }), 2);
  assert.equal(failureRate({ answered: 0, failed: 0 }), undefined, 'a client that logs no failures is not 0%');
  assert.equal(failureRate({}), undefined);
});
