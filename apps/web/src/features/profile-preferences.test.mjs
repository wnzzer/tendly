import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PROFILE_CHOICE_REQUIRED, PROFILE_NONE, isProfileSelectionValid, preferredProfileSelection, rememberProfileSelection } from './profile-preferences.ts';

const CLAUDE = '561f95d2-7fa3-4be2-bb81-d2ce0a80a851';
const CLAUDE_OTHER = '97cbb3a8-6bf3-41bb-b46b-9957ce773292';
const CODEX = 'd46aa367-9b50-414c-b91f-5d65ee498513';
const CUSTOM = '5a4e1046-0f54-43ca-a82a-cea0d540d19b';
const DELETED = 'af7b448c-e998-415d-bc53-576523f330a1';
const profile = (id, provider = 'claude_code', native = true) => ({
  id, provider, name: 'Account label', endpoint_url: null, model: null, permission_mode: 'native', secret_ref: null, created_at: 'now',
  native_config: native ? { source_id: `${provider}-default`, config_dir: '/host/private-config', config_env: null } : null,
});
function storage() {
  const values = new Map(), writes = [];
  return { values, writes, getItem: key => values.get(key) ?? null, setItem(key, value) { values.set(key, value); writes.push([key, value]); } };
}

test('a sole native reference is selected by default without persisting an inferred account choice', () => {
  const store = storage();
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CUSTOM, 'claude_code', false)], store), CLAUDE);
  assert.equal(store.writes.length, 0);
  assert.equal(preferredProfileSelection('codex', [profile(CLAUDE), profile(CODEX, 'codex')], store), CODEX);
});

test('multiple native profiles require a choice rather than guessing an account', () => {
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE_OTHER)], storage()), PROFILE_CHOICE_REQUIRED);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE)], storage()), CLAUDE);
});

test('a valid remembered custom or native profile takes priority over automatic native defaults', () => {
  const store = storage(), profiles = [profile(CLAUDE), profile(CLAUDE_OTHER), profile(CUSTOM, 'claude_code', false)];
  rememberProfileSelection('claude_code', CUSTOM, store);
  assert.equal(preferredProfileSelection('claude_code', profiles, store), CUSTOM);
  rememberProfileSelection('claude_code', CLAUDE_OTHER, store);
  assert.equal(preferredProfileSelection('claude_code', profiles, store), CLAUDE_OTHER);
});

test('the blank per-session home is no longer a choice: not remembered, and an old remembered one is ignored', () => {
  const store = storage();
  rememberProfileSelection('claude_code', '', store);
  assert.equal(store.writes.length, 0);
  // What an earlier version stored for that choice.
  store.values.set('tendly.profile-selection.v1:claude_code', '__isolated_profile__');
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE)], store), CLAUDE);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE_OTHER)], store), PROFILE_CHOICE_REQUIRED);
});

test('with no host profile, a sole custom profile is used; several need a choice; none need one added', () => {
  assert.equal(preferredProfileSelection('claude_code', [profile(CUSTOM, 'claude_code', false)], storage()), CUSTOM);
  assert.equal(preferredProfileSelection('claude_code', [profile(CUSTOM, 'claude_code', false), profile(DELETED, 'claude_code', false)], storage()), PROFILE_CHOICE_REQUIRED);
  assert.equal(preferredProfileSelection('claude_code', [profile(CODEX, 'codex')], storage()), PROFILE_NONE);
  assert.equal(preferredProfileSelection('terminal', [profile(CLAUDE)], storage()), '');
});

test('deleted profile preferences fall back to one native account, explicit choice for many, or the only other profile', () => {
  const store = storage();rememberProfileSelection('claude_code', DELETED, store);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE)], store), CLAUDE);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE_OTHER)], store), PROFILE_CHOICE_REQUIRED);
  assert.equal(preferredProfileSelection('claude_code', [profile(CUSTOM, 'claude_code', false)], store), CUSTOM);
  assert.equal(preferredProfileSelection('claude_code', [], store), PROFILE_NONE);
});

test('provider preferences cannot select an account from another provider', () => {
  const store = storage(), profiles = [profile(CLAUDE), profile(CODEX, 'codex')];
  rememberProfileSelection('claude_code', CLAUDE, store);rememberProfileSelection('codex', CODEX, store);
  assert.equal(preferredProfileSelection('claude_code', profiles, store), CLAUDE);
  assert.equal(preferredProfileSelection('codex', profiles, store), CODEX);
  rememberProfileSelection('claude_code', CODEX, store);
  assert.equal(preferredProfileSelection('claude_code', profiles, store), CLAUDE);
  assert.notEqual(store.writes[0][0], store.writes[1][0]);
});

test('selection validation rejects unresolved choices, missing IDs, cross-provider IDs and no configuration for an agent', () => {
  const profiles = [profile(CLAUDE), profile(CODEX, 'codex')];
  assert.equal(isProfileSelectionValid('claude_code', PROFILE_CHOICE_REQUIRED, profiles), false);
  assert.equal(isProfileSelectionValid('claude_code', DELETED, profiles), false);
  assert.equal(isProfileSelectionValid('claude_code', CODEX, profiles), false);
  assert.equal(isProfileSelectionValid('claude_code', CLAUDE, profiles), true);
  assert.equal(isProfileSelectionValid('claude_code', '', profiles), false);
  assert.equal(isProfileSelectionValid('claude_code', PROFILE_NONE, profiles), false);
  assert.equal(isProfileSelectionValid('terminal', '', []), true);
});

test('unresolved choices, paths, labels and tokens never enter persistence or overwrite a prior choice', () => {
  const store = storage();rememberProfileSelection('claude_code', CLAUDE, store);
  for (const value of [PROFILE_CHOICE_REQUIRED, '/host/private-config', 'sk-test-not-a-profile', 'env:TENDLY_SECRET_TEST', 'Account label']) rememberProfileSelection('claude_code', value, store);
  assert.equal(store.writes.length, 1);
  assert.equal(store.writes[0][1], CLAUDE);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE)], store), CLAUDE);
});

test('failed writes retain the newest selection in memory instead of reverting to an older persisted account', () => {
  const store = storage();rememberProfileSelection('claude_code', CLAUDE, store);
  store.setItem = () => { throw Error('quota'); };
  assert.doesNotThrow(() => rememberProfileSelection('claude_code', CLAUDE_OTHER, store));
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE_OTHER)], store), CLAUDE_OTHER);
  assert.equal(preferredProfileSelection('codex', [profile(CODEX, 'codex')], store), CODEX);
});

test('denied reads and writes fall back to memory, isolated by provider and storage instance', () => {
  const denied = { getItem() { throw Error('denied'); }, setItem() { throw Error('denied'); } };
  rememberProfileSelection('claude_code', CLAUDE_OTHER, denied);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE_OTHER)], denied), CLAUDE_OTHER);
  assert.equal(preferredProfileSelection('codex', [profile(CODEX, 'codex')], denied), CODEX);
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE), profile(CLAUDE_OTHER)], storage()), PROFILE_CHOICE_REQUIRED);
});

test('malformed persisted values are ignored, and an installation with nothing configured asks for a profile', () => {
  const store = { getItem: () => '{unexpected secret/path}', setItem() {} };
  assert.equal(preferredProfileSelection('claude_code', [profile(CLAUDE)], store), CLAUDE);
  assert.equal(preferredProfileSelection('codex', [], store), PROFILE_NONE);
});
