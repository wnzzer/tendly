import { lastChatSeq, type ChatEvent } from "./chat-model";

/**
 * The conversation a view last showed, kept when its pane goes away -- a tab
 * switched away from unmounts it -- so coming back shows the conversation at
 * once instead of an empty pane while it is read again. The view then resumes
 * its socket after the last event it holds, which brings what happened
 * meanwhile and the session's current state.
 */
export interface CachedConversation {
  events: ChatEvent[];
  mode: "structured";
  running: boolean;
  truncated: boolean;
  /** The cursor to the history before these events, if any. */
  older?: number;
}

/**
 * Recently shown conversations, most recent last; the oldest goes first.
 * Measured on a real deployment, 22 conversations held together took about
 * 7 MB of heap, the largest (2,000 events, 2.2 MB on the wire) 3.9 MB, most
 * well under 1 MB: fifty is cheap, and covers every tab a person keeps open.
 */
export function createConversationCache(limit = 50) {
  const entries = new Map<string, CachedConversation>();
  return {
    get(id: string): CachedConversation | undefined {
      const entry = entries.get(id);
      if (!entry) return undefined;
      entries.delete(id);
      entries.set(id, entry);
      return entry;
    },
    /** Only a structured conversation that holds events: there is nothing to resume from otherwise. */
    remember(
      id: string,
      entry: { events: ChatEvent[]; mode?: string; running: boolean; truncated: boolean; older?: number },
    ) {
      if (entry.mode !== "structured" || lastChatSeq(entry.events) === undefined) return;
      entries.delete(id);
      entries.set(id, {
        events: entry.events,
        mode: "structured",
        running: entry.running,
        truncated: entry.truncated,
        ...(entry.older === undefined ? {} : { older: entry.older }),
      });
      while (entries.size > limit) entries.delete(entries.keys().next().value!);
    },
    forget(id: string) {
      entries.delete(id);
    },
    get size() {
      return entries.size;
    },
  };
}

export const conversationCache = createConversationCache();
