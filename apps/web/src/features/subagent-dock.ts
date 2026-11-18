import { computed, nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { ChatItem, ToolItem } from "./chat-model";
import { dockHeading as dockHeadingText } from "./chat-format";
import { dockRoots, isLive, liveStats, runningSubagents, stalledFor, toolsById } from "./tool-runs";

type Translate = (text: string, values?: Record<string, string | number>) => string;

/**
 * Subagents.
 *
 * They run in the background, so most of their work lands after the turn that
 * started them has ended. While one runs it sits in a dock above the composer
 * rather than in the history: what it is doing now, what it has cost, and a
 * preview that opens to its log and its own calls. A finished one stays in the
 * dock, marked, until dismissed, and is in the history from then on. A card the
 * client has gone quiet on for a while says so, since a stuck subagent and a
 * slow one look the same otherwise.
 */

/** The dock's rows with any newly seen subagent joined at the end, so rows keep the order they started in. */
export function joinDock(ids: readonly string[], seen: readonly string[]): readonly string[] {
  const fresh = seen.filter(id => !ids.includes(id));
  return fresh.length ? [...ids, ...fresh] : ids;
}

/** The subagents the dock shows: one dismissed is gone, and one whose client exited is in the history, marked ended. */
export function dockedAgents(ids: readonly string[], tools: ReadonlyMap<string, ToolItem>): ToolItem[] {
  return ids.flatMap(id => {
    const tool = tools.get(id);
    return tool && !tool.ended ? [tool] : [];
  });
}

/** Whether a log scrolled to here is at its newest line, so it keeps following. */
export const atLogEnd = (scrollHeight: number, scrollTop: number, clientHeight: number) =>
  scrollHeight - scrollTop - clientHeight < 24;

export interface SubagentDockOptions {
  sessionId: () => string;
  running: Ref<boolean>;
  items: () => readonly ChatItem[];
  t: Translate;
}

export function useSubagentDock(options: SubagentDockOptions) {
  const { running, t } = options;
  const now = ref(Date.now());
  const subagents = computed(() => (running.value ? runningSubagents(options.items()) : []));
  /** Rows in the order their subagents started; one joins when it is first seen running. */
  const dockIds = ref<readonly string[]>([]),
    dockOpen = ref(true),
    expandedAgent = ref<string | null>(null),
    dock = ref<HTMLElement>();
  watch(options.sessionId, () => {
    dockIds.value = [];
    expandedAgent.value = null;
  });
  watch(
    () => dockRoots(options.items()).map(tool => tool.id),
    ids => {
      dockIds.value = joinDock(dockIds.value, ids);
    },
    { immediate: true },
  );
  const dockAgents = computed(() => (running.value ? dockedAgents(dockIds.value, toolsById(options.items())) : []));
  const dockRunning = computed(() => dockAgents.value.filter(isLive).length);
  // The clock only ticks while something in the dock runs, once a second so
  // its timers count; an idle pane holds no timer.
  let clock: ReturnType<typeof setInterval> | undefined;
  watch(
    () => subagents.value.length > 0 || dockRunning.value > 0,
    active => {
      clearInterval(clock);
      clock = undefined;
      if (active) {
        now.value = Date.now();
        clock = setInterval(() => {
          now.value = Date.now();
        }, 1000);
      }
    },
    { immediate: true },
  );
  onBeforeUnmount(() => clearInterval(clock));
  /** What the history's rows re-render on: the quiet line only moves by the minute. */
  const slowNow = computed(() => Math.floor(now.value / 30_000));
  const dockHeading = computed(() => dockHeadingText(dockAgents.value, t));
  function dismissAgent(id: string) {
    dockIds.value = dockIds.value.filter(entry => entry !== id);
    if (expandedAgent.value === id) expandedAgent.value = null;
  }
  function clearFinished() {
    for (const agent of dockAgents.value) if (agent.status !== "running") dismissAgent(agent.id);
  }
  // The open log follows the newest line, unless it was scrolled back to read.
  let followLog = true;
  function logScrolled(event: Event) {
    const el = event.target as HTMLElement;
    followLog = atLogEnd(el.scrollHeight, el.scrollTop, el.clientHeight);
  }
  watch(expandedAgent, () => {
    followLog = true;
  });
  watch(
    () => dockAgents.value.find(agent => agent.id === expandedAgent.value)?.activity?.length,
    () => {
      if (!followLog) return;
      void nextTick(() => {
        const log = dock.value?.querySelector<HTMLElement>(".subagent-log");
        if (log) log.scrollTop = log.scrollHeight;
      });
    },
    { flush: "post" },
  );
  function quietLine(tool: ToolItem) {
    const quiet = running.value ? stalledFor(tool, now.value) : undefined;
    return quiet === undefined ? "" : t("No progress for {minutes} min", { minutes: Math.floor(quiet / 60_000) });
  }
  function agentStats(tool: ToolItem) {
    return liveStats(tool, now.value);
  }
  // `slowNow` is also what re-renders a history row whose quiet line has changed.
  return {
    now,
    slowNow,
    agentStats,
    dock,
    dockIds,
    dockOpen,
    expandedAgent,
    dockAgents,
    dockRunning,
    dockHeading,
    dismissAgent,
    clearFinished,
    logScrolled,
    quietLine,
  };
}
