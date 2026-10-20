import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp } from 'vue';
import { renderToString } from 'vue/server-renderer';
import { TAB_ICON_PALETTE } from './tab-icons.ts';
import { iconColor, providerColor } from './icon-palette.ts';
import { DEFAULT_CLIENT } from './clients.ts';

let server, TabIcon, LayoutNode, Canvas, Icon, ProviderIcon;
before(async () => {
  // Compile real SFCs without an HTTP listener, HMR, browser, native CLI or account access.
  server = await createServer({ configFile: false, root: fileURLToPath(new URL('../../', import.meta.url)), plugins: [vue()], server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', optimizeDeps: { noDiscovery: true } });
  TabIcon = (await server.ssrLoadModule('/src/features/TabIcon.vue')).default;
  LayoutNode = (await server.ssrLoadModule('/src/components/LayoutNode.vue')).default;
  Canvas = (await server.ssrLoadModule('/src/components/Canvas.vue')).default;
  Icon = (await server.ssrLoadModule('/src/features/Icon.vue')).default;
  ProviderIcon = (await server.ssrLoadModule('/src/features/ProviderIcon.vue')).default;
});
after(async () => { await server?.close(); });
const pane = (id, kind = 'agent_chat', metadata = {}) => ({ type: 'pane', id, kind, title: id, metadata });
const render = (component, props) => renderToString(createSSRApp(component, props));
function tabs(html) { return html.match(/<div\b[^>]*role="tab"[^>]*>[\s\S]*?<\/div>/g) ?? []; }
function assertColor(html, type) {
  const marker = html.match(new RegExp(`<span\\b[^>]*data-tab-icon="${type}"[^>]*>`))?.[0];
  assert.ok(marker, `${type} icon is rendered`);
  assert.ok(marker.toLowerCase().includes(`color:${TAB_ICON_PALETTE[type].color.toLowerCase()}`), `${type} has its own foreground`);
  assert.ok(marker.toLowerCase().includes(`--tab-tint:${TAB_ICON_PALETTE[type].background.toLowerCase()}`), `${type} has its own light tint`);
}

test('TabIcon renders the existing official provider vectors with an explicit colored container', async () => {
  for (const provider of ['claude_code', 'codex']) {
    const html = await render(TabIcon, { kind: 'agent_chat', metadata: { provider } });
    assertColor(html, provider);
    assert.ok(html.includes(`data-icon-provider="${provider}"`));
    assert.match(html, /<svg\b[^>]*fill="currentColor"/);
    assert.ok(html.includes('provider-glyph'));
    assert.ok(!html.includes('<img'));
  }
});

test('active and inactive session tabs keep their provider colors when selection changes', async () => {
  const panes = [pane('claude-tab', 'agent_chat', { provider: 'claude_code' }), pane('codex-tab', 'agent_chat', { provider: 'codex' })];
  for (const activePaneId of ['claude-tab', 'codex-tab']) {
    const html = await render(LayoutNode, { node: { type: 'stack', kind: 'stack', id: 'tabs', panes, activePaneId }, selected: activePaneId });
    const rendered = tabs(html);
    assert.equal(rendered.length, 2);
    assertColor(rendered[0], 'claude_code');assertColor(rendered[1], 'codex');
    assert.equal(rendered.filter(tab => tab.includes('aria-selected="true"')).length, 1);
  }
});

test('inactive session menu targets stay mounted without reserving title space and narrow panes hide workspace badges', async () => {
  const panes = [pane('one', 'agent_chat', { session_id: 'one', workspace_id: 'workspace' }), pane('two', 'agent_chat', { session_id: 'two', workspace_id: 'workspace' })];
  for (const activePaneId of ['one', 'two']) {
    const html = await render(LayoutNode, { node: { type: 'stack', kind: 'stack', id: 'tabs', panes, activePaneId }, workspaceLabels: { workspace: 'An unusually long workspace name' } });
    for (const id of ['one', 'two']) assert.ok(html.includes(`id="session-actions-${id}"`), 'Teleport targets survive tab selection');
    assert.ok(html.includes('An unusually long workspace name'), 'full workspace context remains in accessible labels');
  }
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../components/LayoutNode.vue', import.meta.url), 'utf8');
  assert.match(source, /\.dock-tab\.is-active \{ max-width:300px;/);
  assert.match(source, /\.dock-tab:not\(\.is-active\)>\.dock-tab-session-actions \{ display:none; \}/);
  assert.match(source, /@container dock-pane \(max-width:480px\) \{ \.dock-tab-workspace,\.dock-tab-branch \{ display:none; \}/);
});

test('session provider mapping passes through both recursive split branches for legacy tabs', async () => {
  const node = { type: 'split', id: 'outer', direction: 'horizontal', ratio: 0.5,
    first: pane('left', 'agent_chat', { session_id: 'actual-claude', provider: 'codex' }),
    second: { type: 'split', id: 'inner', direction: 'vertical', ratio: 0.5, first: pane('upper', 'agent_chat', { session_id: 'actual-codex' }), second: pane('lower', 'git_diff') },
  };
  const html = await render(LayoutNode, { node, sessionProviders: { 'actual-claude': 'claude_code', 'actual-codex': 'codex' } });
  const rendered = tabs(html);assert.equal(rendered.length, 3);
  assertColor(rendered[0], 'claude_code');assertColor(rendered[1], 'codex');assertColor(rendered[2], 'git');
  assert.ok(html.includes('data-node-id="left"'));assert.ok(html.includes('data-node-id="upper"'));assert.ok(html.includes('data-node-id="lower"'));
});

test('file, preview and terminal tabs plus add menus use colored native SVG instead of symbol placeholders', async () => {
  const panes = [pane('text', 'editor'), pane('photo', 'file_preview', { path: 'photo.png' }), pane('movie', 'file_preview', { path: 'clip.mp4' }), pane('shell', 'terminal')];
  const html = await render(LayoutNode, { node: { type: 'stack', kind: 'stack', id: 'files', panes, activePaneId: 'text' } });
  const rendered = tabs(html);
  ['text', 'image', 'video', 'terminal'].forEach((type, index) => assertColor(rendered[index], type));
  const menu = html.match(/<div class="dock-menu-items"[^>]*>[\s\S]*?<\/div>/)?.[0];
  // New sessions are one row (the kind made last) with the rest in its submenu, which renders on hover.
  assert.ok(menu);[DEFAULT_CLIENT, 'git'].forEach(type => assertColor(menu, type));
  // Opening a file gives it a pane of its own, so an empty one is not offered.
  assert.ok(!menu.includes('data-tab-icon="text"') && !menu.includes('data-tab-icon="image"'));
  assert.ok(menu.includes('aria-haspopup="menu"'), 'the new-session row offers every kind beside it');
  assert.ok(!html.includes('>±</span>'));assert.ok(!html.includes('>◈</span>'));assert.ok(!html.includes('>▧</span>'));
});

test('key action and provider vectors carry their semantic default without a colored wrapper', async () => {
  const defaults = await render(Icon, { name: 'grid' });
  const defaultPath = defaults.match(/<path\b[^>]*d="([^"]+)"/)?.[1];
  for (const name of ['locate', 'archive', 'restore', 'layout', 'splitHorizontal', 'splitVertical', 'maximize', 'minimize', 'more']) {
    const html = await render(Icon, { name });
    assert.ok(html.includes(`data-icon="${name}"`));
    assert.ok(html.includes(`--icon-semantic-color:${iconColor(name)}`));
    assert.notEqual(html.match(/<path\b[^>]*d="([^"]+)"/)?.[1], defaultPath, `${name} has its own vector`);
  }
  for (const provider of ['claude_code', 'codex', 'terminal']) {
    const html = await render(ProviderIcon, { provider });
    assert.ok(html.includes(`--icon-semantic-color:${providerColor(provider)}`));
    assert.ok(html.includes('color:var(--icon-color, var(--icon-semantic-color))'));
  }
});

test('locatable tabs expose stable pane ids and accessible panel relationships in recursive layouts', async () => {
  const node = { type: 'split', id: 'outer', direction: 'horizontal', ratio: 0.5,
    first: pane('left', 'git_diff'),
    second: { type: 'stack', kind: 'stack', id: 'sessions', activePaneId: 'right', panes: [pane('hidden'), pane('right')] },
  };
  const html = await render(LayoutNode, { node, selected: 'right', locatedPaneId: 'right' });
  for (const id of ['left', 'hidden', 'right']) {
    assert.ok(html.includes(`data-pane-tab-id="${id}"`));
    assert.ok(html.includes(`id="dock-tab-${id}"`));
    assert.ok(html.includes(`aria-controls="dock-panel-${id}"`));
  }
  assert.match(html, /class="dock-pane is-selected is-located"[^>]*data-pane-id="right"/);
  assert.ok(html.includes('id="dock-panel-right" aria-labelledby="dock-tab-right"'));
});

test('canvas keeps presets in a single compact footer menu without a duplicate heading or toolbar', async () => {
  const html = await render(Canvas, { modelValue: { version: 1, root: pane('session-one') } });
  assert.ok(!html.includes('dock-canvas-toolbar'));
  assert.ok(!html.includes('dock-canvas-heading'));
  assert.equal((html.match(/class="dock-layout-menu"/g) ?? []).length, 1);
  const footer = html.slice(html.indexOf('class="dock-canvas-hint"'));
  const menu = footer.slice(footer.indexOf('<details'), footer.indexOf('</details>'));
  for (const preset of ['1:1', '2×2', '1:2:1']) assert.equal((menu.match(new RegExp(`<small[^>]*>${preset}</small>`, 'g')) ?? []).length, 1);
  assert.ok(menu.includes('data-icon="splitHorizontal"'));
  assert.ok(menu.includes('data-icon="splitVertical"'));
});

test('solid and destructive control icons inherit foreground while tab media keeps its own palette', async () => {
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('./Icon.vue', import.meta.url), 'utf8');
  assert.match(source, /:where\(\.tab-icon,\.primary-button,[^)]*\.chat-send,[^)]*\.danger,[^)]*\) :is\(\.app-icon,\.provider-glyph\)\{--icon-color:currentColor\}/);
});
