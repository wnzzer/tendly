import { test } from 'node:test';
import { acknowledgeReceipt } from './chat-model.ts';
import assert from 'node:assert/strict';
import { acknowledgeDraft, appendChatEvent, approvalPayloadAnswers, canClearContext, conversationView, createChatDraftStore, isChatEvent, parseConversationSnapshot, pendingMessageRetry, pruneChatEvents, sessionConfigurationPayload } from './chat-model.ts';
import { markdownBlocks, markdownInline, safeWebUrl } from './markdown-model.ts';

const session = { id: 'session', provider: 'codex', status: 'stopped' };
const profiles = [{ id: 'codex-work', provider: 'codex' }, { id: 'claude-work', provider: 'claude_code' }];

test('a durable message receipt acknowledges a trimmed event without replay or losing a newer draft',()=>{
  const draft={text:'newer edit',pending:{id:'request',content:'old message',text:'old message',state:'unknown'}};
  assert.equal(acknowledgeReceipt(draft,{accepted:true,id:'other'}),false);
  assert.equal(acknowledgeReceipt(draft,{accepted:true,id:'request',duplicate:true}),true);
  assert.equal(draft.text,'newer edit');assert.equal(draft.pending,undefined);
});
test('an uncertain request cannot follow a changed configuration revision even with the same profile ID',()=>{
  const draft={text:'keep',pending:{id:'request',content:'keep',state:'unknown',profileId:'codex-work',configurationRevision:1}};
  assert.throws(()=>pendingMessageRetry(draft,{endpoint_profile_id:'codex-work',configuration_revision:2}),/endpoint changed/);
  assert.deepEqual(pendingMessageRetry(draft,{endpoint_profile_id:'codex-work',configuration_revision:1}),{id:'request',content:'keep'});
});

test('native messages and deltas update a stable message without duplicating replayed sequences', () => {
  const events = [
    { seq: 1, type: 'message', id: 'u', role: 'user', text: 'Build it' },
    { seq: 2, type: 'message', id: 'a', role: 'assistant', text: 'Hello' },
    { seq: 3, type: 'message', id: 'a', role: 'assistant', text: ' world', delta: true },
    { seq: 3, type: 'message', id: 'a', role: 'assistant', text: ' world', delta: true },
  ];
  const view = conversationView(events);
  assert.equal(view.items.length, 2); assert.equal(view.items[1].text, 'Hello world');
  assert.equal(appendChatEvent(events, events[2]).length, 4);
  assert.equal(appendChatEvent(events, { seq: 4, type: 'turn', status: 'completed' }).length, 5);
  assert.equal(events.length, 4);
});

test('bounded history retains old pending approvals and latest control anchors in sequence order', () => {
  const controls = [{ seq: 1, type: 'configuration', id: 'config', profile_name: 'Profile', text: 'Boundary' }, { seq: 2, type: 'ready' }, { seq: 3, type: 'turn', status: 'running' }, { seq: 4, type: 'usage', context_tokens: 123 }, { seq: 5, type: 'approval', id: 'pending', title: 'Keep me', text: 'Native approval', choices: ['accept'] }];
  const history = Array.from({ length: 2005 }, (_, index) => ({ seq: 6 + index, type: 'message', id: String(index), role: 'assistant', text: 'fixture' }));
  const bounded = pruneChatEvents([...controls, ...history], 2000);
  assert.equal(bounded.truncated, true); assert.equal(bounded.events.length, 2005);
  assert.deepEqual(bounded.events.slice(0, 5), controls); assert.equal(conversationView(bounded.events, true).awaitingApproval, true); assert.equal(conversationView(bounded.events, true).turn, 'running');
  assert.equal(conversationView(bounded.events, true).usage.context_tokens, 123);
  const resolved = pruneChatEvents([...bounded.events, { seq: 2011, type: 'approval_resolved', id: 'pending' }], 2000);
  assert.ok(!resolved.events.some(event => event.type === 'approval')); assert.equal(conversationView(resolved.events, true).awaitingApproval, false);
  assert.ok(resolved.events.some(event => event.type === 'turn' && event.status === 'running'));
});

test('event byte budget excludes bounded safety anchors but cannot retain unbounded native approvals', () => {
  const approval = { seq: 1, type: 'approval', id: 'safety', title: 'Keep', text: 'x'.repeat(500), choices: ['decline'] };
  const bounded = pruneChatEvents([approval, { seq: 2, type: 'message', id: 'old', role: 'assistant', text: 'x'.repeat(200) }, { seq: 3, type: 'message', id: 'last', role: 'assistant', text: 'tail' }], 2000, 120);
  assert.equal(bounded.events[0], approval); assert.ok(bounded.historyBytes <= 120); assert.ok(bounded.anchorBytes > 500);
  assert.throws(() => pruneChatEvents(Array.from({ length: 33 }, (_, i) => ({ ...approval, id: String(i), seq: i }))), /Too many/);
  assert.throws(() => pruneChatEvents([{ ...approval, text: 'x'.repeat(193 * 1024) }]), /control event/);
  const ended = pruneChatEvents([approval, { seq: 2, type: 'turn', status: 'running' }, { seq: 3, type: 'exit' }, { seq: 4, type: 'message', id: 'last', role: 'assistant', text: 'tail' }], 1, 120);
  assert.equal(conversationView(ended.events).awaitingApproval, false); assert.equal(conversationView(ended.events).turn, 'interrupted');
});

test('tools, approvals, usage and turn status are projections of official events only', () => {
  const events = [
    { seq: 1, type: 'ready', native_session_id: 'native-thread' },
    { seq: 2, type: 'turn', status: 'running' },
    { seq: 3, type: 'tool', id: '__proto__', name: 'read_file', status: 'running', text: 'fixture output' },
    { seq: 4, type: 'tool', id: '__proto__', name: 'read_file', status: 'completed' },
    { seq: 5, type: 'approval', id: 'permission', title: 'Confirm', text: 'Write file?', choices: ['accept', 'decline'], questions: [] },
  ];
  const before = conversationView(events);
  assert.equal(before.ready, true); assert.equal(before.turn, 'running'); assert.equal(before.awaitingApproval, true);
  assert.deepEqual(before.items[0], { ...events[3], text: 'fixture output' });
  const after = conversationView([...events, { seq: 6, type: 'approval_resolved', id: 'permission' }, { seq: 7, type: 'turn', status: 'completed' }, { seq: 8, type: 'usage', input_tokens: 123, context_tokens: 456 }]);
  assert.equal(after.awaitingApproval, false); assert.equal(after.turn, 'completed'); assert.equal(after.usage.context_tokens, 456);
  assert.equal(before.awaitingApproval, true, 'new snapshots do not mutate earlier rendered projections');
  assert.equal('resolved' in events[4], false, 'provider event is not rewritten');
});

