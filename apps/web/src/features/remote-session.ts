import type { InjectionKey } from "vue";
import type { PaneNode, ProviderKind } from "@tendly/protocol";

/** Under a paired machine's session: what needs this machine's own dialogs is not offered. */
export const IS_REMOTE_PANE: InjectionKey<boolean> = Symbol("remote-pane");

/**
 * The tab for a session of a paired machine. It says `remote_session_id`, not
 * `session_id`: the canvas holds this machine's sessions under that name, and
 * the server refuses one it does not know.
 */
export function remotePane(machine: { id: string; name: string }, session: { id: string; title: string; provider: string }): PaneNode {
  return { type: "pane", id: `remote-${session.id}`, kind: session.provider === "terminal" ? "terminal" : "agent_chat", title: session.title,
    metadata: { machine_id: machine.id, machine_name: machine.name, remote_session_id: session.id, provider: session.provider as ProviderKind } };
}
