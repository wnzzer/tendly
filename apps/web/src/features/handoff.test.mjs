import test from 'node:test';
import assert from 'node:assert/strict';
import { handoffChoices, handoffTitle, profileNote } from './handoff.ts';
import { AGENT_CLIENTS } from './clients.ts';

const t = (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key]));

test('a profile says what choosing it means', () => {
  assert.equal(profileNote({ native_config: { config_dir: '~/.codex' } }, t), 'Uses the sign-in and settings in ~/.codex');
  assert.equal(profileNote({ endpoint_url: 'https://gateway.example' }, t), 'Custom endpoint · https://gateway.example');
  assert.equal(profileNote({}, t), 'Custom endpoint profile');
});

test('a conversation can move to every other client, under each of its profiles and nothing else', () => {
  const profiles = [{ id: 'work', name: 'Work', provider: 'codex', endpoint_url: 'https://gw' }, { id: 'mine', name: 'Mine', provider: 'claude_code' }];
  const choices = handoffChoices('claude_code', profiles, t);
  assert.ok(choices.every(choice => choice.provider !== 'claude_code'), 'never to the client it is already in');
  // No blank home with no sign-in is offered.
  assert.deepEqual(choices.map(choice => [choice.provider, choice.profileId, choice.name]), [['codex', 'work', 'Work']]);
  assert.equal(choices[0].note, 'Custom endpoint · https://gw');
  // A client with no profile has nowhere to start the conversation.
  assert.deepEqual(handoffChoices('codex', profiles, t).map(choice => choice.profileId), ['mine']);
});

test('a terminal has no conversation to carry', () => {
  assert.deepEqual(handoffChoices('terminal', [], t), []);
});

test('the new session is named for where it went', () => {
  assert.equal(handoffTitle('Fix login', { provider: 'codex', profileId: null, name: '', note: '' }, t), 'Fix login · Codex');
  assert.equal(handoffTitle('Fix login', { provider: 'codex', profileId: null, name: '', note: '', branch: 'feature/x' }, t), 'Fix login · feature/x');
});
