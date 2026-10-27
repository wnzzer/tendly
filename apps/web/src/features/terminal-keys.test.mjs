import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { arrowSequence, repeatArrow, selectionPresses, isTap, ctrlSequence, shiftSequence, tabSequence, ENTER_SEQUENCE, ESCAPE_SEQUENCE, TAB_SEQUENCE, BACKTAB_SEQUENCE, FIXED_KEYS, KEY_LAYOUTS, isFixedKey, keyClient, keyTitle } from './terminal-keys.ts';
import { zhCN } from '../i18n/index.ts';

test('arrow keys use the encoding the terminal state asks for', () => {
  // Outside application cursor mode the CSI form is what a shell expects.
  assert.equal(arrowSequence('up', false), '[A');
  assert.equal(arrowSequence('down', false), '[B');
  // A full-screen program that enabled DECCKM expects the SS3 form instead.
  assert.equal(arrowSequence('up', true), 'OA');
  assert.equal(arrowSequence('down', true), 'OB');
});

test('a modified arrow uses the CSI parameter form in either cursor mode', () => {
  assert.equal(arrowSequence('left', false, { ctrl: true }), '\x1b[1;5D');
  assert.equal(arrowSequence('left', true, { ctrl: true }), '\x1b[1;5D');
  assert.equal(arrowSequence('up', true, { shift: true }), '\x1b[1;2A');
  assert.equal(arrowSequence('right', false, { shift: true, ctrl: true }), '\x1b[1;6C');
  // No modifier armed is the plain key.
  assert.equal(arrowSequence('up', true, {}), '\x1bOA');
});

test('shift turns tab into shift+tab and a typed letter into its capital', () => {
  assert.equal(tabSequence(), TAB_SEQUENCE);
  assert.equal(tabSequence({ shift: true }), BACKTAB_SEQUENCE);
  assert.equal(shiftSequence('a'), 'A');
  // A word from an input method passes through unchanged.
  assert.equal(shiftSequence('hello'), 'hello');
});

test('enter sends a carriage return, which is what accepts a prompt', () => {
  assert.equal(ENTER_SEQUENCE, '\r');
});

test('tapping a row presses the arrow that many times, in the right direction', () => {
  assert.deepEqual(selectionPresses(4, 7), { key: 'down', count: 3 });
  assert.deepEqual(selectionPresses(7, 4), { key: 'up', count: 3 });
  // Tapping the row already highlighted moves nothing.
  assert.equal(selectionPresses(5, 5), undefined);
  assert.equal(selectionPresses(0, 0), undefined);
});

test('a mistaken tap costs a bounded number of keystrokes', () => {
  // An unbounded count would let one stray tap in a program that does not keep
  // its cursor on the highlighted row send hundreds of keystrokes.
  assert.deepEqual(selectionPresses(0, 500, 40), { key: 'down', count: 40 });
  assert.deepEqual(selectionPresses(500, 0, 40), { key: 'up', count: 40 });
});

test('a non-integer row is not a selection target', () => {
  assert.equal(selectionPresses(1.5, 3), undefined);
  assert.equal(selectionPresses(1, Number.NaN), undefined);
});

test('repeating an arrow keeps the same encoding throughout', () => {
  assert.equal(repeatArrow('down', 3, false), '[B[B[B');
  assert.equal(repeatArrow('up', 2, true), 'OAOA');
  assert.equal(repeatArrow('up', 0, false), '');
  // A negative count must not throw or produce garbage.
  assert.equal(repeatArrow('up', -5, false), '');
});

test('a tap is distinguished from a scroll, a drag and a text selection', () => {
  assert.equal(isTap({ movedPx: 0, elapsedMs: 60, hasSelection: false }), true);
  assert.equal(isTap({ movedPx: 8, elapsedMs: 300, hasSelection: false }), true);
  // A scroll drag.
  assert.equal(isTap({ movedPx: 60, elapsedMs: 120, hasSelection: false }), false);
  // A long press, which is how a touch device selects text.
  assert.equal(isTap({ movedPx: 2, elapsedMs: 900, hasSelection: false }), false);
  // A fingertip that stayed still but left a selection behind.
  assert.equal(isTap({ movedPx: 1, elapsedMs: 100, hasSelection: true }), false);
});