test('endpoint boundary retains old display history without making it a new-context prompt', () => {
  const view = conversationView([{ type: 'ready' }, { type: 'message', id: 'old', role: 'user', text: 'old confidential context' }, { type: 'configuration', id: 'change', profile_name: 'New account', text: 'New context' }]);
  assert.equal(view.items[0].text, 'old confidential context'); assert.equal(view.items[1].type, 'configuration'); assert.equal(view.ready, false);
  assert.deepEqual(sessionConfigurationPayload(session, profiles, 'codex-work', true), { endpoint_profile_id: 'codex-work', confirmed: true });
});

test('the command list is only claimed to be known once a client has announced one', () => {
  // Nothing heard yet: the list is empty because the client has not spoken.
  const fresh = conversationView([]);
  assert.deepEqual(fresh.commands, []); assert.equal(fresh.commandsAnnounced, false);
  // A client that announces none is known to have none.
  assert.equal(conversationView([{ type: 'ready' }]).commandsAnnounced, true);
  const advertised = conversationView([{ type: 'ready', commands: ['model', 'usage'] }]);
  assert.deepEqual(advertised.commands, ['model', 'usage']); assert.equal(advertised.commandsAnnounced, true);
  // A new endpoint is a new client, so the previous list stops being an answer
  // about this one until it announces its own.
  const switched = conversationView([{ type: 'ready', commands: ['model'] }, { type: 'configuration', id: 'change', profile_name: 'Other account', text: 'New context' }]);
  assert.deepEqual(switched.commands, []); assert.equal(switched.commandsAnnounced, false);
  // Exit does not invalidate what the client said: it runs the same commands
  // when it starts again.
  const exited = conversationView([{ type: 'ready', commands: ['model'] }, { type: 'exit' }]);
  assert.deepEqual(exited.commands, ['model']); assert.equal(exited.commandsAnnounced, true);
});

test('a model announcement never disturbs the conversation, and a new endpoint clears it', () => {
  const history = [
    { type: 'ready', commands: ['model'] },
    { type: 'settings', models: [{ id: 'opus', name: 'Opus', efforts: ['low', 'high'] }, { id: 'haiku', name: 'Haiku' }] },
    { type: 'message', id: 'u1', role: 'user', text: 'keep me' },
    { type: 'settings', model: 'haiku' },
  ];
  const view = conversationView(history);
  // Switching model is not a context boundary: the transcript is untouched and
  // the client is still ready, unlike an endpoint change.
  assert.equal(view.items.length, 1);
  assert.equal(view.items[0].text, 'keep me');
  assert.equal(view.ready, true);
  assert.equal(view.model, 'haiku');
  assert.deepEqual(view.models.map(entry => entry.id), ['opus', 'haiku']);
  // A model that advertises no levels genuinely has none.
  assert.equal(view.models[1].efforts, undefined);

  // A new endpoint is a different client, so its models are unknown again.
  const switched = conversationView([...history, { type: 'configuration', id: 'c', profile_name: 'Other', text: 'New context' }]);
  assert.deepEqual(switched.models, []);
  assert.equal(switched.model, undefined);
});

test('pruning keeps the model list even when a later settings event only names a selection', () => {
  const pruned = pruneChatEvents([
    { seq: 1, type: 'settings', models: [{ id: 'opus', name: 'Opus' }] },
    ...Array.from({ length: 30 }, (_, index) => ({ seq: index + 2, type: 'message', id: 'm' + index, role: 'user', text: 'x' })),
    { seq: 40, type: 'settings', model: 'opus' },
  ], 5);
  const view = conversationView(pruned.events);
  assert.deepEqual(view.models.map(entry => entry.id), ['opus'], 'the list must survive a selection-only update');
  assert.equal(view.model, 'opus');
});

test('a client that clears its own context clears the transcript with it', () => {
  const view = conversationView([
    { seq: 1, type: 'ready', native_session_id: 'first' },
    { seq: 2, type: 'message', id: 'u1', role: 'user', text: 'earlier work' },
    { seq: 3, type: 'usage', context_tokens: 148581, context_window: 1000000 },
    { seq: 4, type: 'cleared' },
    { seq: 5, type: 'ready', native_session_id: 'second' },
    { seq: 6, type: 'message', id: 'u2', role: 'user', text: 'after clearing' },
  ]);
  // Keeping the old transcript showed a conversation the client can no longer
  // refer to, which is what made /clear look like it had done nothing.
  assert.deepEqual(view.items.map(item => item.text), ['after clearing']);
  assert.equal(view.usage.context_tokens, undefined);
  assert.equal(view.ready, true);

  // The boundary has to survive trimming, or the cleared messages come back.
  const pruned = pruneChatEvents([
    { seq: 1, type: 'message', id: 'old', role: 'user', text: 'earlier work' },
    { seq: 2, type: 'cleared' },
    ...Array.from({ length: 30 }, (_, index) => ({ seq: index + 3, type: 'message', id: 'm' + index, role: 'user', text: 'x' })),
  ], 5);
  assert.ok(pruned.events.some(event => event.type === 'cleared'), 'the clear boundary is an anchor');
  assert.equal(conversationView(pruned.events).items.some(item => item.text === 'earlier work'), false);
});

test('fresh native contexts may reuse message and tool IDs without replacing earlier display history', () => {
  const view = conversationView([{ type: 'message', id: '1', role: 'assistant', text: 'Old provider text' }, { type: 'tool', id: '1', name: 'old_tool', status: 'completed' }, { type: 'configuration', id: 'boundary', profile_name: 'New endpoint', text: 'New native context' }, { type: 'message', id: '1', role: 'assistant', text: 'New provider text' }, { type: 'tool', id: '1', name: 'new_tool', status: 'running' }]);
  assert.equal(view.items.length, 5); assert.equal(view.items[0].text, 'Old provider text'); assert.equal(view.items[1].name, 'old_tool'); assert.equal(view.items[3].text, 'New provider text'); assert.equal(view.items[4].name, 'new_tool');
});

