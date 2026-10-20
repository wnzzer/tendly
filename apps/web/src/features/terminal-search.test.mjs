import { test } from 'node:test';
import assert from 'node:assert/strict';
import { findCount, isTerminalFind, terminalClipboardKey } from './terminal-search.ts';

const key = (overrides) => ({ key: 'f', metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...overrides });

test('a Mac finds with ⌘F and leaves Ctrl+F to the shell', () => {
  assert.equal(isTerminalFind(key({ metaKey: true }), true), true);
  assert.equal(isTerminalFind(key({ ctrlKey: true }), true), false);
});

test('elsewhere Ctrl+F stays the shell\'s and Ctrl+Shift+F finds', () => {
  assert.equal(isTerminalFind(key({ ctrlKey: true }), false), false);
  assert.equal(isTerminalFind(key({ key: 'F', ctrlKey: true, shiftKey: true }), false), true);
});

test('the count says where the current match is and how many there are', () => {
  const done = { count: 12, activeIndex: 2, searching: false, limited: false };
  assert.equal(findCount('error', done, 'No matches'), '3 / 12');
  assert.equal(findCount('error', { ...done, limited: true }, 'No matches'), '3 / 12+');
  // Still scanning history: more may come.
  assert.equal(findCount('error', { ...done, searching: true }, 'No matches'), '3 / 12+');
  assert.equal(findCount('error', { count: 0, activeIndex: -1, searching: true, limited: false }, 'No matches'), '…');
  assert.equal(findCount('error', { count: 0, activeIndex: -1, searching: false, limited: false }, 'No matches'), 'No matches');
  assert.equal(findCount('  ', done, 'No matches'), '');
});

test('off a Mac the terminal copies and pastes with Shift added, or with Insert, and Ctrl+C stays the shell\'s', () => {
  const key = (key, mods = {}) => ({ key, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods });
  assert.equal(terminalClipboardKey(key('C', { ctrlKey: true, shiftKey: true }), false), 'copy');
  assert.equal(terminalClipboardKey(key('V', { ctrlKey: true, shiftKey: true }), false), 'paste');
  assert.equal(terminalClipboardKey(key('Insert', { ctrlKey: true }), false), 'copy');
  assert.equal(terminalClipboardKey(key('Insert', { shiftKey: true }), false), 'paste');
  assert.equal(terminalClipboardKey(key('c', { ctrlKey: true }), false), undefined, 'Ctrl+C interrupts the program');
  assert.equal(terminalClipboardKey(key('C', { ctrlKey: true, shiftKey: true }), true), undefined, 'a Mac has ⌘C');
});
