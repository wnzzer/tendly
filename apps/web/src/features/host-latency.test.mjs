import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatLatency, SLOW_LATENCY_MS } from './host-latency.ts';

test('offline reads as offline regardless of any stale measurement', () => {
  assert.deepEqual(formatLatency(12, false), { text: 'Offline', level: 'offline' });
  assert.deepEqual(formatLatency(undefined, false), { text: 'Offline', level: 'offline' });
});

test('online but not yet measured shows a placeholder, still counted as online', () => {
  assert.deepEqual(formatLatency(undefined, true), { text: '—', level: 'online' });
});

test('a low round-trip is online', () => {
  assert.deepEqual(formatLatency(12, true), { text: '12ms', level: 'online' });
  assert.deepEqual(formatLatency(SLOW_LATENCY_MS, true), { text: `${SLOW_LATENCY_MS}ms`, level: 'online' });
});

test('a round-trip past the threshold is slow', () => {
  assert.deepEqual(formatLatency(SLOW_LATENCY_MS + 1, true), { text: `${SLOW_LATENCY_MS + 1}ms`, level: 'slow' });
  assert.equal(formatLatency(800, true).level, 'slow');
});