test('stopped runtime snapshot invalidates stale approvals and running turns without sending any response', () => {
  const events = [{ type: 'ready' }, { type: 'turn', status: 'running' }, { type: 'approval', id: 'lost-rpc', title: 'Old request', text: 'Old', choices: ['accept'] }];
  const view = conversationView(events, false);
  assert.equal(view.ready, false); assert.equal(view.turn, 'interrupted'); assert.equal(view.awaitingApproval, false); assert.equal(view.items[0].resolved, true);
  assert.equal('resolved' in events[2], false, 'display invalidation is not an approval sent to the official client');
});

test('exit interrupts visual running state without claiming that a new agent was started', () => {
  const view = conversationView([{ type: 'ready' }, { type: 'turn', status: 'running' }, { type: 'exit' }]);
  assert.equal(view.ready, false); assert.equal(view.exited, true); assert.equal(view.turn, 'interrupted');
});

test('snapshot parsing preserves legacy missing mode and refuses malformed native events', () => {
  assert.deepEqual(parseConversationSnapshot({ running: false, events: [] }), { mode: undefined, running: false, events: [], truncated: false });
  assert.equal(parseConversationSnapshot({ mode: 'pty', running: true, events: [], truncated: true }).mode, 'pty');
  for (const value of [null, {}, { mode: 'unknown', running: false, events: [] }, { running: 'yes', events: [] }, { running: false, events: [{ type: 'message', id: 'x', role: 'system', text: 'bad' }] }]) assert.throws(() => parseConversationSnapshot(value), /Invalid conversation/);
  assert.equal(isChatEvent({ seq: -1, type: 'exit' }), false);
  assert.equal(isChatEvent({ type: 'usage', input_tokens: NaN }), false);
  assert.equal(isChatEvent({ type: 'approval', id: 'a', title: 'x', text: 'y', choices: ['accept'], questions: [{ id: 'q', question: 'Choose', options: [{ label: 'A' }] }] }), true);
  assert.equal(isChatEvent({ type: 'approval', id: 'a', title: 'x', text: 'y', choices: ['accept'], questions: [{ id: 'q', question: 'Choose', options: [null] }] }), false);
});

test('native question responses retain multiple selections, support other input and keep secret answers single-valued', () => {
  const questions = [{ id: 'multi', question: 'Targets', options: [{ label: 'Web' }, { label: 'API' }], multiSelect: true, isOther: true }, { id: 'single', question: 'One', options: [{ label: 'A' }, { label: 'B' }] }, { id: 'secret', question: 'Value', options: [], isSecret: true, multiSelect: true }];
  assert.deepEqual(approvalPayloadAnswers(questions, { multi: ['Web', 'API'], single: ['B', 'A'], secret: ['synthetic-value', 'discard-extra'] }, { multi: 'Mobile' }), { multi: ['Web', 'API', 'Mobile'], single: ['B'], secret: ['synthetic-value'] });
  assert.deepEqual(approvalPayloadAnswers([{ id: 'fixed', question: 'Only listed', options: [{ label: 'A' }] }], { fixed: ['invented'] }), {});
  assert.deepEqual(approvalPayloadAnswers(questions, {}), {});
});

test('native question IDs are safe data properties and unsupported multiSelect representations fail closed', () => {
  const questions = [{ id: '__proto__', question: 'Fixture', options: [], multiSelect: true }];
  const answers = approvalPayloadAnswers(questions, Object.fromEntries([['__proto__', ['A', 'B']]]));
  assert.equal(Object.hasOwn(answers, '__proto__'), true); assert.deepEqual(answers.__proto__, ['A', 'B']);
  assert.equal(isChatEvent({ type: 'approval', id: 'a', title: 'q', text: '', choices: ['accept'], questions: [{ ...questions[0], multiSelect: 'yes' }] }), false);
});

test('drafts are isolated in memory by session and unknown delivery never clears itself', () => {
  const store = createChatDraftStore(), one = store.get('one'), two = store.get('two');
  one.text = 'private draft'; one.pending = { id: 'request-one', content: one.text, text: one.text, state: 'unknown' };
  assert.equal(two.text, ''); assert.equal(two.pending, undefined); assert.equal(store.get('one'), one);
  assert.equal(acknowledgeDraft(one, []), false); assert.equal(one.pending.state, 'unknown');
  assert.equal(acknowledgeDraft(one, [{ type: 'message', id: 'other', role: 'user', text: one.text }]), false);
  assert.equal(acknowledgeDraft(one, [{ type: 'message', id: 'request-one', role: 'assistant', text: one.text }]), false);
  assert.equal(acknowledgeDraft(one, [{ type: 'message', id: 'request-one', role: 'user', text: one.text }]), true);
  assert.equal(one.text, ''); assert.equal(one.pending, undefined);
  assert.equal(store.get('__proto__').text, '');
});

test('delivery acknowledgement does not discard a subsequently edited draft', () => {
  const draft = { text: 'new text', pending: { id: 'old', content: 'old text', text: 'old text', state: 'unknown' } };
  acknowledgeDraft(draft, [{ type: 'message', id: 'old', role: 'user', text: 'old text' }]);
  assert.equal(draft.text, 'new text'); assert.equal(draft.pending, undefined);
});

test('an explicit unknown-delivery retry keeps both the original UUID and content, and refuses a changed account', () => {
  const draft = { text: 'new edit', pending: { id: 'same-request-uuid', content: 'original request', state: 'unknown', profileId: 'original-profile' } };
  assert.deepEqual(pendingMessageRetry(draft, { endpoint_profile_id: 'original-profile' }), { id: 'same-request-uuid', content: 'original request' });
  assert.throws(() => pendingMessageRetry(draft, { endpoint_profile_id: 'other-profile' }), /another account/);
  assert.equal(draft.pending.state, 'unknown'); assert.equal(draft.text, 'new edit');
  assert.throws(() => pendingMessageRetry({ text: '' }, { endpoint_profile_id: null }), /no uncertain/);
});

