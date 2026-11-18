/**
 * What becomes of a message once it leaves the composer: the pending record
 * the draft keeps while it is in flight, and what the draft holds once the
 * server has answered or the request has failed.
 */
import type { Session } from "@tendly/protocol";
import { ApiConnectionError } from "./api";
import { acknowledgeReceipt, type ChatDraft } from "./chat-model";

type Pending = NonNullable<ChatDraft["pending"]>;

/** A message on its way, tied to the endpoint and configuration it was sent under. */
export function pendingMessage(
  id: string,
  content: string,
  text: string,
  session: Pick<Session, "endpoint_profile_id" | "configuration_revision">,
): Pending {
  return {
    id,
    content,
    text,
    state: "sending",
    profileId: session.endpoint_profile_id,
    configurationRevision: session.configuration_revision ?? 0,
  };
}

/**
 * Whether a failed send may still have reached the client. A refusal (a 4xx)
 * did not; a lost connection, a server error or anything unrecognised might
 * have, so the message is never sent again on its own.
 */
export function deliveryUncertain(cause: unknown): boolean {
  if (cause instanceof ApiConnectionError || !(cause instanceof Error) || !("status" in cause)) return true;
  const status = Number(cause.status);
  return status >= 500 || status < 400;
}

/**
 * The server took the message. The composer lets go of what was sent; a
 * receipt for some other message leaves this one waiting for its event.
 */
export function settleAccepted(draft: ChatDraft, messageId: string, content: string, receipt: unknown) {
  acknowledgeReceipt(draft, receipt);
  if (draft.text === content) draft.text = "";
  // A durable receipt can acknowledge an older, already-trimmed message.
  // Missing receipts still need the canonical event; never replay automatically.
  if (draft.pending?.id === messageId) {
    draft.pending.state = "unknown";
    draft.notice = "Message accepted. Refresh the conversation if its event has not appeared.";
  }
}

/**
 * The send failed. One that may have arrived stays pending, marked uncertain;
 * one that was refused goes back into an empty composer.
 */
export function settleFailed(draft: ChatDraft, messageId: string, content: string, cause: unknown) {
  if (draft.pending?.id !== messageId) return;
  if (deliveryUncertain(cause)) {
    draft.pending.state = "unknown";
    draft.notice =
      "Delivery is uncertain. Your draft is kept; refresh the conversation before deciding what to do. It will not be sent again automatically.";
  } else {
    draft.pending = undefined;
    if (!draft.text) draft.text = content;
  }
}
