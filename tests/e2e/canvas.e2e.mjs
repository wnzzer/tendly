// The canvas from the keyboard: Ctrl+B, released, then a key (shortcuts.ts).
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { openBrowser, paneFor, row, seed, setLayout, startGateway } from './harness.mjs';

let gateway, browser;
before(async () => { gateway = await startGateway(); browser = await openBrowser(gateway, { width: 2400, height: 1000 }); });
afterEach(() => browser.reset());
after(async () => { await browser?.close(); await gateway?.stop(); });

async function twoTerminals() {
  const { sessions: [one, two] } = await seed(browser.api, gateway, ['terminal', 'terminal']);
  await setLayout(browser.api, row(paneFor('one', one), paneFor('two', two)));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  await page.locator('[data-pane-id="two"]').waitFor();
  await page.locator('[data-pane-id="one"] .terminal-host').click();
  return page;
}
async function prefix(page, key) {
  await page.keyboard.press('Control+b');
  await page.locator('.prefix-hint').waitFor();
  if (key) await page.keyboard.press(key);
}
const selected = page => page.locator('.dock-pane.is-selected').getAttribute('data-pane-id');

test('moving to the pane on the right does not flash it', async () => {
  // Reported: Ctrl+B → made the right pane blink. Its locate pulse replaced
  // the pane's entrance animation, which replayed from transparent when the
  // pulse ended.
  const page = await twoTerminals();
  await page.evaluate(() => {
    window.lowest = 1;
    const started = performance.now();
    const sample = () => {
      const pane = document.querySelector('[data-pane-id="two"]');
      if (pane) window.lowest = Math.min(window.lowest, Number(getComputedStyle(pane).opacity));
      if (performance.now() - started < 2500) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
  await prefix(page, 'ArrowRight');
  assert.equal(await selected(page), 'two');
  await page.waitForTimeout(2600);
  assert.equal(await page.evaluate(() => window.lowest), 1, 'the pane moved to never fades');
  assert.deepEqual(browser.errors, []);
});

test('prefix keys move between, split, maximize and close panes', async () => {
  const page = await twoTerminals();
  await prefix(page); await page.keyboard.press('Escape');
  assert.equal(await page.locator('.prefix-hint').count(), 0, 'Escape leaves the prefix');
  await prefix(page, 'l'); assert.equal(await selected(page), 'two');
  await prefix(page, 'h'); assert.equal(await selected(page), 'one');
  await prefix(page, 'o'); assert.equal(await selected(page), 'two');
  await prefix(page, ';'); assert.equal(await selected(page), 'one');
  await prefix(page, 'z'); assert.equal(await page.locator('.dock-pane').count(), 1, 'maximized');
  await prefix(page, 'z'); assert.equal(await page.locator('.dock-pane').count(), 2, 'restored');
  await prefix(page, '%'); assert.equal(await page.locator('.dock-pane').count(), 3, 'split side by side');
  assert.ok(await page.evaluate(() => !!document.activeElement?.closest('.dock-empty')), 'the new pane is focused');
  await prefix(page, 'x'); assert.equal(await page.locator('.dock-pane').count(), 2, 'empty pane closed');
  await prefix(page, '?'); await page.locator('[aria-modal="true"]').waitFor(); await page.keyboard.press('Escape');
  await prefix(page, ':'); await page.locator('.palette').waitFor(); await page.keyboard.press('Escape');
  assert.deepEqual(browser.errors, []);
});

test('a terminal runs what is typed into it, and Ctrl+B twice reaches it', async () => {
  const page = await twoTerminals();
  await page.keyboard.type('echo e2e-$((40+2))'); await page.keyboard.press('Enter');
  await page.locator('[data-pane-id="one"]').getByText('e2e-42').first().waitFor();
  await prefix(page, 'Control+b');
  assert.equal(await page.locator('.prefix-hint').count(), 0);
  assert.deepEqual(browser.errors, []);
});