test('configuration requires explicit confirmation and same provider, allowing only stopped or ready idle structured sessions', () => {
  assert.throws(() => sessionConfigurationPayload(session, profiles, 'codex-work', false), /Confirm/);
  assert.throws(() => sessionConfigurationPayload(session, profiles, 'claude-work', true), /same client/);
  assert.throws(() => sessionConfigurationPayload(session, profiles, 'missing', true), /same client/);
  assert.deepEqual(sessionConfigurationPayload(session, profiles, null, true), { endpoint_profile_id: null, confirmed: true });
  const live = { ...session, status: 'running' }, idle = { mode: 'structured', ready: true, busy: false, awaitingApproval: false };
  assert.throws(() => sessionConfigurationPayload(live, profiles, null, true), /Wait/);
  assert.throws(() => sessionConfigurationPayload(live, profiles, null, true, { ...idle, mode: 'pty' }), /Wait/);
  assert.throws(() => sessionConfigurationPayload(live, profiles, null, true, { ...idle, busy: true }), /Wait/);
  assert.throws(() => sessionConfigurationPayload(live, profiles, null, true, { ...idle, awaitingApproval: true }), /Wait/);
  assert.deepEqual(sessionConfigurationPayload(live, profiles, null, true, idle), { endpoint_profile_id: null, confirmed: true });
});

test('Markdown is a small text AST: headings, code, lists and emphasis never become raw HTML', () => {
  const blocks = markdownBlocks('# Title\n\nA **bold** thought with `code`.\n\n```ts\n<script>alert(1)</script>\n```\n\n- first\n- second\n\n> quote');
  assert.equal(blocks[0].type, 'heading'); assert.equal(blocks[0].level, 1);
  assert.ok(markdownInline(blocks[1].text).some(part => part.type === 'strong' && part.text === 'bold'));
  assert.deepEqual(blocks[2], { type: 'code', language: 'ts', text: '<script>alert(1)</script>' });
  assert.deepEqual(blocks[3], { type: 'list', ordered: false, items: ['first', 'second'] });
  assert.equal(blocks[4].type, 'quote');
  assert.deepEqual(markdownBlocks('<img src=x onerror=alert(1)>'), [{ type: 'paragraph', text: '<img src=x onerror=alert(1)>' }]);
});

test('Markdown links allow explicit credential-free HTTP(S), not scripts, data, native files or automatic images', () => {
  for (const url of ['javascript:alert(1)', 'data:text/html,test', 'file:///etc/passwd', 'https://user:pass@example.invalid', '/api/action']) assert.equal(safeWebUrl(url), undefined);
  assert.equal(safeWebUrl('https://example.invalid/docs'), 'https://example.invalid/docs');
  assert.ok(markdownInline('[unsafe](javascript:bad)').every(part => part.type !== 'link'));
  assert.deepEqual(markdownInline('[docs](https://example.invalid/docs)'), [{ type: 'link', text: 'docs', href: 'https://example.invalid/docs' }]);
  assert.ok(markdownInline('![image](https://example.invalid/pixel.png)').every(part => part.type !== 'image'));
});

test('model and thinking depth are session-only overrides, and never fight a shared host configuration', () => {
  // Omitted fields keep whatever the profile provides, so an unchanged control
  // does not pin the profile's current value onto the session.
  assert.deepEqual(sessionConfigurationPayload(session, profiles, 'codex-work', true, undefined, {}),
    { endpoint_profile_id: 'codex-work', confirmed: true });
  assert.deepEqual(sessionConfigurationPayload(session, profiles, 'codex-work', true, undefined, { effort: 'high' }),
    { endpoint_profile_id: 'codex-work', effort: 'high', confirmed: true });
  assert.deepEqual(sessionConfigurationPayload(session, profiles, 'codex-work', true, undefined, { model: 'gpt-x', effort: 'max' }),
    { endpoint_profile_id: 'codex-work', model: 'gpt-x', effort: 'max', confirmed: true });
  // Whitespace is not a value.
  assert.deepEqual(sessionConfigurationPayload(session, profiles, 'codex-work', true, undefined, { model: '  ', effort: ' ' }),
    { endpoint_profile_id: 'codex-work', confirmed: true });

  // A shared native configuration may still take a session override: it is a
  // launch flag for this session and never rewrites the client's own settings.
  const shared = [{ id: 'host-codex', provider: 'codex', native_config: { source_id: 'codex-default', config_dir: '/home/u/.codex' } }];
  assert.deepEqual(sessionConfigurationPayload(session, shared, 'host-codex', true), { endpoint_profile_id: 'host-codex', confirmed: true });
  assert.deepEqual(sessionConfigurationPayload(session, shared, 'host-codex', true, undefined, { effort: 'high' }),
    { endpoint_profile_id: 'host-codex', effort: 'high', confirmed: true });

  // The existing guards still apply before any override is considered.
  assert.throws(() => sessionConfigurationPayload(session, profiles, 'codex-work', false, undefined, { effort: 'high' }), /Confirm/);
  assert.throws(() => sessionConfigurationPayload({ ...session, status: 'running' }, profiles, 'codex-work', true, undefined, { effort: 'high' }), /Wait for/);
  assert.throws(() => sessionConfigurationPayload(session, profiles, 'claude-work', true, undefined, { effort: 'high' }), /same client/);
});

test('a sent message leaves the composer, even when the wire content was reshaped', () => {
  // Completing a command from the menu leaves a trailing space, and composing
  // trims it. Comparing the composer against the wire content therefore never
  // matched, so the composer kept holding a command it had already sent.
  const command = { text: '/model ', pending: { id: 'm1', content: '/model', text: '/model ', state: 'sending' } };
  assert.equal(acknowledgeDraft(command, [{ type: 'message', id: 'm1', role: 'user', text: '/model' }]), true);
  assert.equal(command.text, '');

  // An attachment header is added to the wire content and was never in the
  // composer at all.
  const attached = { text: 'look at this', pending: { id: 'm2', content: 'Attached file in this workspace:\n- a.png\n\nlook at this', text: 'look at this', state: 'sending' } };
  assert.equal(acknowledgeReceipt(attached, { accepted: true, id: 'm2' }), true);
  assert.equal(attached.text, '');

  // Still only the sent message is cleared.
  const edited = { text: 'a newer thought', pending: { id: 'm3', content: '/model', text: '/model ', state: 'sending' } };
  acknowledgeReceipt(edited, { accepted: true, id: 'm3' });
  assert.equal(edited.text, 'a newer thought');
});

