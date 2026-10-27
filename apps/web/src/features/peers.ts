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
export interface Discovered { name: string; url: string; os: string; tendly: boolean; version: string | null; pairs: boolean; paired: boolean }
export interface RemoteSession { id: string; title: string; provider: string; status: string; workspace_id: string; branch: string | null; activity: string }
export interface RemoteSessions { name: string; grant: Grant; workspaces: { id: string; name: string }[]; sessions: RemoteSession[] }

const origin = () => typeof location === "undefined" ? undefined : location.origin;
export const readPeers = () => request<Peers>("/peers");
export const invitePeer = (grant: Grant) => request<Invite>("/peers/invite", json("POST", { grant, origin: origin() }));
export const addPeer = (invite: string, grant: Grant) => request<Peer>("/peers", json("POST", { invite, grant, origin: origin() }));
export const regrantPeer = (id: string, grant: Grant) => request<Peer>(`/peers/${encodeURIComponent(id)}`, json("PUT", { grant }));
export const removePeer = (id: string) => request<void>(`/peers/${encodeURIComponent(id)}`, json("DELETE"));
export const peerSessions = (id: string) => request<RemoteSessions>(`/peers/${encodeURIComponent(id)}/sessions`);
export const openPeer = (id: string, sessionId?: string) => request<{ url: string }>(`/peers/${encodeURIComponent(id)}/open`, json("POST", sessionId ? { session_id: sessionId } : {}));
export const discoverPeers = () => request<Discovered[]>("/peers/discover");

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
