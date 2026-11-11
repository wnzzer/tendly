import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lastQuickProvider, planQuickSession, quickTitle, rememberQuickProvider } from './quick-session.ts';

const profile = (overrides = {}) => ({
  id: '11111111-1111-4111-8111-111111111111', name: 'Host Claude', provider: 'claude_code',
  endpoint_url: null, model: null, permission_mode: 'native', secret_ref: null, proxy_url: null,
  effort: null, model_aliases: {}, created_at: '2026-09-15T00:00:00Z',
  native_config: { source_id: 'claude-default', config_dir: '/home/u/.claude' }, ...overrides,
});
function storage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return { getItem: key => map.get(key) ?? null, setItem: (key, value) => map.set(key, value), map };
}

test('a lone host account is created without asking, so one click is enough', () => {
  const plan = planQuickSession('claude_code', [profile()], { structuredChat: true });
  assert.equal(plan.needsDialog, false);
  assert.equal(plan.body.provider, 'claude_code');
  assert.equal(plan.body.endpoint_profile_id, '11111111-1111-4111-8111-111111111111');
  assert.equal(plan.body.interaction_mode, 'structured');
  // Nothing is marked temporary unless it was asked for.
  assert.equal('ephemeral' in plan.body, false);
});

test('several host accounts fall back to the dialog rather than picking one silently', () => {
  const profiles = [profile(), profile({ id: '22222222-2222-4222-8222-222222222222', name: 'Second Claude' })];
  const plan = planQuickSession('claude_code', profiles, {});
  assert.equal(plan.needsDialog, true);
  assert.equal(plan.body, undefined);
});

test('with nothing configured for the client the dialog opens, rather than starting a home with no sign-in', () => {
  const plan = planQuickSession('codex', [profile()], {});
  assert.equal(plan.needsDialog, true);
  assert.equal(plan.body, undefined);
});

test('a terminal never needs an account, so it is always a direct create', () => {
  // Even with ambiguous Claude accounts present, a terminal has nothing to choose.
  const profiles = [profile(), profile({ id: '22222222-2222-4222-8222-222222222222' })];
  const plan = planQuickSession('terminal', profiles, { structuredChat: true });
  assert.equal(plan.needsDialog, false);
  assert.equal(plan.body.endpoint_profile_id, null);
  // Terminal cannot use the structured chat runtime.
  assert.equal('interaction_mode' in plan.body, false);
});

test('temporary is sent only when asked for and supported by the backend', () => {
  assert.equal(planQuickSession('terminal', [], { ephemeral: true, ephemeralSupported: true }).body.ephemeral, true);
  // An old backend must not receive a field it will reject.
  assert.equal('ephemeral' in planQuickSession('terminal', [], { ephemeral: true, ephemeralSupported: false }).body, false);
  assert.equal('ephemeral' in planQuickSession('terminal', [], { ephemeralSupported: true }).body, false);
});

test('repeated quick creates stay distinguishable in the sidebar', () => {
  assert.equal(quickTitle('claude_code', false, []), 'Claude Code');
  assert.equal(quickTitle('claude_code', false, ['Claude Code']), 'Claude Code 2');
  assert.equal(quickTitle('claude_code', false, ['Claude Code', 'Claude Code 2']), 'Claude Code 3');
  // A scratch window reads as one at a glance, and numbers separately.
  assert.equal(quickTitle('codex', true, []), 'Codex · scratch');
  assert.equal(quickTitle('codex', true, ['Codex · scratch']), 'Codex · scratch 2');
  const plan = planQuickSession('terminal', [], { existingTitles: ['Terminal'] });
  assert.equal(plan.body.title, 'Terminal 2');
});

test('the plain + button remembers the last provider and says which one it will create', () => {
  const store = storage();
  assert.equal(lastQuickProvider(store), 'claude_code');
  rememberQuickProvider('codex', store);
  assert.equal(lastQuickProvider(store), 'codex');
  // A junk or unknown stored value must not produce an invalid provider.
  assert.equal(lastQuickProvider(storage({ 'tendly.quick-provider.v1': 'not-a-provider' })), 'claude_code');
  rememberQuickProvider('not-a-provider', store);
  assert.equal(lastQuickProvider(store), 'codex');
  // Denied storage keeps the choice for this session instead of throwing.
  const denied = { getItem() { throw Error('denied'); }, setItem() { throw Error('denied'); } };
  rememberQuickProvider('terminal', denied);
  assert.equal(lastQuickProvider(denied), 'terminal');
});

test('a default endpoint from Preferences settles several accounts without the dialog', () => {
  const second = '22222222-2222-4222-8222-222222222222';
  const profiles = [profile(), profile({ id: second, name: 'Second Claude' })];
  const plan = planQuickSession('claude_code', profiles, { preferredProfileId: second });
  assert.equal(plan.needsDialog, false);
  assert.equal(plan.body.endpoint_profile_id, second);
  // A preference naming a profile that is gone is ignored, not trusted.
  assert.equal(planQuickSession('claude_code', profiles, { preferredProfileId: '33333333-3333-4333-8333-333333333333' }).needsDialog, true);
});