test('clearing is offered whenever the client can do it, including with an empty composer', () => {
  const live = { preview: false, running: true, ready: true, busy: false, connected: true, commands: ['clear', 'model'] };
  // Nothing here depends on the composer. A first attempt gated this on "a
  // message could be sent", which needs typed text, so the action vanished
  // exactly when someone reaches for it.
  assert.equal(canClearContext(live), true);
  // Only the client can drop its own context, so it must have said it can.
  assert.equal(canClearContext({ ...live, commands: ['model'] }), false);
  assert.equal(canClearContext({ ...live, commands: [] }), false);
  // A client that is not there, not ready, mid-turn or disconnected cannot.
  assert.equal(canClearContext({ ...live, running: false }), false);
  assert.equal(canClearContext({ ...live, ready: false }), false);
  assert.equal(canClearContext({ ...live, busy: true }), false);
  assert.equal(canClearContext({ ...live, connected: false }), false);
  assert.equal(canClearContext({ ...live, preview: true }), false);
});

test('subagent progress joins its parent tool card without displacing the tool output', () => {
  const view = conversationView([
    { type: 'tool', id: 't1', name: 'Task', status: 'running', text: '{"prompt":"find the bug"}' },
    { type: 'message', id: 'a1', role: 'assistant', text: 'Delegating.' },
    { type: 'tool', id: 't1', name: 'Task', status: 'running', activity: 'Reading chat-model.ts' },
    { type: 'tool', id: 't1', name: 'Task', status: 'running', activity: '→ Grep' },
    { type: 'tool', id: 't1', name: 'Task', status: 'completed', text: 'Found it in conversationView.' },
  ]);
  const card = view.items.find(item => item.id === 't1');
  // The reason the bridge used to drop subagent output was that emitting it as
  // a message overwrote the main reply. It must still stand untouched.
  assert.equal(view.items.filter(item => item.type === 'message').length, 1);
  assert.equal(view.items.find(item => item.type === 'message').text, 'Delegating.');
  // Progress accumulates, and the tool's own result is not replaced by it.
  assert.equal(card.activity, 'Reading chat-model.ts\n→ Grep');
  assert.equal(card.text, 'Found it in conversationView.');
  assert.equal(card.status, 'completed');
});

test('a long-running subagent cannot grow a card without bound', () => {
  const events = [{ type: 'tool', id: 't1', name: 'Task', status: 'running', text: '{}' }];
  for (let i = 0; i < 400; i += 1) events.push({ type: 'tool', id: 't1', name: 'Task', status: 'running', activity: `step ${i} `.repeat(20) });
  const card = conversationView(events).items.find(item => item.id === 't1');
  assert.ok(card.activity.length <= 4100, `activity grew to ${card.activity.length}`);
  // Truncation keeps the newest work, which is what a running card is for.
  assert.ok(card.activity.includes('step 399'), 'the latest step must survive truncation');
  assert.ok(!card.activity.includes('step 0 '), 'the oldest step should have been dropped');
  assert.ok(card.activity.startsWith('…'), 'truncation must be visible');
});

test('an event without activity leaves existing progress alone', () => {
  const card = conversationView([
    { type: 'tool', id: 't1', name: 'Task', status: 'running', activity: 'searching' },
    { type: 'tool', id: 't1', name: 'Task', status: 'completed', text: 'done' },
  ]).items.find(item => item.id === 't1');
  assert.equal(card.activity, 'searching');
  assert.equal(card.text, 'done');
});

test('how tools are approved is the client\'s own reported state, and a new endpoint knows none of it', () => {
  const history = [
    { seq: 1, type: 'ready', commands: ['model'] },
    { seq: 2, type: 'settings', models: [{ id: 'opus', name: 'Opus' }], permission_mode: 'ask', permission_modes: ['ask', 'plan', 'accept_edits', 'danger'] },
    { seq: 3, type: 'message', id: 'u1', role: 'user', text: 'keep me' },
    { seq: 4, type: 'settings', models: [{ id: 'opus', name: 'Opus' }], permission_mode: 'danger', permission_modes: ['ask', 'plan', 'accept_edits', 'danger'] },
  ];
  const view = conversationView(history);
  // Switching how tools are approved is not a context boundary either: the
  // transcript stands and the client stays ready.
  assert.equal(view.items.length, 1);
  assert.equal(view.ready, true);
  assert.equal(view.permissionMode, 'danger');
  assert.deepEqual(view.permissionModes, ['ask', 'plan', 'accept_edits', 'danger']);
  // The set is the client's answer, so a client offering fewer modes offers
  // fewer: Codex has no plan or accept-edits to switch into.
  const codex = conversationView([{ seq: 1, type: 'settings', permission_mode: 'ask', permission_modes: ['ask', 'danger'] }]);
  assert.deepEqual(codex.permissionModes, ['ask', 'danger']);
  // A different endpoint is a different client, so its modes are unknown until
  // it says, exactly as its models are.
  const switched = conversationView([...history, { seq: 5, type: 'configuration', id: 'c', profile_name: 'Other', text: 'New context' }]);
  assert.equal(switched.permissionMode, undefined);
  assert.deepEqual(switched.permissionModes, []);
});

test('an unattended mode survives pruning, and an invented one never reaches the view', () => {
  const pruned = pruneChatEvents([
    { seq: 1, type: 'settings', models: [{ id: 'opus', name: 'Opus' }], permission_mode: 'danger', permission_modes: ['ask', 'danger'] },
    ...Array.from({ length: 30 }, (_, index) => ({ seq: index + 2, type: 'message', id: 'm' + index, role: 'user', text: 'x' })),
  ], 5);
  assert.equal(conversationView(pruned.events).permissionMode, 'danger');
  assert.equal(isChatEvent({ type: 'settings', permission_mode: 'danger' }), true);
  assert.equal(isChatEvent({ type: 'settings', permission_mode: 7 }), false);
  assert.equal(isChatEvent({ type: 'settings', permission_modes: ['ask', 'Danger Mode'] }), false);
  assert.equal(isChatEvent({ type: 'settings', permission_modes: Array.from({ length: 9 }, () => 'ask') }), false);
});

test('images resolve inside the workspace only, relative to the file that names them', async () => {
  const { markdownInline, workspaceImagePath, attachedImagePaths } = await import('./markdown-model.ts');
  assert.deepEqual(markdownInline('see ![shot](img/a.png) now')[1], { type: 'image', alt: 'shot', src: 'img/a.png' });
  assert.equal(markdownInline('![up](../shared/b.png)')[0].type, 'image');
  assert.equal(workspaceImagePath('img/a.png', 'docs'), 'docs/img/a.png');
  assert.equal(workspaceImagePath('../assets/b.webp', 'docs/guide'), 'docs/assets/b.webp');
  assert.equal(workspaceImagePath('/root.png', 'docs'), 'root.png');
  assert.equal(workspaceImagePath('../../escape.png', 'docs'), undefined);
  assert.equal(workspaceImagePath('https://tracker.example/pixel.png'), undefined);
  assert.equal(workspaceImagePath('//cdn.example/x.png'), undefined);
  assert.equal(workspaceImagePath('notes.txt'), undefined);
  assert.deepEqual(attachedImagePaths('Attached files in this workspace:\n- shots/one.png\n- notes.txt\n\nLook at this'), ['shots/one.png']);
  assert.deepEqual(attachedImagePaths('No header here\n- shots/one.png'), []);
});

