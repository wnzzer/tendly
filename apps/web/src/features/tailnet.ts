import { json, request } from "./api";

/** GET/PUT /api/tailnet (crates/server/src/tailscale.rs). */
export interface TailnetProblem {
  code: string;
  detail: string;
  command: string | null;
  link: string | null;
}
export interface TailnetMachine {
  name: string;
  ip: string | null;
  login: string | null;
  tailnet: string | null;
  https: boolean;
}
export interface TailnetStatus {
  enabled: boolean;
  forced: boolean;
  phase: "off" | "starting" | "live" | "error";
  url: string | null;
  machine: TailnetMachine | null;
  verified: boolean | null;
  checked_at: number | null;
  problem: TailnetProblem | null;
  target: string;
  via_tailnet: boolean;
}

export const readTailnet = () => request<TailnetStatus>("/tailnet");
export const switchTailnet = (enabled: boolean) => request<TailnetStatus>("/tailnet", json("PUT", { enabled }));
export const recheckTailnet = () => request<TailnetStatus>("/tailnet/check", json("POST"));
export const tailnetLoginCode = () => request<{ link: string; svg: string }>("/tailnet/login-code");

/**
 * The way from nothing to a phone that opens the workspace, in the order the
 * server walks it. Each problem the server reports belongs to one step, so the
 * page can say exactly where it stopped.
 */
export const TAILNET_STEPS = ["install", "connect", "name", "publish", "verify"] as const;
export type TailnetStep = (typeof TAILNET_STEPS)[number];
export type StepState = "done" | "active" | "failed" | "warning" | "pending";

const STEP_OF_PROBLEM: Record<string, TailnetStep> = {
  not_installed: "install",
  outdated: "install",
  daemon_down: "connect",
  not_running: "connect",
  no_magic_dns: "name",
  serve_consent: "publish",
  no_https: "publish",
  port_taken: "publish",
  access_denied: "publish",
  failed: "publish",
  unreachable: "verify",
};

export function tailnetSteps(status: TailnetStatus): { id: TailnetStep; state: StepState }[] {
  const mark = (reached: number, at: StepState) =>
    TAILNET_STEPS.map((id, index) => ({
      id,
      state: (index < reached ? "done" : index === reached ? at : "pending") as StepState,
    }));
  switch (status.phase) {
    case "off":
      return mark(-1, "pending");
    // A known name means the CLI ran, the tailnet is up and MagicDNS answered.
    case "starting":
      return mark(status.machine ? 3 : 0, "active");
    case "live":
      return mark(4, status.verified === false ? "warning" : status.verified ? "done" : "active");
    case "error":
      return mark(TAILNET_STEPS.indexOf(STEP_OF_PROBLEM[status.problem?.code ?? ""] ?? "publish"), "failed");
  }
}

/** The token a sign-in link carries in its fragment (`#tendly-token=…`). */
export const LOGIN_FRAGMENT = "tendly-token";
export function tokenFromHash(hash: string): string | null {
  const value = new URLSearchParams(hash.replace(/^#/, "")).get(LOGIN_FRAGMENT);
  return value?.trim() || null;
}
