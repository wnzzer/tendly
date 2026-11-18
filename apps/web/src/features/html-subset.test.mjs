import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeEntities, hasInlineHtml, opensHtmlBlock, parseHtml } from "./html-subset.ts";

test("a README header reads as its elements, with only the attributes allowed", () => {
  const tree = parseHtml(
    '<p align="center" style="color:red" onclick="x()">\n  <a href="https://example.com" target="_top"><img src="docs/a.png" alt="CI" width="100%" onerror="alert(1)"></a>\n</p>',
  );
  assert.deepEqual(tree, [
    {
      tag: "p",
      attrs: { align: "center" },
      children: [
        "\n  ",
        {
          tag: "a",
          attrs: { href: "https://example.com" },
          children: [{ tag: "img", attrs: { src: "docs/a.png", alt: "CI", width: "100%" }, children: [] }],
        },
        "\n",
      ],
    },
  ]);
});

test("scripts and styles go with their content; unknown elements keep only their text", () => {
  assert.deepEqual(parseHtml("<div>a<script>alert(1)</script>b<style>p{}</style><font color=red>c</font></div>"), [
    { tag: "div", attrs: {}, children: ["abc"] },
  ]);
  assert.deepEqual(parseHtml("<!-- hidden --><b>x</b>"), [{ tag: "b", attrs: {}, children: ["x"] }]);
  assert.deepEqual(parseHtml('<iframe src="https://evil">text'), []);
});

test("entities decode once, and markup inside text stays text", () => {
  assert.equal(decodeEntities("&lt;b&gt; &amp;amp; &#x41;&#66; &nbsp;&bogus;"), "<b> &amp; AB  &bogus;");
  assert.deepEqual(parseHtml("<kbd>&lt;img src=x onerror=1&gt;</kbd>"), [
    { tag: "kbd", attrs: {}, children: ["<img src=x onerror=1>"] },
  ]);
});

test("unclosed and stray tags close where they can", () => {
  assert.deepEqual(parseHtml("<details open><summary>More</summary>body</p>"), [
    { tag: "details", attrs: { open: "" }, children: [{ tag: "summary", attrs: {}, children: ["More"] }, "body"] },
  ]);
  assert.deepEqual(parseHtml("<b>x<i>y</b>z"), [
    { tag: "b", attrs: {}, children: ["x", { tag: "i", attrs: {}, children: ["y"] }] },
    "z",
  ]);
});

test("an HTML block opens on a known element or a comment, not on prose that mentions one", () => {
  assert.ok(opensHtmlBlock('<p align="center">'));
  assert.ok(opensHtmlBlock("<!-- note -->"));
  assert.ok(opensHtmlBlock("</div>"));
  assert.ok(!opensHtmlBlock("<T> is a type parameter"));
  assert.ok(!opensHtmlBlock("a <b>bold</b> word"));
  assert.ok(!opensHtmlBlock("<https://example.com>"));
});

test("inside prose only inline elements are read, and other angle brackets stay text", () => {
  assert.ok(hasInlineHtml("press <kbd>Ctrl</kbd>"));
  assert.ok(!hasInlineHtml("a Vec<T> and <https://example.com>"));
  assert.deepEqual(parseHtml("press <kbd>Ctrl</kbd> on Vec<T> &amp; <div>x</div>", true), [
    "press ",
    { tag: "kbd", attrs: {}, children: ["Ctrl"] },
    " on Vec<T> &amp; <div>x</div>",
  ]);
});