test('GFM tables become table blocks, with alignment, code pipes and ragged rows handled', async () => {
  const { markdownBlocks } = await import('./markdown-model.ts');
  const blocks = markdownBlocks('Intro line\n| | Claude | Codex |\n|---|:---:|---:|\n| Start | `a | b` | `app-server` |\n| Short |\n\nAfter');
  assert.deepEqual(blocks.map(b => b.type), ['paragraph', 'table', 'paragraph']);
  const table = blocks[1];
  assert.deepEqual(table.header, ['', 'Claude', 'Codex']);
  assert.deepEqual(table.align, [undefined, 'center', 'right']);
  assert.deepEqual(table.rows, [['Start', '`a | b`', '`app-server`'], ['Short', '', '']]);
  // Pipes without a divider row stay prose.
  assert.equal(markdownBlocks('a | b\nc | d')[0].type, 'paragraph');
});

test('file references are found where agents write them, and ordinary words are not', async () => {
  const { fileReference, markdownInline } = await import('./markdown-model.ts');
  assert.deepEqual(fileReference('crates/server/src/main.rs:42'), { path: 'crates/server/src/main.rs', line: 42 });
  assert.deepEqual(fileReference('chat-claude.mjs:17'), { path: 'chat-claude.mjs', line: 17 });
  assert.deepEqual(fileReference('src/App.vue#L12'), { path: 'src/App.vue', line: 12 });
  assert.deepEqual(fileReference('/Users/kl/repo/a.ts:3:9'), { path: '/Users/kl/repo/a.ts', line: 3 });
  assert.deepEqual(fileReference('file:///Users/kl/repo/README.md'), { path: '/Users/kl/repo/README.md' });
  // A Windows host: drive paths and `\` separators come back in `/` form.
  assert.deepEqual(fileReference('C:\\Users\\kl\\repo\\a.ts:3'), { path: 'C:/Users/kl/repo/a.ts', line: 3 });
  assert.deepEqual(fileReference('file:///C:/Users/kl/repo/README.md'), { path: 'C:/Users/kl/repo/README.md' });
  assert.deepEqual(fileReference('src\\main.rs:7'), { path: 'src/main.rs', line: 7 });
  assert.equal(fileReference('\\\\server\\share\\a.ts'), undefined, 'a UNC share is not a workspace file');
  for (const word of ['set_model', '1.2.3', 'v0.1.20', 'turn/start', 'e.g.', 'https://example.com/a.js']) assert.equal(fileReference(word), undefined, word);
  // Inline code, markdown links and prose paths with a directory all link;
  // a bare name in a sentence stays text.
  assert.deepEqual(markdownInline('see `chat-common.mjs:53` now').find(p => p.type === 'file'), { type: 'file', text: 'chat-common.mjs:53', path: 'chat-common.mjs', line: 53, code: true });
  assert.equal(markdownInline('[the entry](src/main.ts#L4)')[0].type, 'file');
  assert.deepEqual(markdownInline('edit apps/web/src/App.vue:12, then').filter(p => p.type === 'file').map(p => [p.path, p.line]), [['apps/web/src/App.vue', 12]]);
  assert.ok(markdownInline('open main.rs please').every(p => p.type === 'text'));
  assert.deepEqual(markdownInline('docs at https://example.com/x. Done').find(p => p.type === 'link'), { type: 'link', text: 'https://example.com/x', href: 'https://example.com/x' });
  assert.ok(markdownInline('`set_model`').every(p => p.type === 'code'));
});

test('a card remembers when it started and when its duration was last reported', () => {
  const tool = (extra) => ({ type: 'tool', id: 'agent', name: 'Agent', status: 'running', ...extra });
  const view = conversationView([
    tool({ text: '{}' }),
    tool({ at: 100, stats: { ms: 0 } }),
    tool({ at: 900, stats: { tools: 1, ms: 700 } }),
    // Activity alone moves `at` but not the time the duration was measured.
    tool({ at: 2_000, activity: 'Reading' }),
  ]);
  const [agent] = view.items;
  assert.equal(agent.since, 100);
  assert.equal(agent.ms_at, 900);
  assert.equal(agent.at, 2_000);
});

test('a subagent\'s calls hang under it, as deep as they went, and keep its counts', () => {
  const tool = (id, extra = {}) => ({ type: 'tool', id, name: extra.name ?? 'Read', status: 'running', ...extra });
  const view = conversationView([
    tool('agent-1', { name: 'Agent', text: '{}' }),
    tool('agent-2', { name: 'Agent', parent: 'agent-1' }),
    tool('read-1', { parent: 'agent-2' }),
    { type: 'message', id: 'm', role: 'assistant', text: 'Waiting on it.' },
    tool('read-1', { status: 'completed', text: 'contents', parent: 'agent-2' }),
    tool('agent-1', { name: 'Agent', stats: { tools: 3, tokens: 1200 }, at: 1 }),
    // A later event without counts keeps the ones already known.
    tool('agent-1', { name: 'Agent', status: 'completed', text: 'done' }),
    // A parent trimmed from history: its call is shown at the top, not lost.
    tool('orphan', { parent: 'gone' }),
  ]);
  assert.deepEqual(view.items.map(item => item.id), ['agent-1', 'm', 'orphan']);
  const [agent] = view.items;
  assert.deepEqual(agent.stats, { tools: 3, tokens: 1200 });
  assert.equal(agent.text, 'done');
  assert.equal(agent.children[0].id, 'agent-2');
  assert.equal(agent.children[0].children[0].status, 'completed');
  assert.equal(agent.children[0].children[0].text, 'contents');
});