test('the terminal surface exposes the controls and keeps them out of the scroll area', () => {
  const pane = readFileSync(new URL('../components/TerminalPane.vue', import.meta.url), 'utf8');
  const source = readFileSync(new URL('../components/TerminalKeyBar.vue', import.meta.url), 'utf8');
  // The pane shows the bar to a fingertip.
  assert.match(pane, /<TerminalKeyBar\s+v-if="touchDevice"/);
  // Without these the feature does not exist, however correct the helpers are.
  assert.match(source, /terminal-keys/);
  assert.match(source, /class="terminal-keys"/);
  for (const label of ['Arrow left', 'Arrow up', 'Arrow down', 'Arrow right', 'Enter', 'Ctrl', 'Shift', 'Paste', 'Photo or file', 'Tap a row to select it']) assert.ok(source.includes(`t('${label}')`) || source.includes(`"${label}"`), `${label} control is present`);
  // The keys themselves come from the layouts, one per program the terminal runs.
  assert.match(source, /KEY_LAYOUTS\[props\.client\]/);
  assert.match(pane, /:client="keyClient\(provider\)"/);
  // Tapping the terminal brings the keyboard up, so the bar has no key for it.
  assert.doesNotMatch(source, /t\('Keyboard'\)/);
  const styles = readFileSync(new URL('./workflows.css', import.meta.url), 'utf8');
  // Floating, not a row of the terminal: a full-screen program has been told
  // how tall the screen is, and taking a row back would misalign its redraws.
  assert.match(styles, /\.terminal-keys \{[^}]*position: absolute/);
  assert.match(styles, /\.native-terminal \{[^}]*position: relative/);
});

