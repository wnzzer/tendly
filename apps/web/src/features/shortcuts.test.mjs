import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CHORDS, SHORTCUTS, PREFIX_COMMANDS, isRemove, isRename, formatChord, keyBelongsToTarget, matches, platformShortcuts, prefixCommand, prefixKey } from './shortcuts.ts';

const key = (key, mods = {}) => ({ key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });

test('a chord means ⌘ on a Mac and Ctrl elsewhere, and is shown that way', () => {
  const palette = SHORTCUTS.find(item => item.id === 'palette').chord;
  assert.equal(matches(key('k', { metaKey: true }), palette, true), true);
  assert.equal(matches(key('k', { ctrlKey: true }), palette, true), false);
  assert.equal(matches(key('k', { ctrlKey: true }), palette, false), true);
  assert.equal(matches(key('K', { ctrlKey: true, shiftKey: true }), palette, false), false, 'an extra Shift is a different chord');
  assert.equal(formatChord(palette, true), '⌘K');
  assert.equal(formatChord(SHORTCUTS.find(item => item.id === 'settings').chord, false), 'Ctrl+,');
  assert.equal(formatChord(CHORDS.remove, true), '⌘⌫');
  assert.equal(formatChord(CHORDS.confirm, false), 'Ctrl+Enter');
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
  for (const command of PREFIX_COMMANDS) for (const name of command.keys) {
    assert.equal(prefixKey(key(name), false), undefined);
    assert.equal(prefixKey(key(name), true), prefixCommand(name).id);
  }
  assert.equal(prefixKey(key('%', { shiftKey: true }), true), 'split-horizontal');
  assert.equal(prefixKey(key('|', { shiftKey: true }), true), 'split-horizontal');
  assert.equal(prefixKey(key('-'), true), 'split-vertical');
  assert.equal(prefixKey(key('Shift', { shiftKey: true }), true), undefined);
});

test('every prefix key does one thing, and a key names its variant', () => {
  const keys = PREFIX_COMMANDS.flatMap(command => command.keys);
  assert.equal(new Set(keys).size, keys.length);
  assert.equal(prefixCommand('ArrowLeft').id, 'focus-left');
  assert.equal(prefixCommand('j').id, 'focus-down');
  assert.equal(prefixCommand('L').id, 'resize-right');
  assert.equal(prefixCommand('3').id, 'tab-3');
  assert.equal(prefixCommand('C').id, 'new-session', 'shares the id, and so the action, of the global chord');
});

test('a repeatable key keeps the prefix for itself, and anything else is typed as usual', () => {
  assert.equal(prefixKey(key('H', { shiftKey: true }), true, true), 'resize-left');
  assert.equal(prefixKey(key('K', { shiftKey: true, repeat: true }), true, true), 'resize-up', 'holding the key keeps moving');
  assert.equal(prefixKey(key('a'), true, true), 'cancel-pass');
  assert.equal(prefixKey(key('z'), true, true), 'cancel-pass', 'only the repeatable keys');
  assert.equal(prefixKey(key('Escape'), true, true), 'cancel', 'Esc is not passed on to interrupt a turn');
  assert.equal(prefixKey(key('b', { ctrlKey: true }), true, true), 'prefix', 'Ctrl+B starts a new prefix, as in tmux, rather than reaching the terminal');
});

test('Ctrl+B is the prefix everywhere, so the sidebar chord exists only where ⌘ is not Ctrl', () => {
  assert.ok(platformShortcuts(true).some(item => item.id === 'sidebar'));
  assert.ok(!platformShortcuts(false).some(item => item.id === 'sidebar'));
  assert.ok(PREFIX_COMMANDS.some(item => item.id === 'sidebar'), 'still one key away');
});

test('escape and unknown keys cancel, composition and browser shortcuts pass through', () => {
  assert.equal(prefixKey(key('Escape'), true), 'cancel');
  assert.equal(prefixKey(key('y'), true), 'cancel');
  assert.equal(prefixKey(key('Process', { isComposing: true }), true), 'cancel-pass');
  assert.equal(prefixKey(key('w', { metaKey: true }), true), 'cancel-pass');
  assert.equal(prefixKey(key('ArrowRight', { altKey: true, metaKey: true }), false), undefined);
});

test('the app keeps no chord a browser keeps for itself', () => {
  // ⌘⇧ letters open bookmarks, profiles, the home page and devtools panes in one browser or another.
  for (const item of SHORTCUTS) assert.ok(!(item.chord.mod && item.chord.shift), `${item.id} is a mod+Shift chord`);
});

test('both layers share their keys, and a capital is more of the lower case', () => {
  const prefix = id => PREFIX_COMMANDS.find(item => item.id === id).keys;
  assert.deepEqual(prefix('palette'), [':']);
  assert.deepEqual(prefix('help'), ['?']);
  assert.deepEqual(prefix('settings'), [',']);
  assert.deepEqual([prefix('focus-sidebar'), prefix('sidebar')], [['b'], ['B']]);
  assert.deepEqual([prefix('focus-files'), prefix('files')], [['e'], ['E']]);
  assert.deepEqual([prefix('choose-session'), prefix('sessions')], [['s'], ['S']]);
  assert.deepEqual([prefix('choose-view'), prefix('canvas')], [['w'], ['W']]);
  assert.deepEqual([prefix('new-here'), prefix('new-session')], [['c'], ['C']]);
});

test('on a page only the keys that go somewhere answer', () => {
  const pages = PREFIX_COMMANDS.filter(item => item.pages).map(item => item.id);
  for (const id of ['palette', 'help', 'settings', 'sessions', 'usage', 'system', 'canvas', 'choose-session', 'sidebar']) assert.ok(pages.includes(id), id);
  for (const id of ['split-horizontal', 'close-pane', 'resize', 'next-tab', 'focus-input']) assert.ok(!pages.includes(id), id);
});

test('F2 renames and Delete, or ⌘⌫ on a Mac, removes', () => {
  assert.equal(isRename(key('F2')), true);
  assert.equal(isRename(key('F2', { shiftKey: true })), false);
  assert.equal(isRemove(key('Delete'), false), true);
  assert.equal(isRemove(key('Backspace', { metaKey: true }), true), true);
  assert.equal(isRemove(key('Backspace'), true), false, 'a plain Backspace is typing');
  assert.equal(isRemove(key('Backspace', { ctrlKey: true }), false), true);
});
