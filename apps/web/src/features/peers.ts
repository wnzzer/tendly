import { json, request } from "./api";

/** /api/peers (crates/server/src/peers.rs): other machines this one is paired with. */
export type Grant = "view" | "spawn" | "control";
export const GRANTS: readonly Grant[] = ["view", "spawn", "control"];
export const GRANT_LABELS: Record<Grant, string> = {
  view: "See its sessions",
  spawn: "Start sessions too",
  control: "Full control",
};
export const GRANT_NOTES: Record<Grant, string> = {
  view: "Workspace and session names and status. Never conversations.",
  spawn: "Also start sessions and follow them: what its agents use tendly_spawn with machine for.",
  control: "Also open this machine's window from there, signed in, with everything that allows.",
};

export interface Peer {
  id: string;
  name: string;
  url: string | null;
  /** What it may do here. */
  grant: Grant;
  /** What it lets this machine do there, as it last said. */
  granted: Grant | null;
  paired_at: string;
  /** `null` when it has no address this machine can reach. */
  online?: boolean | null;
  version?: string | null;
  problem?: string | null;
}
export interface Peers { name: string; peers: Peer[] }
export interface Invite { link: string; code: string; url: string; expires_in: number }
export interface Discovered {
  name: string; url: string; os: string; tendly: boolean; version: string | null; pairs: boolean; paired: boolean;
  /** Signed in to the same Tailscale account as this machine: it pairs without an invite. */
  mine: boolean;
}
export interface RemoteSession { id: string; title: string; provider: string; status: string; workspace_id: string; branch: string | null; activity: string; temporary?: boolean }
export interface RemoteSessions { name: string; grant: Grant; workspaces: { id: string; name: string }[]; sessions: RemoteSession[] }

const origin = () => typeof location === "undefined" ? undefined : location.origin;
export const readPeers = () => request<Peers>("/peers");
export const invitePeer = (grant: Grant) => request<Invite>("/peers/invite", json("POST", { grant, origin: origin() }));
export const addPeer = (invite: string, grant: Grant) => request<Peer>("/peers", json("POST", { invite, grant, origin: origin() }));
/** Pair with a machine of the same Tailscale account: both grants are decided here. */
export const pairOnTailnet = (url: string, grant: Grant, asked: Grant) => request<Peer>("/peers", json("POST", { url, grant, asked, origin: origin() }));
export const regrantPeer = (id: string, grant: Grant) => request<Peer>(`/peers/${encodeURIComponent(id)}`, json("PUT", { grant }));
export const removePeer = (id: string) => request<void>(`/peers/${encodeURIComponent(id)}`, json("DELETE"));
export const peerSessions = (id: string) => request<RemoteSessions>(`/peers/${encodeURIComponent(id)}/sessions`);
export const openPeer = (id: string, sessionId?: string) => request<{ url: string }>(`/peers/${encodeURIComponent(id)}/open`, json("POST", sessionId ? { session_id: sessionId } : {}));
/** Its own window, at this session, signed in by a ticket good once. Opened first so no popup blocker takes it. */
export async function openPeerWindow(id: string, sessionId?: string) {
  const tab = window.open("", "_blank");
  try {
    const { url } = await openPeer(id, sessionId);
    if (tab) { tab.opener = null; tab.location.href = url; } else window.location.href = url;
  } catch (cause) { tab?.close(); throw cause; }
}
export const discoverPeers = () => request<Discovered[]>("/peers/discover");

/** GET /api/peers/sessions: every paired machine's sessions, each asked for at once. */
export interface MachineSessions {
  id: string; name: string; granted: Grant | null;
  /** `null` when it has no address this machine can reach. */
  online: boolean | null; problem?: string;
  workspaces: { id: string; name: string }[]; sessions: RemoteSession[];
}
export const allPeerSessions = () => request<{ machines: MachineSessions[] }>("/peers/sessions");

/**
 * How a session there is doing. A chat session says so itself; a terminal one
 * says only whether its process runs.
 */
export type RemoteState = "waiting" | "working" | "idle" | "stopped";
export function remoteState(session: RemoteSession): RemoteState {
  if (session.activity === "waiting_for_approval" || session.status === "waiting") return "waiting";
  if (session.activity === "working" || session.activity === "starting") return "working";
  if (session.activity === "idle") return "idle";
  return session.status === "running" || session.status === "starting" ? "working" : "stopped";
}
export interface MachineRow extends RemoteSession { workspace: string; state: RemoteState }
/**
 * A machine as the sidebar shows it: the sessions the search matches, those
 * waiting or working first, the rest newest first as the machine listed them,
 * and how many more there are than fit.
 */
export function sidebarMachine(machine: MachineSessions, query = "", limit = 8) {
  const needle = query.trim().toLocaleLowerCase();
  const names = new Map(machine.workspaces.map(workspace => [workspace.id, workspace.name]));
  const all: MachineRow[] = machine.sessions
    .filter(session => !session.temporary)
    .map(session => ({ ...session, workspace: names.get(session.workspace_id) ?? "", state: remoteState(session) }));
  const matches = !needle || machine.name.toLocaleLowerCase().includes(needle)
    ? all
    : all.filter(row => row.title.toLocaleLowerCase().includes(needle) || row.workspace.toLocaleLowerCase().includes(needle));
  const rank: Record<RemoteState, number> = { waiting: 0, working: 1, idle: 2, stopped: 2 };
  const ordered = matches.map((row, index) => ({ row, index }))
    .sort((a, b) => rank[a.row.state] - rank[b.row.state] || a.index - b.index)
    .map(({ row }) => row);
  return {
    rows: ordered.slice(0, limit),
    more: Math.max(0, ordered.length - limit),
    waiting: all.filter(row => row.state === "waiting").length,
    working: all.filter(row => row.state === "working").length,
    /** A search that matches nothing here hides the machine. */
    hidden: Boolean(needle) && !matches.length,
  };
}

/**
 * What a machine found on the tailnet offers: pairing in one click when it is
 * the same account's, an invite when it is someone else's.
 */
export type FoundAction = "paired" | "update" | "pair" | "invite";
export function foundAction(machine: Discovered): FoundAction {
  if (machine.paired) return "paired";
  if (!machine.pairs) return "update";
  return machine.mine ? "pair" : "invite";
}

/** The one-time ticket a link from another paired machine carries (`#tendly-ticket=…`). */
export const TICKET_FRAGMENT = "tendly-ticket";
export function ticketFromHash(hash: string): string | null {
  const value = new URLSearchParams(hash.replace(/^#/, "")).get(TICKET_FRAGMENT);
  return value?.trim() || null;
}
/** Whether a pasted text is an invite link at all, before asking the server. */
export function looksLikeInvite(text: string): boolean {
  try { return new URLSearchParams(new URL(text.trim()).hash.replace(/^#/, "")).has("tendly-pair"); }
  catch { return false; }
}
