/**
 * How a client's request for permission is shown: which of its choices the
 * pane offers, what they are called, and its text split into the reason the
 * client gave and the detail behind it.
 */

/** The answers the pane can give a request. A client may offer others; those are left out. */
export type ApprovalDecision = "accept" | "accept_session" | "decline" | "cancel";

const DECISIONS: readonly string[] = ["accept", "accept_session", "decline", "cancel"];

/** The choices a request offered that the pane knows how to give, in the client's order. */
export function approvalChoices(item: { choices: readonly string[] }): ApprovalDecision[] {
  return item.choices.filter(choice => DECISIONS.includes(choice)) as ApprovalDecision[];
}

export const DECISION_LABEL: Record<ApprovalDecision, string> = {
  accept: "Allow once",
  accept_session: "Allow for this session",
  decline: "Decline",
  cancel: "Cancel turn",
};
export const ACCESS_LABEL: Record<string, string> = {
  read: "Read",
  write: "Write",
  execute: "Run a command",
  network: "Network access",
};

/**
 * A native request's text is usually the client's own JSON. Its reason reads
 * as a sentence; the rest is detail, folded away rather than shown raw. Text
 * that is not a JSON object has neither, and is shown as it is.
 */
export function approvalBody(text: string): { reason?: string; detail?: string } | undefined {
  try {
    const parsed: unknown = JSON.parse(text);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return undefined;
    const { reason, ...rest } = parsed as Record<string, unknown>;
    const detail = Object.fromEntries(
      Object.entries(rest).filter(([, value]) => value !== undefined && value !== null),
    );
    return {
      reason: typeof reason === "string" && reason.trim() ? reason : undefined,
      detail: Object.keys(detail).length ? JSON.stringify(detail, null, 2) : undefined,
    };
  } catch {
    return undefined;
  }
}
