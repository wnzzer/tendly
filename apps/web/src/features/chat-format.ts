/**
 * What a conversation shows as text beside its tools and turns: how long a
 * tool ran and what it cost, what a turn read and wrote, and what the
 * subagent dock's heading says. Each takes the translator so the wording stays
 * in the dictionary.
 */
import { SUBAGENT_TOOLS, type ToolItem, type ToolStats, type TurnUsage } from "./chat-model";
import { cachedPercent, compactCount, isLive } from "./tool-runs";

type Translate = (text: string, values?: Record<string, string | number>) => string;

/** "42s", or "3m 5s" from a minute on. */
export function duration(ms: number, t: Translate): string {
  const seconds = Math.round(ms / 1000);
  return seconds < 60
    ? t("{seconds}s", { seconds })
    : t("{minutes}m {seconds}s", { minutes: Math.floor(seconds / 60), seconds: seconds % 60 });
}

/** "12 tool uses · 4.5k tokens · 1m 2s", from whichever of these the client reported. */
export function statsLine(stats: ToolStats, t: Translate): string {
  const parts: string[] = [];
  if (stats.tools !== undefined) parts.push(t("{count} tool uses", { count: stats.tools }));
  if (stats.tokens !== undefined) parts.push(t("{count} tokens", { count: compactCount(stats.tokens) }));
  if (stats.ms !== undefined) parts.push(duration(stats.ms, t));
  return parts.join(" · ");
}

/** "↑ 9.4k · ↓ 1.2k · 99.7% cached": read, written, and how much of the read was cached. */
export function turnUsageLine(spent: TurnUsage, t: Translate): string {
  const parts = [`↑ ${compactCount(spent.input ?? 0)}`, `↓ ${compactCount(spent.output ?? 0)}`];
  if (spent.input && spent.cached)
    parts.push(t("{percent}% cached", { percent: cachedPercent(spent.input, spent.cached) }));
  return parts.join(" · ");
}

/** The same turn's figures in full, for its tooltip. */
export function turnUsageTitle(spent: TurnUsage, t: Translate): string {
  return t("This turn read {input} tokens ({cached} from the prompt cache) and wrote {output}", {
    input: (spent.input ?? 0).toLocaleString(),
    cached: (spent.cached ?? 0).toLocaleString(),
    output: (spent.output ?? 0).toLocaleString(),
  });
}

/** What the dock's heading says: subagents and background commands are counted apart. */
export function dockHeading(tools: readonly ToolItem[], t: Translate): string {
  const live = tools.filter(isLive);
  const agents = live.filter(tool => SUBAGENT_TOOLS.has(tool.name)).length;
  const commands = live.length - agents;
  if (!live.length)
    return t(tools.every(tool => SUBAGENT_TOOLS.has(tool.name)) ? "Subagents finished" : "Background work finished");
  if (!commands) return agents === 1 ? t("1 subagent running") : t("{count} subagents running", { count: agents });
  if (!agents)
    return commands === 1
      ? t("1 background command running")
      : t("{count} background commands running", { count: commands });
  return t("{agents} subagents and {commands} commands running", { agents, commands });
}

/** What the pane is doing, as far as its status line is concerned. */
export interface ChatStatus {
  preview: boolean;
  booting: boolean;
  loading: boolean;
  awaitingApproval: boolean;
  working: boolean;
  disconnected: boolean;
  structured: boolean;
}

/** The status line's text, untranslated: the most pressing of the pane's states wins. */
export function statusLabel(state: ChatStatus): string {
  if (state.preview) return "Read-only UI preview";
  if (state.booting) return "Starting {provider}…";
  if (state.loading) return "Loading conversation…";
  if (state.awaitingApproval) return "Waiting for your approval";
  if (state.working) return "Agent is working";
  if (state.disconnected) return "View disconnected";
  return state.structured ? "Ready for your next idea" : "Native client session";
}
