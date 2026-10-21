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

test('a conversation can move to every other client, under each profile and a fresh home', () => {
  const profiles = [{ id: 'work', name: 'Work', provider: 'codex', endpoint_url: 'https://gw' }, { id: 'mine', name: 'Mine', provider: 'claude_code' }];
  const choices = handoffChoices('claude_code', profiles, t);
  assert.ok(choices.every(choice => choice.provider !== 'claude_code'), 'never to the client it is already in');
  assert.deepEqual(choices.filter(choice => choice.provider === 'codex').map(choice => [choice.profileId, choice.name]), [['work', 'Work'], [null, 'Native · isolated configuration']]);
  assert.equal(choices.find(choice => choice.profileId === 'work').note, 'Custom endpoint · https://gw');
  assert.deepEqual(new Set(choices.map(choice => choice.provider)), new Set(AGENT_CLIENTS.filter(provider => provider !== 'claude_code')));
});

test('a terminal has no conversation to carry', () => {
  assert.deepEqual(handoffChoices('terminal', [], t), []);
});

test('the new session is named for where it went', () => {
  assert.equal(handoffTitle('Fix login', { provider: 'codex', profileId: null, name: '', note: '' }, t), 'Fix login · Codex');
  assert.equal(handoffTitle('Fix login', { provider: 'codex', profileId: null, name: '', note: '', branch: 'feature/x' }, t), 'Fix login · feature/x');
});
