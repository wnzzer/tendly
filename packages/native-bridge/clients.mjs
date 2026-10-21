// The agent clients Tendly drives, by the provider id the server sends.
// Each entry names that client's own adapter per concern, so the bridges look
// a client up here instead of branching on its id; adding a client is its
// modules plus one entry.
//
// - `account(job, options)`: an account operation; `options.env` is already
//   isolated for this client (see account.mjs).
// - `history(cwd, configDir, configEnv)`: the client's saved sessions for one
//   workspace, as {items, truncated}.
// - `configEnv`: the variable that points the client at its config directory.
// - `envStrip`: the client's own variables. None of them is inherited by an
//   account process of any client, so one never runs on another's credentials.
import { claudeAccount } from './account-claude.mjs';
import { codexAccount } from './account-codex.mjs';
import { piAccount } from './account-pi.mjs';
import { claudeHistory } from './history-claude.mjs';
import { codexHistory } from './history-codex.mjs';
import { piHistory } from './history-pi.mjs';

export const CLIENTS = Object.freeze({
  claude_code: Object.freeze({ account: claudeAccount, history: claudeHistory, configEnv: 'CLAUDE_CONFIG_DIR', envStrip: /^(ANTHROPIC_|CLAUDE_|CLAUDECODE$)/i }),
  codex: Object.freeze({ account: codexAccount, history: codexHistory, configEnv: 'CODEX_HOME', envStrip: /^(OPENAI_|CODEX_|AZURE_OPENAI_)/i }),
  pi: Object.freeze({ account: piAccount, history: piHistory, configEnv: 'PI_CODING_AGENT_DIR', envStrip: /^PI_/i }),
});

/** The registered client for a provider id; inherited keys such as `constructor` are not ids. */
export const clientFor = provider => typeof provider === 'string' && Object.hasOwn(CLIENTS, provider) ? CLIENTS[provider] : undefined;
