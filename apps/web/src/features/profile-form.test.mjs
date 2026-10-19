import { test } from 'node:test';
import assert from 'node:assert/strict';
import { keyHeader, keyHeaderProblem, keyMode, profilePayload, profileSummary } from './profile-form.ts';
import { secretStatus } from './secret-names.ts';

const form = (overrides = {}) => ({ name: ' Work ', provider: 'codex', endpoint_url: ' ', api: '', model: ' gpt-5 ', effort: '', permission_mode: 'native', secret_ref: '', proxy_url: '', ...overrides });

test('a save trims text and sends an empty field as null', () => {
  assert.deepEqual(profilePayload(form(), undefined, {}, [], false), {
    name: 'Work', provider: 'codex', endpoint_url: null, model: 'gpt-5', permission_mode: 'native', secret_ref: null,
  });
});

test('an optional setting is sent when set, or when the profile had it, so it can be cleared', () => {
  const set = profilePayload(form({ proxy_url: 'http://p:1', effort: 'high' }), undefined, { fast: 'gpt-5-mini' }, ['gpt-5'], false);
  assert.equal(set.proxy_url, 'http://p:1');
  assert.equal(set.effort, 'high');
  assert.deepEqual(set.model_aliases, { fast: 'gpt-5-mini' });
  assert.deepEqual(set.models, ['gpt-5']);
  // Cleared on a profile that had them: sent empty, so the server clears them.
  const cleared = profilePayload(form(), { proxy_url: 'http://p:1', effort: 'high', model_aliases: { a: 'b' }, models: ['x'] }, {}, [], false);
  assert.equal(cleared.proxy_url, null);
  assert.equal(cleared.effort, null);
  assert.deepEqual(cleared.model_aliases, {});
  assert.deepEqual(cleared.models, []);
  // Never had them: not sent at all.
  const never = profilePayload(form(), {}, {}, [], false);
  for (const key of ['proxy_url', 'effort', 'model_aliases', 'models', 'api']) assert.equal(key in never, false, key);
});

test('only a client with a choice of API is sent one', () => {
  assert.equal(profilePayload(form({ api: 'responses' }), undefined, {}, [], true).api, 'responses');
  assert.equal('api' in profilePayload(form({ api: 'responses' }), undefined, {}, [], false), false, 'switched to a client without the choice');
  assert.equal(profilePayload(form(), { api: 'chat' }, {}, [], true).api, null, 'cleared');
});

test('a profile reads as its model, host and proxy, or whose sign-in it uses', () => {
  const t = text => text;
  assert.equal(profileSummary({ model: 'opus', endpoint_url: 'https://api.example.com/v1', proxy_url: 'http://p:1' }, t), 'opus · api.example.com · Proxy configured');
  assert.equal(profileSummary({}, t), 'Native client default · Official endpoint');
  assert.equal(profileSummary({ endpoint_url: 'not a url' }, t), 'Native client default · not a url');
});

test('a key is stored, adoptable from the server, or shadowed by it', () => {
  const tendly = { name: 'TENDLY_SECRET_OPENAI', source: 'tendly' };
  const server = { name: 'TENDLY_SECRET_OPENAI', source: 'environment' };
  assert.deepEqual(secretStatus('env:TENDLY_SECRET_OPENAI', [tendly]), { stored: 'TENDLY_SECRET_OPENAI', shadowed: false });
  assert.deepEqual(secretStatus('env:TENDLY_SECRET_OPENAI', [server]), { adoptable: 'TENDLY_SECRET_OPENAI', shadowed: false });
  assert.deepEqual(secretStatus('env:TENDLY_SECRET_OPENAI', [{ ...server, stored: true }]), { stored: 'TENDLY_SECRET_OPENAI', shadowed: true });
  assert.deepEqual(secretStatus('env:TENDLY_SECRET_OPENAI', []), { shadowed: false });
  assert.deepEqual(secretStatus('', [tendly]), { shadowed: false });
});

test('a key header is a preset or a header of the endpoint\'s, and goes back to the form as one', () => {
  assert.deepEqual(keyHeader({ key_mode: 'bearer' }), { name: 'Authorization', value: 'Bearer {key}' });
  assert.deepEqual(keyHeader({ key_mode: 'custom', key_name: ' X-Token ', key_value: ' Token {key} ' }), { name: 'X-Token', value: 'Token {key}' });
  assert.equal(keyHeader({ key_mode: '' }), null);
  assert.deepEqual(keyMode({ name: 'authorization', value: 'Bearer {key}' }), { key_mode: 'bearer', key_name: '', key_value: '' });
  assert.deepEqual(keyMode({ name: 'X-API-Key', value: '{key}' }).key_mode, 'x-api-key');
  assert.deepEqual(keyMode({ name: 'Authorization', value: 'Token {key}' }), { key_mode: 'custom', key_name: 'Authorization', key_value: 'Token {key}' });
  assert.deepEqual(keyMode(null).key_mode, '');
  assert.equal(keyHeaderProblem({ key_mode: 'custom', key_name: 'X-Token', key_value: '{key}' }), '');
  for (const [name, value] of [['X Token', '{key}'], ['X-Token', 'no key'], ['X-Token', '{key}{key}'], ['X-Token', '${key}'], ['X-Token', '!{key}']]) assert.notEqual(keyHeaderProblem({ key_mode: 'custom', key_name: name, key_value: value }), '', `${name}: ${value}`);
});

test('a key header is sent when set, and cleared as null where the profile had one', () => {
  assert.deepEqual(profilePayload(form({ key_mode: 'x-api-key' }), undefined, {}, [], false).key_header, { name: 'x-api-key', value: '{key}' });
  assert.equal(profilePayload(form(), { key_header: { name: 'x-api-key', value: '{key}' } }, {}, [], false).key_header, null);
  assert.equal('key_header' in profilePayload(form(), {}, {}, [], false), false);
});
