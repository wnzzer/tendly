import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SHORTCUTS, formatChord, keyBelongsToTarget, matches, shortcutFor, PANE_SHORTCUTS, prefixKey } from './shortcuts.ts';

const key = (key, mods = {}) => ({ key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });

test('a chord means ⌘ on a Mac and Ctrl elsewhere, and is shown that way', () => {
  const palette = SHORTCUTS.find(item => item.id === 'palette').chord;
  assert.equal(matches(key('k', { metaKey: true }), palette, true), true);
  assert.equal(matches(key('k', { ctrlKey: true }), palette, true), false);
  assert.equal(matches(key('k', { ctrlKey: true }), palette, false), true);
  assert.equal(matches(key('K', { ctrlKey: true, shiftKey: true }), palette, false), false, 'an extra Shift is a different chord');
  assert.equal(formatChord(palette, true), '⌘K');
  assert.equal(formatChord(SHORTCUTS.find(item => item.id === 'new-session').chord, false), 'Ctrl+Shift+O');
});

test('every shortcut is distinct', () => {
  const seen = new Set(SHORTCUTS.map(item => formatChord(item.chord, false)));
  assert.equal(seen.size, SHORTCUTS.length);
});

test('a terminal keeps every key, and a field keeps the keys that are not chords', () => {
  // A minimal Element stand-in: only closest() is consulted.
  const element = selectors => Object.assign(Object.create(globalThis.Element?.prototype ?? Object.prototype), { closest: selector => selector.split(',').some(part => selectors.includes(part.trim())) ? {} : null });
  if (!globalThis.Element) globalThis.Element = function Element() {};
  Object.setPrototypeOf(element([]), globalThis.Element.prototype);
  const terminal = Object.assign(Object.create(globalThis.Element.prototype), { closest: selector => selector.includes('.xterm') ? {} : null });
  const field = Object.assign(Object.create(globalThis.Element.prototype), { closest: selector => selector.includes('textarea') ? {} : null });
  assert.equal(keyBelongsToTarget(terminal, { key: 'k', mod: true }), true, 'Ctrl+K is the shell’s');
  assert.equal(keyBelongsToTarget(field, { key: 'k', mod: true }), false, 'a chord reaches the app from a field');
  assert.equal(keyBelongsToTarget(field, { key: '/' }), true, 'a plain key is typing');
  assert.equal(keyBelongsToTarget(null, { key: 'k', mod: true }), false);
});


test('prefix is Ctrl+B on every platform, and double prefix passes one real key through', () => {
  assert.equal(prefixKey(key('b', { ctrlKey: true }), false), 'prefix');
  assert.equal(prefixKey(key('b', { metaKey: true }), false), undefined);
  assert.equal(prefixKey(key('b', { ctrlKey: true }), true), 'pass');
  assert.equal(prefixKey(key('b', { ctrlKey: true, repeat: true }), true), 'ignore');
});

test('prefix commands only run after prefix and shifted punctuation works', () => {
  for (const item of PANE_SHORTCUTS) {
    assert.equal(prefixKey(key(item.chord.key), false), undefined);
    assert.equal(prefixKey(key(item.chord.key), true), item.id);
    assert.equal(matches(key(item.chord.key), item.chord), false);
  }
  assert.equal(prefixKey(key('%', { shiftKey: true }), true), 'split-horizontal');
  assert.equal(prefixKey(key('Shift', { shiftKey: true }), true), undefined);
});

test('escape and unknown keys cancel, composition and browser shortcuts pass through', () => {
  assert.equal(prefixKey(key('Escape'), true), 'cancel');
  assert.equal(prefixKey(key('j'), true), 'cancel');
  assert.equal(prefixKey(key('Process', { isComposing: true }), true), 'cancel-pass');
  assert.equal(prefixKey(key('w', { metaKey: true }), true), 'cancel-pass');
  assert.equal(prefixKey(key('ArrowRight', { altKey: true, metaKey: true }), false), undefined);
});
