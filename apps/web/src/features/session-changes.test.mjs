import { test } from 'node:test';
import assert from 'node:assert/strict';
import { asksBeforeDiscard, deletableSessions, isAlreadyGone, opensOnArrival, replaceSession } from './session-changes.ts';
import { ApiConnectionError, ApiError } from './api.ts';

const session = (id, extra = {}) => ({ id, title: id, provider: 'claude_code', status: 'idle', interaction_mode: 'structured', ...extra });

test('a structured chat and a terminal start as their pane is placed; a legacy PTY waits to be asked', () => {
  assert.equal(opensOnArrival(session('chat')), true);
  assert.equal(opensOnArrival(session('term', { provider: 'terminal', interaction_mode: 'terminal' })), true);
  assert.equal(opensOnArrival(session('pty', { interaction_mode: 'terminal' })), false);
});

test('only a 404 from the server counts as already deleted', () => {
  assert.equal(isAlreadyGone(new ApiError('gone', 404)), true);
  assert.equal(isAlreadyGone(new ApiError('nope', 500)), false);
  assert.equal(isAlreadyGone(new ApiConnectionError('DELETE')), false, 'an unreachable host says nothing about the session');
  assert.equal(isAlreadyGone(new Error('x')), false);
});

test('a delete goes ahead only for temporary and archived sessions not already being deleted', () => {
  const temporary = session('t', { ephemeral: true }), archived = session('a', { archived_at: '2026-01-01T00:00:00Z' }), listed = session('l');
  assert.deepEqual(deletableSessions([temporary, archived, listed], []).map(item => item.id), ['t', 'a']);
  assert.deepEqual(deletableSessions([temporary, archived], ['t']).map(item => item.id), ['a']);
});

test('closing a temporary window asks first only while its conversation is mid-turn', () => {
  for (const status of ['running', 'starting', 'waiting']) assert.equal(asksBeforeDiscard(session('s', { status })), true, status);
  // A client stays up between turns: what the view last held says whether one is open.
  const turn = status => ({ type: 'turn', status, seq: 2 });
  assert.equal(asksBeforeDiscard(session('s', { status: 'running' }), [{ type: 'ready', seq: 1 }, turn('completed')]), false, 'idle between turns');
  assert.equal(asksBeforeDiscard(session('s', { status: 'running' }), [{ type: 'ready', seq: 1 }, turn('running')]), true, 'mid-turn');
  assert.equal(asksBeforeDiscard(session('s', { status: 'running' }), [{ type: 'ready', seq: 1 }, { type: 'approval', id: 'a', title: 'Run?', text: 'ls', choices: ['accept'], seq: 2 }]), true, 'a question is open');
  assert.equal(asksBeforeDiscard(session('s', { status: 'idle' })), false);
  assert.equal(asksBeforeDiscard(session('s', { status: 'running', provider: 'terminal', interaction_mode: 'terminal' })), false, 'a terminal is running for as long as it is open');
});

test('the server\'s copy of a session replaces only that one', () => {
  const list = [session('a'), session('b')];
  const next = replaceSession(list, session('b', { title: 'renamed' }));
  assert.deepEqual(next.map(item => item.title), ['a', 'renamed']);
  assert.equal(next[0], list[0]);
  assert.deepEqual(list.map(item => item.title), ['a', 'b'], 'the list passed in is left as it was');
});
