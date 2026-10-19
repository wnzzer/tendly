import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp } from 'vue';
import { renderToString } from 'vue/server-renderer';

let server, Panel;
const catalog = [
  { id: 'claude-fable-5', name: 'Fable 5' },
  { id: 'gpt-5.5', name: 'GPT-5.5' },
];

before(async () => {
  // The actual SFC and template, compiled: no browser, no network, no check
  // is ever sent from here.
  server = await createServer({ configFile: false, root: fileURLToPath(new URL('../../', import.meta.url)), plugins: [vue()], server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', optimizeDeps: { noDiscovery: true } });
  Panel = (await server.ssrLoadModule('/src/features/ModelSelection.vue')).default;
});
after(async () => { await server?.close(); });

async function render(props) {
  const context = {};
  const html = await renderToString(createSSRApp(Panel, { catalog, windowText: () => '', placeholder: () => '', ...props }), context);
  return html + Object.values(context.teleports ?? {}).join('');
}
const plan = (api, path, guessed = false) => ({ client: 'Pi', api, path, guessed });

test('the panel lists the endpoint catalogue with each row untested and unchecked', async () => {
  const html = await render({ selected: [], probe: async () => [] });
  assert.ok(html.includes('Fable 5'), 'the name');
  assert.ok(html.includes('claude-fable-5'), 'and the ID where it differs from the name');
  assert.ok(html.includes('未检测'), 'a model nobody checked says so');
  assert.ok(html.includes('模型清单'), 'Chinese is the default copy');
  // Discovery never checked anything, so no pruning action is offered yet.
  assert.ok(!html.includes('剔除失败项') && !html.includes('只保留可用<'), 'nothing to prune before a check');
});

test('a server without the check keeps the list and offers no button', async () => {
  const html = await render({ selected: [], probe: undefined });
  assert.ok(html.includes('Fable 5'), 'the catalogue still shows');
  assert.ok(!html.includes('检测当前列表'), 'no check on an older backend');
});

test('the panel offers one check action and says what it costs', async () => {
  const html = await render({ selected: [], probe: async () => [] });
  assert.ok(html.includes('检测当前列表'), 'the batch action');
  assert.ok(html.includes('按调用计费'), 'the cost, before anyone presses it');
  assert.ok(!html.includes('>停止<'), 'nothing to stop until it runs');
  // Nothing is checked yet, so no per-model answer is invented.
  assert.ok(!html.includes('可用') || html.includes('未检测'));
});

test('chosen models are ticked, count towards the summary, and offer advanced settings', async () => {
  const html = await render({ selected: ['gpt-5.5'], probe: async () => [] });
  const checked = html.match(/<input\b[^>]*type="checkbox"[^>]*>/g) ?? [];
  assert.equal(checked.length, 2);
  assert.equal(checked.filter(tag => /\bchecked\b/.test(tag)).length, 1);
  assert.ok(html.includes('已选 1 / 2'), 'the summary counts the choice');
  assert.ok(html.includes('>高级<'), 'per-model context window stays reachable');
});

test('the panel says which protocol a check would speak before it runs', async () => {
  const html = await render({ selected: [], probe: async () => [], plan: plan('anthropic-messages', '/v1/messages') });
  assert.ok(html.includes('Anthropic Messages'), 'the protocol, named');
  assert.ok(html.includes('/v1/messages'), 'and the path it would be sent to');
  assert.ok(!html.includes('由 URL 推断'), 'nothing to guess when the API is named');
});

test('a guessed protocol is called a guess, because it decides the request', async () => {
  const html = await render({ selected: [], probe: async () => [], plan: plan('openai-completions', '/v1/chat/completions', true) });
  assert.ok(html.includes('OpenAI Chat Completions'));
  assert.ok(html.includes('由 URL 推断'), 'the guess is named as one');
});

test('a server without the plan simply says nothing about protocol', async () => {
  const html = await render({ selected: [], probe: async () => [] });
  assert.ok(html.includes('Fable 5'), 'the catalogue still shows');
  assert.ok(!html.includes('Anthropic Messages'), 'and no protocol is invented');
});
