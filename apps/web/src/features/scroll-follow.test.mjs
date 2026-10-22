import { test } from 'node:test';
import assert from 'node:assert/strict';
import { followsAfterScroll } from './scroll-follow.ts';

test('a person scrolling up stops following at once, however little they moved', () => {
  assert.equal(followsAfterScroll({ following: true, movedUp: true, byPerson: true, gap: 25 }), false);
});

test('coming back near the bottom follows again', () => {
  assert.equal(followsAfterScroll({ following: false, movedUp: false, byPerson: true, gap: 40 }), true);
  assert.equal(followsAfterScroll({ following: false, movedUp: false, byPerson: true, gap: 400 }), false);
});

test('a move up nobody asked for changes nothing', () => {
  // Content below shrank and the browser moved the view: still following.
  assert.equal(followsAfterScroll({ following: true, movedUp: true, byPerson: false, gap: 0 }), true);
  assert.equal(followsAfterScroll({ following: false, movedUp: true, byPerson: false, gap: 500 }), false);
  // Near the bottom too: a late scroll from the wheel that stopped following does not resume it.
  assert.equal(followsAfterScroll({ following: false, movedUp: true, byPerson: false, gap: 20 }), false);
});
