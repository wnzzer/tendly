import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp } from 'vue';
import { renderToString } from 'vue/server-renderer';

let server, ChatSessionPane, AccountsDialog, capabilities, i18n, drafts;
before(async () => {
  server = await createServer({ configFile: false, root: fileURLToPath(new URL('../../', import.meta.url)), plugins: [vue()], server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', optimizeDeps: { noDiscovery: true } });
  ChatSessionPane = (await server.ssrLoadModule('/src/features/ChatSessionPane.vue')).default;
  AccountsDialog = (await server.ssrLoadModule('/src/features/AccountsDialog.vue')).default;
  capabilities = await server.ssrLoadModule('/src/features/backend-capabilities.ts');
  i18n = await server.ssrLoadModule('/src/i18n/index.ts');
  drafts = (await server.ssrLoadModule('/src/features/chat-model.ts')).chatDrafts;
});
after(async () => { await server?.close(); });

test('official quota badge can be hidden per account without hiding custom endpoint balances', async () => {
  const Badge = (await server.ssrLoadModule('/src/features/BalanceBadge.vue')).default;
  const { accountQuotas, balances } = await server.ssrLoadModule('/src/features/balance-store.ts');
  const { preferences } = await server.ssrLoadModule('/src/features/preferences.ts');
  accountQuotas.byProfile.quota = { id: 'account', status: 'signed_in', limits: { primary: { used_percent: 20 } } };
  const old = preferences.value;
  try {
    preferences.value = { ...old, hidden_account_quotas: [] };
    const visible = await render(Badge, { profileId: 'quota' });
    assert.ok(visible.includes('balance-badge'));
    preferences.value = { ...old, hidden_account_quotas: ['account'] };
    assert.ok(!(await render(Badge, { profileId: 'quota' })).includes('balance-badge'));
    balances.rows.push({ id: 'quota', reading: { value: '12', unit: { kind: 'money', currency: 'USD' }, checked_at: new Date().toISOString() } });
    assert.ok((await render(Badge, { profileId: 'quota' })).includes('balance-badge'));
  } finally { preferences.value = old; delete accountQuotas.byProfile.quota; balances.rows = balances.rows.filter(row => row.id !== 'quota'); }
});
const fixture = (changes = {}) => ({ id: 'render-fixture', workspace_id: 'workspace-fixture', provider: 'codex', title: 'Fixture session', status: 'stopped', created_at: '2026-09-09T00:00:00Z', updated_at: '2026-09-09T00:00:00Z', endpoint_profile_id: null, provider_session_id: null, error: null, interaction_mode: 'structured', ...changes });
async function render(component, props = {}) { const context = {}; const html = await renderToString(createSSRApp(component, props), context); return html + (context.teleports?.body ?? ''); }
function enable() { capabilities.setBackendCapabilities({ ok: true, api_version: 2, capabilities: ['structured_chat', 'official_accounts', 'session_configuration'] }); i18n.useI18n().setLocale('en'); }

test('real chat SFC renders a mobile composer, provider identity and endpoint control without making a request', async () => {
  enable(); const savedFetch = globalThis.fetch; let calls = 0;
  globalThis.fetch = async () => { calls++; throw Error('SSR must not call an API'); };
  try {
    const html = await render(ChatSessionPane, { session: fixture(), profiles: [{ id: 'codex-profile', name: 'Codex work', provider: 'codex' }, { id: 'claude-profile', name: 'Other client account', provider: 'claude_code' }] });
    assert.ok(html.includes('class="chat-composer"')); assert.ok(html.includes('aria-label="Message your agent"')); assert.ok(html.includes('aria-label="Send message"'));
    // Another client's profile is offered only as a move to that client, never as this session's endpoint.
    const elsewhere = html.indexOf('Switch to another client');
    assert.ok(html.includes('Codex work')); assert.ok(elsewhere > 0 && html.indexOf('Other client account') > elsewhere);
    assert.ok(html.includes('provider-glyph'), 'the existing Codex provider identity is rendered'); assert.ok(html.includes('chat-floating-menu')); assert.ok(!html.includes('Open native client view'), 'structured bridges do not pretend to be a live PTY fallback');
    assert.equal(calls, 0);
  } finally { globalThis.fetch = savedFetch; }
});

test('legacy and unsupported backends preserve the native option without enabling chat silently', async () => {
  capabilities.setBackendCapabilities({ ok: true }); i18n.useI18n().setLocale('en');
  const html = await render(ChatSessionPane, { session: fixture({ interaction_mode: 'pty' }), profiles: [] });
  assert.ok(html.includes('Structured conversation requires an updated backend.'));
  assert.ok(html.includes('Open native client view')); assert.match(html, /<textarea[^>]*disabled/);
});

test('uncertain drafts survive remounts, remain session-isolated and render HTML as escaped textarea text', async () => {
  enable(); const draft = drafts.get('draft-render');
  draft.text = '<img src=x onerror=alert(1)>'; draft.pending = { id: 'pending-render', content: draft.text, state: 'unknown' };
  const props = { session: fixture({ id: 'draft-render' }), profiles: [] };
  const first = await render(ChatSessionPane, props), second = await render(ChatSessionPane, props);
  assert.ok(first.includes('&lt;img src=x onerror=alert(1)&gt;')); assert.ok(!first.includes('<img src=x'));
  assert.ok(second.includes('Waiting for delivery confirmation.')); assert.equal(draft.pending.id, 'pending-render');
  assert.ok(!(await render(ChatSessionPane, { session: fixture({ id: 'separate-render' }), profiles: [] })).includes('onerror'));
});

test('Claude first-view copy explains headless workspace-trust behavior and retains native tool ownership', async () => {
  enable();
  const html = await render(ChatSessionPane, { session: fixture({ provider: 'claude_code' }), profiles: [] });
  assert.ok(html.includes('skips the interactive workspace-trust prompt')); assert.ok(html.includes('supported tool approvals still come from the native client'));
});

test('a prompt-free ready event still renders the welcome and trust copy, with a clear imported-history boundary',async()=>{
  enable();
  const html=await render(ChatSessionPane,{session:fixture({provider:'claude_code',native_source_id:'fixture-history'}),profiles:[],previewSnapshot:{mode:'structured',running:true,events:[{seq:1,type:'ready'}]}});
  assert.ok(html.includes('What shall we build?'));assert.ok(html.includes('skips the interactive workspace-trust prompt'));assert.ok(html.includes('earlier transcript is not yet displayed here'));
});

test('account SFC exposes capability-based absence rather than fabricated login or quota success', async () => {
  capabilities.setBackendCapabilities({ ok: true }); i18n.useI18n().setLocale('en');
  const html = await render(AccountsDialog);
  assert.ok(html.includes('Account management needs an updated backend')); assert.ok(html.includes('No login, refresh or quota result is being simulated.'));
  assert.ok(!html.includes('<progress')); assert.ok(!html.includes('Sign in with device code'));
});

test('account SFC mount-free render lists an explicit add action but never auto-signs-in or resets quota', async () => {
  enable(); const savedFetch = globalThis.fetch; let calls = 0; globalThis.fetch = async () => { calls++; throw Error('No account requests during SSR'); };
  try { const html = await render(AccountsDialog); assert.ok(html.includes('Add account')); assert.ok(html.includes('Keep your accounts in one place')); assert.equal(calls, 0); }
  finally { globalThis.fetch = savedFetch; }
});

test('existing native configuration import belongs to account management and forwards its endpoint profile', () => {
  const accounts = readFileSync(new URL('./AccountsDialog.vue', import.meta.url), 'utf8');
  assert.match(accounts, /\/accounts\/import-native/);
  assert.match(accounts, /confirmed_shared_config: true/);
  assert.match(accounts, /\/endpoint-profiles\/['"] \+ encodeURIComponent\(account\.profile_id\)/);
  const profiles = readFileSync(new URL('./ProfilesDialog.vue', import.meta.url), 'utf8');
  assert.doesNotMatch(profiles, /class="secondary-button native-import-button"/);
});

test('new first-view application copy switches between Chinese and English without translating native identifiers', async () => {
  enable(); i18n.useI18n().setLocale('zh-CN');
  const chat = await render(ChatSessionPane, { session: fixture(), profiles: [] }), accounts = await render(AccountsDialog);
  assert.ok(chat.includes('今天，一起做点什么？')); assert.ok(accounts.includes('官方账号'));
  for (const source of ['Use chat UI', 'Official accounts', 'Confirm endpoint change', 'Review the previous reset request', 'Allow once', 'Ready for your next idea']) assert.notEqual(i18n.translate(source, 'zh-CN'), source);
});

test('the native chat renderer never uses raw HTML and both new views provide mobile-sized touch targets', () => {
  const chat = readFileSync(new URL('./ChatSessionPane.vue', import.meta.url), 'utf8'), accounts = readFileSync(new URL('./AccountsDialog.vue', import.meta.url), 'utf8');
  assert.ok(!chat.includes('v-html')); assert.ok(!chat.includes('innerHTML'));
  assert.match(chat, /font-size:var\(--input-text\)/); assert.match(chat, /min-height:44px/); assert.match(chat, /safe-area-inset-bottom/);
  assert.match(accounts, /font-size:var\(--input-text\)/); assert.match(accounts, /min-height:44px/);
  assert.ok(!chat.includes('localStorage')); assert.ok(!accounts.includes('localStorage'));
});

test('explicit preview snapshot renders real safe Markdown, tool and approval components while every mutation stays disabled', async () => {
  enable(); const originalFetch = globalThis.fetch; let calls = 0;
  globalThis.fetch = async () => { calls++; throw Error('Preview cannot access a backend'); };
  try {
    const previewSnapshot = { mode: 'structured', running: true, events: [
      { type: 'ready' },
      { type: 'message', id: 'fixture-message', role: 'assistant', text: '**Real component**\n\n<script>window.fixtureAttack=true</script>\n\n[unsafe](javascript:alert)\n\n```html\n<img src=x onerror=alert(1)>\n```' },
      { type: 'tool', id: 'fixture-tool', name: 'Fixture read', status: 'completed', text: 'Fixture output only' },
      { type: 'approval', id: 'fixture-approval', title: 'Fixture approval', text: 'Fixture permission only', choices: ['accept', 'decline', 'cancel'] },
      { type: 'usage', context_tokens: 99 },
    ] };
    const html = await render(ChatSessionPane, { session: fixture(), profiles: [], previewSnapshot });
    assert.ok(html.includes('<strong>Real component</strong>')); assert.ok(html.includes('chat-pane'));
    assert.ok(html.includes('&lt;script&gt;')); assert.ok(!html.includes('<script>')); assert.ok(!html.includes('href="javascript:')); assert.ok(!html.includes('<img src=x'));
    assert.ok(html.includes('class="chat-tool"')); assert.ok(html.includes('chat-approval')); assert.ok(html.includes('99 context tokens'));
    assert.match(html, /<button[^>]*disabled[^>]*>Allow once(<kbd[^>]*>[^<]*<\/kbd>)?<\/button>/); assert.match(html, /<button[^>]*aria-label="Send message"[^>]*disabled/);
    assert.ok(!html.includes('class="chat-floating-menu"')); assert.equal(calls, 0);
    assert.doesNotMatch(html, /<textarea[^>]*disabled/, 'local typing remains possible in the preview');
  } finally { globalThis.fetch = originalFetch; }
});

test('native plan-style questions render selectable options and an explicit other answer', async () => {
  enable();
  const html = await render(ChatSessionPane, {
    session: fixture({ provider: 'claude_code' }),
    profiles: [],
    previewSnapshot: { mode: 'structured', running: true, events: [
      { type: 'ready' },
      { type: 'approval', id: 'plan-question', title: 'Choose plan mode', text: 'Select how to continue.', choices: ['accept', 'decline', 'cancel'], questions: [{ id: 'mode', header: 'Mode', question: 'How should changes be approved?', options: [{ label: 'Review each change', description: 'Ask before each edit.' }, { label: 'Allow edits', description: 'Apply edits in this turn.' }], isOther: true }] },
    ] },
  });
  assert.match(html, /role="radiogroup"/);
  assert.match(html, /<input type="radio"[^>]*value="Review each change"/);
  assert.ok(html.includes('Review each change')); assert.ok(html.includes('Allow edits')); assert.ok(html.includes('Other answer'));
  assert.match(html, /type="text"[^>]*placeholder="Other answer"/);
});

test('development preview is a separate guarded entry and default production build has no alternate HTML input', () => {
  const entry = readFileSync(new URL('../ui-preview.ts', import.meta.url), 'utf8');
  assert.ok(entry.includes('import.meta.env.DEV')); assert.ok(!entry.includes('fetch('));
  const config = readFileSync(new URL('../../vite.config.ts', import.meta.url), 'utf8');
  assert.ok(!config.includes('ui-preview.html'));
});
