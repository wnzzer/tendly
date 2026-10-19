export type ProviderKind = "claude_code" | "codex" | "pi" | "terminal";
export type AgentProviderKind = Exclude<ProviderKind, "terminal">;
export type SessionStatus = "starting" | "running" | "waiting" | "stopped" | "failed";
/** `main_model`: the session's main model ID at launch (how Claude Code's slot variables follow it); `main_model_context`: the context window set for that model. */
export type EnvironmentValue = { kind: "literal"; value: string } | { kind: "secret_ref"; reference: string } | { kind: "unset" } | { kind: "main_model" } | { kind: "main_model_context" };
export type EnvironmentOverrides = Record<string, EnvironmentValue>;

export interface Workspace {
  id: string;
  name: string;
  root_path: string;
  created_at: string;
}

export interface Session {
  interaction_mode?: "pty" | "structured";
  configuration_revision?: number;
  environment?: EnvironmentOverrides;
  id: string;
  workspace_id: string;
  provider: ProviderKind;
  title: string;
  status: SessionStatus;
  created_at: string;
  updated_at: string;
  /** Reversible list organization, independent of running status and history. */
  archived_at?: string | null;
  /** Chosen once at creation: closing its window discards the record and stops
   * its process. Only ever cleared (promoted), never set on an existing session. */
  ephemeral?: boolean;
  endpoint_profile_id: string | null;
  endpoint_snapshot?: EndpointProfile | null;
  provider_session_id: string | null;
  native_source_id?: string | null;
  /** Set on a terminal opened as an escape hatch out of a structured session:
   * the id of the session whose conversation it reopens. Such a session runs
   * the client interactively on the same conversation, for commands the
   * structured pipe cannot carry. */
  resume_source_id?: string | null;
  /** Source configuration revision pinned when a terminal continuation is created. */
  resume_configuration_revision?: number | null;
  /** A worktree of the workspace's repository this session runs in; absent means the workspace itself. */
  checkout_path?: string | null;
  checkout_branch?: string | null;
  /** The last thing the user said here, one short line, for telling sessions apart in a list. */
  preview?: string | null;
  error: string | null;
}

export interface EndpointProfile {
  environment?: EnvironmentOverrides;
  id: string;
  name: string;
  provider: ProviderKind;
  endpoint_url: string | null;
  model: string | null;
  permission_mode: "native" | "interactive" | "trusted" | "plan" | "blocked";
  secret_ref: string | null;
  proxy_url?: string | null;
  /** Reasoning effort default, when the client supports one. */
  effort?: string | null;
  model_aliases?: Record<string, string>;
  /** The endpoint models a session's menu offers; empty or absent offers all it serves. */
  models?: string[];
  /** Which API the endpoint speaks, for a client that speaks several; absent leaves it to the client. */
  api?: string | null;
  /** The header the endpoint reads its key from, `{key}` in the value; absent keeps the protocol's own. */
  key_header?: KeyHeader | null;
  /** A live reference to native client configuration, not copied configuration contents. */
  native_config?: { source_id: string; config_dir: string; config_env?: string | null } | null;
  created_at: string;
}

/** One request header carrying an endpoint's key: `x-api-key: {key}`, `Authorization: Bearer {key}`, `X-Token: {key}`. */
export interface KeyHeader { name: string; value: string }

export interface FileEntry { name: string; path: string; kind: "file" | "directory" | "symlink" | "other"; size: number; }
export interface GitFile { index: string; worktree: string; path: string; original_path?: string; }
/** `operation`: a merge, rebase, cherry-pick or revert stopped part-way. */
export type GitOperation = "merge" | "rebase" | "cherry-pick" | "revert";
export interface GitStatus { branch: string | null; files: GitFile[]; ahead?: number; behind?: number; upstream?: string; operation?: GitOperation; }
/** One commit in a log. `refs` are Git's decorations: `HEAD -> main`, `origin/main`, `tag: v1`. Times are in seconds. */
export interface GitCommit { hash: string; parents: string[]; author: string; email: string; date: number; refs: string[]; subject: string; }
export interface GitLog { commits: GitCommit[]; more: boolean; }
export interface GitChangedFile { status: string; path: string; original_path?: string; added?: number; removed?: number; untracked?: boolean; }
export interface GitCommitDetail { hash: string; parents: string[]; author: string; email: string; date: number; committer: string; committer_email: string; commit_date: number; refs: string[]; message: string; files: GitChangedFile[]; truncated: boolean; }
export interface GitRef { name: string; hash: string; subject: string; date: number; upstream?: string; ahead?: number; behind?: number; gone?: boolean; }
export interface GitWorktree { path: string; branch?: string | null; main: boolean; }
export interface GitBranches { current?: string | null; branches: string[]; worktrees: GitWorktree[]; local?: GitRef[]; remote?: GitRef[]; }
export interface GitRemote { name: string; url: string; push_url?: string; }
export interface GitStash { index: number; hash: string; message: string; date: number; untracked: boolean; }
export interface GitTag { name: string; hash: string; annotated: boolean; subject: string; date: number; }
/** How an operation that can stop part-way ended. */
export interface GitOutcome { conflicts: boolean; message: string; }
export interface GitDiff { diff: string; path: string | null; staged: boolean; binary: boolean; truncated: boolean; }
export interface TextFile { path: string; content: string; version: string; }
export interface NativeHistorySource { id:string; provider:AgentProviderKind; label:string; path:string; available:boolean; config_env?:string|null; }
export interface NativeHistoryItem { id:string; provider:AgentProviderKind; title:string; cwd:string; updated_at:string; imported_session_id:string|null; }
export interface ModelCatalog { models: Array<{id:string;name:string;efforts?:string[]}>; source_url: string; has_more: boolean; }
/** One model's answer from an explicit check: it ran, or why it did not. */
export type ModelProbeStatus = "ok" | "unauthorized" | "not_found" | "rate_limited" | "timeout" | "failed";
/**
 * The wire protocols a check can speak. A client with a choice (Pi) names one
 * per profile; the others have exactly one.
 */
export type ModelProbeApi = "anthropic-messages" | "openai-responses" | "openai-completions" | "google-generative-ai";
export interface ModelProbeResult {
  model: string;
  status: ModelProbeStatus;
  /** The protocol the check used: the same model can answer on one and fail on another. */
  api: ModelProbeApi;
  latency_ms?: number;
  detail?: string;
}
/** What a check would send, without sending it. */
export interface ModelProbePlan {
  /** The client that would send it. */
  client: string;
  api: ModelProbeApi;
  /** The path under the endpoint, e.g. `/v1/messages`. */
  path: string;
  /** Whether the profile named no API and this one was guessed from the URL. */
  guessed: boolean;
}
export type WorkspaceLayout = Record<string, unknown>;
export * from "./layout";
