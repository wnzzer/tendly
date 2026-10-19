import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { canReopenInTerminal, escapeHatchSource, liveEscapeHatch } from './terminal-reopen.ts';

const structured = { provider: 'claude_code', interaction_mode: 'structured', provider_session_id: 'thread_01' };
const enabled = { sessionTerminalEscape: true };

test('the escape hatch is offered for a structured session that has a conversation', () => {
  assert.equal(canReopenInTerminal(structured, enabled), true);
  assert.equal(canReopenInTerminal({ ...structured, provider: 'codex' }, enabled), true);
});

test('a session with no conversation yet cannot be reopened', () => {
  // The server refuses this, so a button here would only ever produce an error.
  assert.equal(canReopenInTerminal({ ...structured, provider_session_id: null }, enabled), false);
  assert.equal(canReopenInTerminal({ ...structured, provider_session_id: undefined }, enabled), false);
  assert.equal(canReopenInTerminal({ ...structured, provider_session_id: '' }, enabled), false);
});

test('only structured mode needs an escape hatch', () => {
  // A PTY session already *is* a terminal; there is nothing to escape to.
  assert.equal(canReopenInTerminal({ ...structured, interaction_mode: 'pty' }, enabled), false);
  assert.equal(canReopenInTerminal({ ...structured, interaction_mode: undefined }, enabled), false);
  assert.equal(canReopenInTerminal({ ...structured, provider: 'terminal' }, enabled), false);
});

test('a backend without the capability is never offered the action', () => {
  // An older backend has no such route, so the action must stay hidden rather
  // than fail when it is used.
  assert.equal(canReopenInTerminal(structured), false);
  assert.equal(canReopenInTerminal(structured, {}), false);
  assert.equal(canReopenInTerminal(structured, { sessionTerminalEscape: false }), false);
});

test('the request targets the session it reopens, and encodes its id', () => {
  const source = readFileSync(new URL('./terminal-reopen.ts', import.meta.url), 'utf8');
  assert.match(source, /\/sessions\/\$\{encodeURIComponent\(session\.id\)\}\/terminal/);
  assert.match(source, /json\("POST"\)/);
});

test('the new session is started explicitly, because creating one never launches anything', () => {
  const source = readFileSync(new URL('./terminal-reopen.ts', import.meta.url), 'utf8');
  // The create route deliberately launches nothing, as every other session
  // route does not. Without this the pane would open on a stopped terminal.
  assert.match(source, /\/sessions\/\$\{encodeURIComponent\(opened\.id\)\}\/start/);
});

test('the capability is read from the health response, defaulting to absent', () => {
  const source = readFileSync(new URL('./backend-capabilities.ts', import.meta.url), 'utf8');
  // A capability that defaulted to `true` for a modern api_version would turn
  // on for every backend at version 2, including ones without the route.
  assert.match(source, /sessionTerminalEscape: has\("session_terminal_escape", false\)/);
});

test('an escape hatch leads back to the conversation it reopened, and only then', () => {
  assert.equal(escapeHatchSource({ provider: 'claude_code', resume_source_id: 'source' }), 'source');
  assert.equal(escapeHatchSource({ provider: 'claude_code', resume_source_id: null }), undefined);
  // A terminal session's own shell is never a detour from a conversation.
  assert.equal(escapeHatchSource({ provider: 'terminal', resume_source_id: 'source' }), undefined);
});

test('the conversation knows when its terminal is live', () => {
  const hatch = (status, extra = {}) => ({ id: `t-${status}`, provider: 'claude_code', resume_source_id: 'source', status, ...extra });
  assert.equal(liveEscapeHatch({ id: 'source' }, [hatch('running')])?.id, 't-running');
  assert.equal(liveEscapeHatch({ id: 'source' }, [hatch('stopped'), hatch('failed')]), undefined, 'an exited terminal holds nothing');
  assert.equal(liveEscapeHatch({ id: 'other' }, [hatch('running')]), undefined);
  assert.equal(liveEscapeHatch({ id: 'source' }, [hatch('running', { provider: 'terminal' })]), undefined);
});
