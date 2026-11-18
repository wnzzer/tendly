import { reactive } from "vue";
import { approvalPayloadAnswers, type ChatItem } from "./chat-model";

export type ApprovalItem = Extract<ChatItem, { type: "approval" }>;

/** A radio's value for "none of these": the typed answer beside it is what is sent. */
export const OTHER_ANSWER = "__tendly_other__";

/**
 * What has been answered so far to the agents' questions, per request and per
 * question: the options chosen, and any answer typed under "other".
 *
 * Held by the session rather than the dock that draws the questions, because
 * the composer's keys submit them too and a request can be paged away from and
 * back to without losing what was chosen.
 */
export function useApprovalAnswers() {
  const chosen = reactive<Record<string, Record<string, string[]>>>(Object.create(null));
  const other = reactive<Record<string, Record<string, string>>>(Object.create(null));

  /** Forget one request's answers, once it is resolved; or every request's. */
  function clear(id?: string) {
    for (const key of id ? [id] : new Set([...Object.keys(chosen), ...Object.keys(other)])) {
      delete chosen[key];
      delete other[key];
    }
  }
  /** A single choice, or a typed answer. */
  function answer(id: string, question: string, event: Event) {
    const value = (event.target as HTMLInputElement).value;
    (chosen[id] ??= Object.create(null))[question] = value && value !== OTHER_ANSWER ? [value] : [];
    if (value !== OTHER_ANSWER && other[id]) delete other[id][question];
  }
  /** One option of several, ticked or unticked. */
  function toggle(id: string, question: string, value: string, event: Event) {
    const current = (chosen[id] ??= Object.create(null))[question] ?? [];
    chosen[id][question] = (event.target as HTMLInputElement).checked
      ? [...new Set([...current, value])]
      : current.filter(item => item !== value);
  }
  /** The "other" answer typed; for a single choice it replaces whichever option was chosen. */
  function typeOther(id: string, question: string, event: Event, multiSelect = false) {
    const value = (event.target as HTMLInputElement).value;
    (other[id] ??= Object.create(null))[question] = value;
    if (value.trim() && !multiSelect) (chosen[id] ??= Object.create(null))[question] = [];
  }
  /** The answers as the client takes them. */
  const payload = (item: ApprovalItem) => approvalPayloadAnswers(item.questions, chosen[item.id], other[item.id]);
  /** A question is still unanswered, so the answers cannot be submitted. */
  function incomplete(item: ApprovalItem) {
    const answers = payload(item);
    return item.questions.some(question => !Object.hasOwn(answers, question.id));
  }
  return { chosen, other, clear, answer, toggle, typeOther, payload, incomplete };
}

export type ApprovalAnswers = ReturnType<typeof useApprovalAnswers>;
