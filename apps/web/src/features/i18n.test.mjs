import test from "node:test";
import assert from "node:assert/strict";
import { computed } from "vue";
import { createI18n, DEFAULT_LOCALE, LOCALE_STORAGE_KEY, normalizeLocale, translate, zhCN } from "../i18n/index.ts";

test("Chinese is the default, with English preference restored across instances", () => {
  const data = new Map();
  const storage = { getItem: key => data.get(key) ?? null, setItem: (key, value) => data.set(key, value) };
  const first = createI18n(storage);
  assert.equal(first.locale.value, DEFAULT_LOCALE);
  assert.equal(first.t("Workspace canvas"), "工作区画布");
  first.setLocale("en");
  assert.equal(data.get(LOCALE_STORAGE_KEY), "en");
  assert.equal(createI18n(storage).t("Workspace canvas"), "Workspace canvas");
});

test("all consumers react immediately to a language change without remounting panes", () => {
  const i18n = createI18n();
  const label = computed(() => i18n.t("New session"));
  assert.equal(label.value, "新建会话");
  i18n.setLocale("en");
  assert.equal(label.value, "New session");
  i18n.setLocale("zh-CN");
  assert.equal(label.value, "新建会话");
});

test("invalid preferences and unavailable browser storage remain usable", () => {
  assert.equal(normalizeLocale("es"), "zh-CN");
  const denied = {
    getItem() {
      throw new Error("denied");
    },
    setItem() {
      throw new Error("denied");
    },
  };
  const i18n = createI18n(denied);
  assert.equal(i18n.locale.value, "zh-CN");
  assert.doesNotThrow(() => i18n.setLocale("en"));
  assert.equal(i18n.locale.value, "en");
});

test("interpolation supports counts, paths and repeated placeholders without changing values", () => {
  assert.equal(translate("{count} changes", "zh-CN", { count: 3 }), "3 项变更");
  assert.equal(
    translate("Download {path}", "zh-CN", { path: "/tmp/Changes/{count}.ts" }),
    "下载 /tmp/Changes/{count}.ts",
  );
  assert.equal(
    translate("{name}: {name} {missing}", "en", { name: "<b>原文</b>" }),
    "<b>原文</b>: <b>原文</b> {missing}",
  );
});

test("unknown copy and native output fall back exactly; there is no global text replacement", () => {
  const native = "Claude Code: run git diff --stat; file: Workspace canvas.md";
  assert.equal(translate(native, "zh-CN"), native);
  assert.equal(translate("gpt-5.3-codex", "zh-CN"), "gpt-5.3-codex");
  assert.equal(translate("Workspace canvas", "en"), "Workspace canvas");
  assert.equal(translate("toString", "zh-CN"), "toString");
});

test("translation placeholders match the English source contract", () => {
  const placeholders = text => (text.match(/\{\w+\}/g) ?? []).sort();
  for (const [source, translated] of Object.entries(zhCN)) {
    assert.deepEqual(placeholders(translated), placeholders(source), source);
  }
});