test('tapping a row sends arrows only while that gesture is explicitly armed', () => {
  const source = readFileSync(new URL('../components/TerminalPane.vue', import.meta.url), 'utf8');
  // A buffer-type gate was tried first and is wrong: Claude Code draws its
  // menus with Ink, in the ordinary buffer, so "alternate screen" excluded the
  // one case this exists for while a shell prompt looks identical to it. The
  // terminal cannot tell a menu from a prompt, so the gesture is armed by hand.
  // Windows checks the source out with CRLF line endings.
  const tap = source.match(/function gestureEnd\([\s\S]*?\r?\n\}\r?\n/)?.[0] ?? '';
  assert.ok(tap.includes('selectionPresses('), 'the tap handler is found');
  assert.doesNotMatch(tap, /buffer\.active\.type/);
  assert.match(source, /!tapSelect\.value/);
  assert.match(source, /selectionPresses\(/);
  const bar = readFileSync(new URL('../components/TerminalKeyBar.vue', import.meta.url), 'utf8');
  assert.ok(bar.includes("t('Tap a row to select it')"), 'the mode is switchable from the bar');
  assert.match(bar, /:aria-pressed="tapSelect"/, 'and says whether it is on');
});

test('ctrl is a one-shot modifier on what the soft keyboard types', () => {
  const source = readFileSync(new URL('../components/TerminalPane.vue', import.meta.url), 'utf8');
  // It rewrites the terminal's own input, so the soft keyboard supplies the
  // letter, and it releases after one key so it cannot stick on unnoticed.
  assert.match(source, /ctrlArmed\.value\) \{ ctrlArmed\.value = false; data = ctrlSequence\(data\)/);
  const bar = readFileSync(new URL('../components/TerminalKeyBar.vue', import.meta.url), 'utf8');
  // And the bar says whether it is armed.
  assert.match(bar, /:aria-pressed="armed\(slot\)"/);
  assert.match(bar, /slot === "ctrl" \? ctrlArmed\.value/);
});

test('paste reads the clipboard where it may and falls back to a field where it may not', () => {
  const source = readFileSync(new URL('../components/TerminalPane.vue', import.meta.url), 'utf8');
  // The Clipboard API exists only in a secure context, and Tendly is often
  // reached over plain http on a private network; there the person pastes
  // into an ordinary field, which every mobile browser supports.
  assert.match(source, /navigator\.clipboard\?\.readText/);
  assert.match(source, /class="terminal-paste"/);
  // terminal.paste honours bracketed paste mode, so a multi-line paste is not
  // run line by line as if each newline were Enter.
  const xterm = readFileSync(new URL('./terminal-view-xterm.ts', import.meta.url), 'utf8');
  assert.match(xterm, /terminal\.paste\(/);
  // wterm has no paste of its own: the view wraps it when the program asked for bracketed paste.
  const wterm = readFileSync(new URL('./terminal-view-wterm.ts', import.meta.url), 'utf8');
  assert.match(wterm, /bracketedPaste\(\) \? `\\x1b\[200~\$\{body\}\\x1b\[201~`/);
});

test('each fixed key sends what the physical key would', () => {
  const sent = Object.fromEntries(Object.entries(FIXED_KEYS).map(([id, key]) => [id, key.send.join('|')]));
  assert.deepEqual(sent, {
    esc: '\x1b', escEsc: '\x1b|\x1b', shiftTab: '\x1b[Z', tab: '\t', slash: '/', newline: '\n',
    ctrlA: '\x01', ctrlC: '\x03', ctrlD: '\x04', ctrlE: '\x05', ctrlL: '\x0c', ctrlO: '\x0f', ctrlR: '\x12', ctrlT: '\x14', ctrlZ: '\x1a',
  });
  // The control codes agree with what the one-shot Ctrl makes of the same letter.
  for (const [id, letter] of [['ctrlA', 'a'], ['ctrlC', 'c'], ['ctrlD', 'd'], ['ctrlE', 'e'], ['ctrlL', 'l'], ['ctrlO', 'o'], ['ctrlR', 'r'], ['ctrlT', 't'], ['ctrlZ', 'z'], ['newline', 'j']]) {
    assert.equal(FIXED_KEYS[id].send[0], ctrlSequence(letter), id);
  }
  // Esc twice is two presses: one write of two escapes reads as Alt+Esc.
  assert.equal(FIXED_KEYS.escEsc.send.length, 2);
});

test('the bar puts each program\'s everyday keys first, and only keys it has', () => {
  assert.equal(keyClient('claude_code'), 'claude_code');
  assert.equal(keyClient('codex'), 'codex');
  for (const other of ['terminal', 'pi', undefined, null]) assert.equal(keyClient(other), 'shell');
  // Claude Code and Codex: interrupt, change mode, cancel, commands, from their own `?` lists.
  for (const client of ['claude_code', 'codex']) {
    assert.deepEqual(KEY_LAYOUTS[client].main.slice(0, 4), ['esc', 'shiftTab', 'ctrlC', 'slash'], client);
    assert.ok(KEY_LAYOUTS[client].more.includes('newline'), `${client} has a new line that does not send`);
  }
  assert.ok(KEY_LAYOUTS.claude_code.more.includes('ctrlO'), 'Claude Code: verbose output');
  assert.ok(KEY_LAYOUTS.codex.more.includes('ctrlR'), 'Codex: search history');
  assert.deepEqual(KEY_LAYOUTS.shell.main.slice(0, 4), ['tab', 'ctrlC', 'ctrlR', 'ctrlD']);
  for (const [client, { main, more }] of Object.entries(KEY_LAYOUTS)) {
    // Upright on a phone, the first row holds at most ten keys and ⋯.
    assert.ok(main.length <= 10, `${client} main row fits a phone`);
    for (const slot of ['left', 'up', 'down', 'right', 'enter']) assert.ok(main.includes(slot), `${client} keeps ${slot}`);
    // The one-shot modifiers stay, for any combination not listed.
    assert.ok(more.includes('ctrl') && more.includes('shift'), client);
    const all = [...main, ...more];
    assert.equal(new Set(all).size, all.length, `${client} shows no key twice`);
  }
});

test('every key says what it does, in both languages, and what each client does with it', () => {
  for (const client of Object.keys(KEY_LAYOUTS)) {
    for (const slot of [...KEY_LAYOUTS[client].main, ...KEY_LAYOUTS[client].more].filter(isFixedKey)) {
      const title = keyTitle(client, slot);
      assert.ok(zhCN[title], `missing translation: ${title}`);
      const { label, translated } = FIXED_KEYS[slot];
      // Key names stay as keyboards print them; only words are translated.
      if (translated) assert.ok(zhCN[label], `missing translation: ${label}`);
      else assert.match(label, /^[\^⇧×A-Za-z0-9\/ ]+$/, `${slot} is a key name`);
    }
  }
  assert.match(keyTitle('claude_code', 'escEsc'), /clear the input/);
  assert.match(keyTitle('codex', 'escEsc'), /edit the last message/);
  assert.match(keyTitle('codex', 'ctrlT'), /transcript/);
  assert.match(keyTitle('claude_code', 'ctrlT'), /tasks/);
});
