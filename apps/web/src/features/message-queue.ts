import { reactive } from "vue";
import { composeMessage, type Attachment } from "./attachment-state";

/**
 * Messages written while the agent is still working.
 *
 * Neither client accepts a second message mid-turn, so pressing Enter then
 * used to do nothing. The message waits here instead, visible above the
 * composer where it can still be edited back or removed, and the pane sends
 * the first one the moment the turn ends. Held per session in memory only: a
 * reload drops the queue rather than sending something the user no longer sees.
 */
/** Attachments wait with the words; a screenshot pasted mid-turn used to be
 * refused here, and Enter then did nothing at all. */
export interface QueuedMessage {
  id: string;
  text: string;
  attachments?: Attachment[];
}

/** What goes on the wire for a waiting message: its words, with its files named. */
export function queuedContent(message: QueuedMessage) {
  return composeMessage(message.text, message.attachments ?? []);
}

const queues = reactive(new Map<string, QueuedMessage[]>());

export const messageQueue = {
  get(sessionId: string): QueuedMessage[] {
    if (!queues.has(sessionId)) queues.set(sessionId, []);
    return queues.get(sessionId)!;
  },
  push(sessionId: string, message: QueuedMessage) {
    messageQueue.get(sessionId).push(message);
  },
  remove(sessionId: string, id: string) {
    const list = messageQueue.get(sessionId),
      index = list.findIndex(item => item.id === id);
    return index < 0 ? undefined : list.splice(index, 1)[0];
  },
  shift(sessionId: string) {
    return messageQueue.get(sessionId).shift();
  },
};
