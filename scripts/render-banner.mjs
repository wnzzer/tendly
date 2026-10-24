// Render docs/assets/banner-{light,dark}.png from docs/assets/banner.html, and
// docs/assets/wordmark-{light,dark}.png (the README header) from wordmark.html.
// Fonts come from apps/web/node_modules; the contour artwork is drawn here so
// the banner and the app share one backdrop. Run from the repository root:
//   node scripts/render-banner.mjs
// Needs Playwright with Chromium. It is not a dependency of the repository:
// install it anywhere and point PLAYWRIGHT_MODULE at its index.mjs, or have
// `playwright` resolvable from here.
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const root = resolve(new URL("..", import.meta.url).pathname);
const fontDir = "file://" + resolve(root, "apps/web/node_modules/@fontsource-variable");
// Contour lines, flowing from the lower left towards the upper right.
function curve(y0, amp, phase, k) {
  const W = 1280; const pts = [];
  for (let x = 0; x <= W; x += 16) {
    const y = y0 + amp * Math.sin(x / 210 + phase) + amp * 0.5 * Math.sin(x / 80 + phase * 1.6) - k * ((x - W * 0.5) / W) ** 2 * 420;
    pts.push(`${x.toFixed(1)} ${y.toFixed(1)}`);
  }
  return "M" + pts.join(" L");
}
const art = Array.from({ length: 8 }, (_, i) => `<path d="${curve(330 + i * 26, 34 - i * 2, i * 0.5, 0.9 - i * 0.05)}" opacity="${(0.8 - i * 0.08).toFixed(2)}"/>`).join("");
const html = readFileSync(resolve(root, "docs/assets/banner.html"), "utf8").replaceAll("FONT_DIR", fontDir).replace("ART_PATHS", art);

const browser = await chromium.launch();
for (const theme of ["light", "dark"]) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 400 }, deviceScaleFactor: 2 });
  // A file: page may load file: fonts; a setContent page (about:blank) may not.
  const file = resolve(mkdtempSync(resolve(tmpdir(), "banner-")), `${theme}.html`);
  writeFileSync(file, html.replace('class="wrap THEME"', `class="wrap ${theme}"`));
  await page.goto("file://" + file, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  await page.locator(".wrap").screenshot({ path: resolve(root, `docs/assets/banner-${theme}.png`), omitBackground: true });
  await page.close();
}
const wordmark = readFileSync(resolve(root, "docs/assets/wordmark.html"), "utf8").replaceAll("FONT_DIR", fontDir);
for (const theme of ["light", "dark"]) {
  const page = await browser.newPage({ viewport: { width: 800, height: 200 }, deviceScaleFactor: 2 });
  const file = resolve(mkdtempSync(resolve(tmpdir(), "wordmark-")), `${theme}.html`);
  writeFileSync(file, wordmark.replace('class="wrap THEME"', `class="wrap ${theme}"`));
  await page.goto("file://" + file, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  await page.locator(".wrap").screenshot({ path: resolve(root, `docs/assets/wordmark-${theme}.png`), omitBackground: true });
  await page.close();
}
await browser.close();
console.log("rendered banner-{light,dark}.png and wordmark-{light,dark}.png");
