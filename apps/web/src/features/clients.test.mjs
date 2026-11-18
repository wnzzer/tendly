import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ACCOUNT_CLIENTS,
  AGENT_CLIENTS,
  DEFAULT_CLIENT,
  PROVIDER_KINDS,
  agentClient,
  clientInfo,
  clientLabel,
  clientTone,
  isAgentClient,
  isProviderKind,
  nativeCatalogNote,
  providerLabel,
} from "./clients.ts";
import { ICON_PALETTE } from "./icon-palette.ts";

test("agent clients come in menu order, the default first, and the terminal only among session kinds", () => {
  assert.deepEqual(AGENT_CLIENTS, ["claude_code", "codex", "pi"]);
  assert.equal(DEFAULT_CLIENT, "claude_code");
  assert.deepEqual(PROVIDER_KINDS, ["claude_code", "codex", "pi", "terminal"]);
  // The account form has always opened on Codex.
  assert.deepEqual(ACCOUNT_CLIENTS, ["codex", "claude_code", "pi"]);
  assert.ok(isAgentClient("codex"));
  assert.ok(!isAgentClient("terminal"));
  assert.ok(!isAgentClient("toString"));
  assert.ok(isProviderKind("terminal"));
  assert.ok(isProviderKind("claude_code"));
  assert.ok(!isProviderKind("shell"));
  assert.ok(!isProviderKind(undefined));
});

test("labels name each client, the terminal, and an unknown provider as itself", () => {
  assert.equal(clientLabel("claude_code"), "Claude Code");
  assert.equal(clientLabel("codex"), "Codex");
  assert.equal(providerLabel("codex"), "Codex");
  assert.equal(providerLabel("terminal"), "Terminal");
  assert.equal(providerLabel("future_client"), "future_client");
  assert.equal(agentClient("terminal"), undefined);
  assert.equal(agentClient("codex"), clientInfo("codex"));
});

test("every client is complete enough to draw, colour and offer", () => {
  for (const id of AGENT_CLIENTS) {
    const info = clientInfo(id);
    assert.match(info.glyph.path, /^M[\d.\s-]/, `${id} has a glyph`);
    assert.ok(Object.hasOwn(ICON_PALETTE, info.iconTone), `${id} has a palette tone`);
    assert.match(info.tabTint, /^#[0-9A-F]{6}$/i, `${id} has a tab tint`);
    // A client that asks can also be told not to; one that never asks offers no modes.
    assert.ok(
      info.approvals
        ? info.permissionModes.includes("ask") && info.permissionModes.includes("danger")
        : !info.permissionModes.length,
      `${id}'s modes match whether it asks`,
    );
    assert.ok(info.approvals || info.chatNote, `${id} says so in the chat when it never asks`);
    assert.ok(info.contextWindow.help, `${id} explains its context window`);
  }
});

test("client-specific behaviour is read from the registry", () => {
  assert.deepEqual(clientInfo("claude_code").permissionModes, ["ask", "plan", "accept_edits", "danger"]);
  assert.deepEqual(clientInfo("codex").permissionModes, ["ask", "danger"]);
  assert.deepEqual(clientInfo("pi").permissionModes, []);
  assert.equal(clientInfo("pi").approvals, false);
  assert.equal(clientInfo("claude_code").profilePlan, true);
  assert.equal(clientInfo("codex").profilePlan, false);
  assert.equal(clientInfo("claude_code").modelSlots, true);
  assert.equal(clientInfo("codex").modelSlots, false);
  assert.equal(clientInfo("claude_code").contextWindow.variable, "CLAUDE_CODE_MAX_CONTEXT_TOKENS");
  assert.equal(clientInfo("codex").contextWindow.variable, undefined);
  assert.ok(clientInfo("claude_code").proxyNote);
  assert.equal(clientInfo("codex").proxyNote, undefined);
  assert.ok(clientInfo("claude_code").chatNote);
  assert.equal(clientInfo("codex").chatNote, undefined);
});

test("a mark is tinted by its client token, and nothing else is", () => {
  assert.deepEqual(clientTone("claude_code"), {
    "--client-ink": "var(--claude)",
    "--client-soft": "var(--claude-soft)",
  });
  assert.deepEqual(clientTone("terminal"), {});
});

test("a native catalog is recognised by the source its client reports", () => {
  assert.equal(
    nativeCatalogNote("codex://model/list"),
    "Loaded from the native Codex catalog. Account access may differ.",
  );
  assert.equal(nativeCatalogNote("https://api.example.com/v1/models"), undefined);
  assert.equal(nativeCatalogNote(undefined), undefined);
});
