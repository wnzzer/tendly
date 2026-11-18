import { test } from "node:test";
import assert from "node:assert/strict";
import { ApiConnectionError, ApiError } from "./api.ts";
import { deliveryUncertain, pendingMessage, settleAccepted, settleFailed } from "./message-delivery.ts";

const session = { endpoint_profile_id: "work", configuration_revision: 3 };
const sending = (text = "hello") => ({ text, pending: pendingMessage("m1", text.trim(), text, session) });

test("a pending message remembers the endpoint and configuration it was sent under", () => {
  assert.deepEqual(pendingMessage("m1", "hi", " hi", session), {
    id: "m1",
    content: "hi",
    text: " hi",
    state: "sending",
    profileId: "work",
    configurationRevision: 3,
  });
  assert.equal(pendingMessage("m1", "hi", "hi", { endpoint_profile_id: null }).configurationRevision, 0);
});

test("only a refusal is known not to have arrived", () => {
  assert.equal(deliveryUncertain(new ApiError("Bad request", 400)), false);
  assert.equal(deliveryUncertain(new ApiError("Conflict", 409)), false);
  assert.equal(deliveryUncertain(new ApiError("Server error", 500)), true);
  assert.equal(deliveryUncertain(new ApiError("Redirect", 302)), true);
  assert.equal(deliveryUncertain(new ApiConnectionError("POST")), true);
  assert.equal(deliveryUncertain(new Error("no status")), true);
  assert.equal(deliveryUncertain("thrown string"), true);
});

test("an accepted message leaves the composer, and its own receipt clears it", () => {
  const draft = sending();
  settleAccepted(draft, "m1", "hello", { accepted: true, id: "m1" });
  assert.equal(draft.text, "");
  assert.equal(draft.pending, undefined);
  assert.equal(draft.notice, undefined);
});

test("a receipt for another message leaves this one waiting for its event", () => {
  const draft = sending();
  settleAccepted(draft, "m1", "hello", { accepted: true, id: "older" });
  assert.equal(draft.text, "");
  assert.equal(draft.pending.state, "unknown");
  assert.match(draft.notice, /Message accepted/);
});

test("text typed after sending is kept when the send is accepted", () => {
  const draft = sending();
  draft.text = "next thought";
  settleAccepted(draft, "m1", "hello", { accepted: true, id: "m1" });
  assert.equal(draft.text, "next thought");
});

test("a refused message goes back into an empty composer", () => {
  const draft = sending();
  draft.text = "";
  settleFailed(draft, "m1", "hello", new ApiError("Bad request", 400));
  assert.equal(draft.pending, undefined);
  assert.equal(draft.text, "hello");
  const typed = sending();
  typed.text = "newer";
  settleFailed(typed, "m1", "hello", new ApiError("Bad request", 400));
  assert.equal(typed.text, "newer");
});

test("a message that may have arrived stays pending and is not resent", () => {
  const draft = sending();
  settleFailed(draft, "m1", "hello", new ApiConnectionError("POST"));
  assert.equal(draft.pending.state, "unknown");
  assert.match(draft.notice, /Delivery is uncertain/);
});

test("a failure for a message already replaced changes nothing", () => {
  const draft = sending();
  settleFailed(draft, "other", "hello", new ApiError("Bad request", 400));
  assert.equal(draft.pending.id, "m1");
  assert.equal(draft.pending.state, "sending");
});
