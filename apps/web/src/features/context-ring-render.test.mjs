import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";

let server, ContextRing, i18n;
before(async () => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  ContextRing = (await server.ssrLoadModule("/src/features/ContextRing.vue")).default;
  i18n = await server.ssrLoadModule("/src/i18n/index.ts");
});
after(async () => {
  await server?.close();
});
const render = props => renderToString(createSSRApp(ContextRing, props));

test("the ring renders a real arc and an accessible label from reported numbers", async () => {
  i18n.useI18n().setLocale("en");
  const html = await render({ usage: { context_tokens: 50000, context_window: 200000 } });
  assert.match(html, /25%/);
  // The label states both sides of the ratio, so the ring is never the only
  // way to know what it is measuring.
  assert.match(html, /Context 25% · 50k of 200k tokens/);
  assert.match(html, /aria-label="Context 25%/);
  assert.match(html, /role="img"/);
  const dash = Number(/stroke-dasharray="([\d.]+)"/.exec(html)[1]);
  const offset = Number(/stroke-dashoffset="([\d.]+)"/.exec(html)[1]);
  assert.ok(Math.abs(offset / dash - 0.75) < 1e-6, `offset ${offset} of ${dash}`);
});

test("an unreported context window renders nothing rather than an empty or guessed ring", async () => {
  for (const usage of [undefined, {}, { context_tokens: 4000 }, { context_window: 200000 }]) {
    const html = await render({ usage });
    assert.equal(html.includes("context-ring"), false, JSON.stringify(usage));
    assert.equal(html.includes("%"), false, JSON.stringify(usage));
  }
});

test("a nearly full context is visually distinct and localized", async () => {
  const high = await render({ usage: { context_tokens: 170000, context_window: 200000 } });
  assert.match(high, /context-ring high/);
  const full = await render({ usage: { context_tokens: 199000, context_window: 200000 } });
  assert.match(full, /context-ring full/);
  i18n.useI18n().setLocale("zh-CN");
  const chinese = await render({ usage: { context_tokens: 50000, context_window: 200000 } });
  assert.match(chinese, /上下文已用 25%/);
  i18n.useI18n().setLocale("en");
});

test("the share is shown with the window it is a share of", async () => {
  // 143k of a 1M model really is 14%. Without the denominator that reads as a
  // bug, because the same count against 200k would be 71%.
  const wide = await render({ usage: { context_tokens: 142827, context_window: 1000000 } });
  assert.match(wide, /14%/);
  assert.match(wide, /143k\/1M/);
  const narrow = await render({ usage: { context_tokens: 142827, context_window: 200000 } });
  assert.match(narrow, /71%/);
  assert.match(narrow, /143k\/200k/);
});