test('nested, counted and timed tool events are well-formed only with sane values', () => {
  const base = { type: 'tool', id: 't', name: 'Read', status: 'running' };
  assert.ok(isChatEvent({ ...base, parent: 'a', at: 5, stats: { tokens: 1, tools: 2, ms: 3 } }));
  assert.ok(!isChatEvent({ ...base, parent: '' }));
  assert.ok(!isChatEvent({ ...base, stats: { cost: 1 } }));
  assert.ok(!isChatEvent({ ...base, at: 'now' }));
});

test('a subagent keeps the description it was started with after its input is replaced', () => {
  const view = conversationView([
    { type: 'tool', id: 'a', name: 'Agent', status: 'running', text: '{"description":"Scan the bridge","prompt":"…"}' },
    { type: 'tool', id: 'a', name: 'Agent', status: 'completed', text: 'Found three.' },
    // A result that happens to be JSON is never read as a description.
    { type: 'tool', id: 'b', name: 'Read', status: 'completed', text: '{"description":"from the result"}' },
  ]);
  assert.equal(view.items[0].label, 'Scan the bridge');
  assert.equal(view.items[0].text, 'Found three.');
  assert.equal(view.items[1].label, undefined);
});

test('every call is summed up from what its input already says', async () => {
  const { toolLabel } = await import('./chat-model.ts');
  assert.equal(toolLabel('{"command":"npm test","description":"Run the tests"}'), 'Run the tests');
  assert.equal(toolLabel('{"command":"ls -la\\nrm x"}'), 'ls -la');
  assert.equal(toolLabel('{"file_path":"/home/u/project/src/a.ts"}'), 'src/a.ts');
  assert.equal(toolLabel('{"pattern":"TODO","path":"/repo/src"}'), 'TODO · repo/src');
  assert.equal(toolLabel('{"url":"https://example.com/a"}'), 'example.com/a');
  assert.equal(toolLabel('{}'), undefined);
  assert.equal(toolLabel('not json'), undefined);
});

test('a call still running when its client exits is ended, not running for ever', () => {
  const view = conversationView([
    { type: 'tool', id: 'agent', name: 'Agent', status: 'running', text: '{}' },
    { type: 'tool', id: 'inner', name: 'Bash', status: 'running', parent: 'agent' },
    { type: 'tool', id: 'done', name: 'Read', status: 'completed', text: 'x' },
    { type: 'exit' },
    { type: 'ready' },
  ]);
  const [agent, done] = view.items;
  assert.equal(agent.status, 'running');
  assert.equal(agent.ended, true);
  assert.equal(agent.children[0].ended, true);
  assert.equal(done.ended, undefined);
});

test('a turn\'s cost sits under that turn\'s last reply, and never moves the context meter', () => {
  const spent = { input: 82200, output: 900, cached: 81000 };
  const view = conversationView([
    { type: 'message', id: 'u1', role: 'user', text: 'one' },
    { type: 'turn', id: 'u1', status: 'running' },
    { type: 'message', id: 'a1', role: 'assistant', text: 'first' },
    { type: 'tool', id: 't', name: 'Read', status: 'completed', text: 'x', result_tokens: 12000 },
    { type: 'message', id: 'a2', role: 'assistant', text: 'second' },
    { type: 'usage', context_tokens: 40500, context_window: 200000 },
    { type: 'usage', turn: spent },
    { type: 'turn', id: 'u1', status: 'completed' },
    // The next turn has no reply yet: its cost has nowhere of its own to go.
    { type: 'message', id: 'u2', role: 'user', text: 'two' },
    { type: 'turn', id: 'u2', status: 'running' },
    { type: 'usage', turn: { input: 5, output: 5 } },
  ]);
  const byId = Object.fromEntries(view.items.map(item => [item.id, item]));
  assert.deepEqual(byId.a2.usage, spent);
  assert.equal(byId.a1.usage, undefined);
  assert.equal(byId.u2.usage, undefined);
  assert.equal(byId.t.result_tokens, 12000);
  assert.deepEqual(view.usage, { context_tokens: 40500, context_window: 200000 });
  // A turn's cost is history, not a control kept through every trim.
  const { events } = pruneChatEvents([{ type: 'usage', context_tokens: 1 }, { type: 'usage', turn: spent }], 0);
  assert.deepEqual(events, [{ type: 'usage', context_tokens: 1 }]);
  assert.ok(isChatEvent({ type: 'usage', turn: spent }));
  assert.ok(!isChatEvent({ type: 'usage', turn: { cost: 1 } }));
});

test('message usage stays with its own Pi reply inside a multi-reply turn', () => {
  const view = conversationView([
    { type: 'turn', id: 'u', status: 'running' },
    { type: 'message', id: 'a1', role: 'assistant', text: 'first' },
    { type: 'usage', message_id: 'a1', turn: { input: 42, output: 4, cached: 30 } },
    { type: 'message', id: 'a2', role: 'assistant', text: 'second' },
    { type: 'usage', message_id: 'a2', turn: { input: 50, output: 6, cached: 40 } },
  ]);
  assert.deepEqual(view.items.filter(item => item.type === 'message').map(item => item.usage), [
    { input: 42, output: 4, cached: 30 },
    { input: 50, output: 6, cached: 40 },
  ]);
});

test('a conversation carried from another client opens, including one stored before its boundary was named', () => {
  const text = 'Conversation carried from another client. The next message includes the earlier conversation.';
  for (const boundary of [{ type: 'configuration', id: 'c', text, seq: 1 }, { type: 'configuration', id: 'c', profile_name: 'Pi 2', text, seq: 1 }]) {
    const snapshot = parseConversationSnapshot({ mode: 'structured', running: true, events: [boundary, { type: 'ready', seq: 2 }] });
    assert.equal(conversationView(snapshot.events).items[0].type, 'configuration');
  }
  assert.equal(isChatEvent({ type: 'configuration', id: 'c', profile_name: 7, text }), false);
});

test('a steer the turn ended without reading opens the next turn, unmarked', () => {
  const view = conversationView([
    { type: 'message', id: 'first', role: 'user', text: 'hold', seq: 1 },
    { type: 'turn', id: 'first', status: 'running', seq: 2 },
    { type: 'message', id: 'late', role: 'user', text: 'late', steered: true, seq: 3 },
    { type: 'message', id: 'reply-1', role: 'assistant', text: 'done', seq: 4 },
    { type: 'turn', id: 'first', status: 'completed', seq: 5 },
    { type: 'turn', id: 'late', status: 'running', seq: 6 },
    { type: 'message', id: 'reply-2', role: 'assistant', text: 'read it', seq: 7 },
  ]);
  assert.deepEqual(view.items.map(item => [item.id, !!item.steered]), [['first', false], ['reply-1', false], ['late', false], ['reply-2', false]]);
});

test('a resumed greeting says where it resumes, and a view resumes from its last event', async () => {
  const { lastChatSeq } = await import('./chat-model.ts');
  const resumed = parseConversationSnapshot({ running: true, events: [{ type: 'ready', seq: 8 }], after: 7 });
  assert.equal(resumed.after, 7);
  assert.ok(!('after' in parseConversationSnapshot({ running: true, events: [] })));
  for (const after of [-1, 1.5, '7', null]) assert.throws(() => parseConversationSnapshot({ running: true, events: [], after }), /Invalid conversation response/);
  assert.equal(lastChatSeq([]), undefined);
  assert.equal(lastChatSeq([{ type: 'ready' }]), undefined);
  assert.equal(lastChatSeq([{ type: 'ready', seq: 3 }, { type: 'turn', id: 't', status: 'running', seq: 9 }, { type: 'ready' }]), 9);
  // Adding a resumed greeting keeps what the view held and skips anything it already had.
  const held = [{ type: 'ready', seq: 6 }, { type: 'ready', seq: 7 }];
  const merged = [{ type: 'ready', seq: 7 }, { type: 'ready', seq: 8 }].reduce(appendChatEvent, held);
  assert.deepEqual(merged.map(event => event.seq), [6, 7, 8]);
});

test('a finished message keeps its place but not its pieces, as the server stores it', () => {
  const deltas = ['Hel', 'lo'].map((text, i) => ({ type: 'message', id: 'a', role: 'assistant', text, delta: true, seq: 2 + i }));
  let events = [{ type: 'message', id: 'u', role: 'user', text: 'hi', seq: 1 }, deltas[0], { type: 'tool', id: 't', name: 'Bash', status: 'running', text: 'ls', seq: 3 }, { ...deltas[1], seq: 4 }];
  events = appendChatEvent(events, { type: 'message', id: 'a', role: 'assistant', text: 'Hello', seq: 5 });
  assert.deepEqual(events.map(e => [e.seq, e.type, e.text]), [[1, 'message', 'hi'], [2, 'message', ''], [3, 'tool', 'ls'], [5, 'message', 'Hello']]);
  // The message reads whole, where it began.
  const view = conversationView(events);
  assert.deepEqual(view.items.map(i => i.type === 'message' ? i.text : i.type), ['hi', 'Hello', 'tool']);
});

test('keeping a long history bounded does not re-serialize it for every streamed piece', () => {
  let events = Array.from({ length: 5000 }, (_, i) => ({ type: 'tool', id: `t${i}`, name: 'Bash', status: 'completed', text: 'x'.repeat(200), seq: i + 1 }));
  events = pruneChatEvents(events).events;
  const stringify = JSON.stringify; let calls = 0;
  JSON.stringify = (...args) => { calls++; return stringify(...args); };
  try {
    for (let i = 0; i < 100; i++) events = pruneChatEvents(appendChatEvent(events, { type: 'message', id: 'm', role: 'assistant', text: 'x', delta: true, seq: 6000 + i })).events;
  } finally { JSON.stringify = stringify; }
  assert.ok(calls <= 200, `${calls} serializations for 100 pieces`);
  assert.equal(events.length, 5100, 'a long history is kept, not cut at 2,000');
});

test('a snapshot carries the cursor to older history, and older pages go in front once', async () => {
  const { parseConversationPage, prependChatEvents } = await import('./chat-model.ts');
  assert.equal(parseConversationSnapshot({ running: false, events: [], older: 42 }).older, 42);
  assert.ok(!('older' in parseConversationSnapshot({ running: false, events: [], older: null })));
  assert.throws(() => parseConversationSnapshot({ running: false, events: [], older: 0 }), /Invalid/);
  const page = parseConversationPage({ type: 'page', events: [{ type: 'ready', seq: 1 }, { type: 'ready', seq: 2 }], older: null });
  assert.deepEqual(page, { events: [{ type: 'ready', seq: 1 }, { type: 'ready', seq: 2 }] });
  assert.throws(() => parseConversationPage({ type: 'snapshot', events: [] }), /Invalid/);
  // The latest page carries an old settings event already: it is not repeated.
  const merged = prependChatEvents(page.events, [{ type: 'ready', seq: 2 }, { type: 'ready', seq: 3 }]);
  assert.deepEqual(merged.map(e => e.seq), [1, 2, 3]);
});

test('history let go from the front says where reading older history resumes', () => {
  const events = Array.from({ length: 10 }, (_, i) => ({ type: 'tool', id: `t${i}`, name: 'Bash', status: 'completed', seq: i + 1 }));
  const bounded = pruneChatEvents(events, 4);
  assert.deepEqual(bounded.events.map(e => e.seq), [7, 8, 9, 10]);
  assert.equal(bounded.dropped, 6);
  assert.ok(!('dropped' in pruneChatEvents(events)));
});

test('rules, underlined headings and multi-line quotes read as a document does', async () => {
  const { markdownBlocks } = await import('./markdown-model.ts');
  assert.deepEqual(markdownBlocks('Title\n=====\n\nPart\n---\n\n***\n\n> one\n> two'), [
    { type: 'heading', level: 1, text: 'Title' },
    { type: 'heading', level: 2, text: 'Part' },
    { type: 'rule' },
    { type: 'quote', text: 'one\ntwo' },
  ]);
  assert.deepEqual(markdownBlocks('## Closed ##'), [{ type: 'heading', level: 2, text: 'Closed' }]);
});

test('HTML blocks are read as such only when asked, and a message keeps them as text', async () => {
  const { markdownBlocks } = await import('./markdown-model.ts');
  const text = '<p align="center">\n  <img src="a.png">\n</p>\n\n<!-- a\nnote -->\nAfter';
  assert.deepEqual(markdownBlocks(text, { html: true }), [
    { type: 'html', text: '<p align="center">\n  <img src="a.png">\n</p>' },
    { type: 'html', text: '<!-- a\nnote -->' },
    { type: 'paragraph', text: 'After' },
  ]);
  assert.equal(markdownBlocks(text)[0].type, 'paragraph');
  assert.deepEqual(markdownBlocks('Generic <T> stays prose', { html: true }), [{ type: 'paragraph', text: 'Generic <T> stays prose' }]);
});
