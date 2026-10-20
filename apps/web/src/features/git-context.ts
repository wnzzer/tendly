import { inject, type InjectionKey, type Ref } from "vue";
import type { GitOutcome, GitRemote, GitStatus } from "@tendly/protocol";
import { json, request } from "./api";
import { placeUrl } from "./checkout-place";

export interface PromptField { key: string; label: string; value?: string; placeholder?: string; multiline?: boolean; required?: boolean; hint?: string; }
export interface PromptCheck { key: string; label: string; value?: boolean; hint?: string; }
export interface PromptChoice { key: string; label: string; value: string; options: { value: string; label: string; hint?: string }[]; }
/** A small form or a confirmation, asked in a dialog. `danger` paints the
 * confirming button red: it is for what cannot be taken back. */
export interface PromptSpec { title: string; description?: string; fields?: PromptField[]; checks?: PromptCheck[]; choices?: PromptChoice[]; confirm: string; danger?: boolean; }
export type PromptAnswer = Record<string, string | boolean>;

/**
 * What the Git pane shares with its views: the status everyone reads, one
 * `run` for every action -- so one thing happens at a time, errors land in
 * one place and the status refreshes after -- and the dialog that asks.
 */
export interface GitContext {
  /** The workspace and checkout the pane shows (`placeKey`). */
  place: () => string;
  status: Ref<GitStatus>;
  remotes: Ref<GitRemote[]>;
  busy: Ref<boolean>;
  /** Bumped after anything that may move history, so views reload. */
  revision: Ref<number>;
  /** On a phone: a detail is open over the list. */
  detailOpen: Ref<boolean>;
  run<T>(work: () => Promise<T>, success?: string | ((result: T) => string | undefined)): Promise<T | undefined>;
  refresh(): Promise<void>;
  ask(spec: PromptSpec): Promise<PromptAnswer | undefined>;
  openFile(path: string): void;
  showHistory(path?: string): void;
  copy(text: string): Promise<void>;
}
export const GIT_CONTEXT: InjectionKey<GitContext> = Symbol("git");
export function useGit(): GitContext {
  const context = inject(GIT_CONTEXT);
  if (!context) throw new Error("Git views live inside the Git pane");
  return context;
}

/** `/api/workspaces/:id/git/<path>` in a place's checkout, GET or POST. */
export function gitGet<T>(place: string, path: string) { return request<T>(placeUrl(place, `/git/${path}`)); }
export function gitPost<T = void>(place: string, path: string, body: unknown = {}) { return request<T>(placeUrl(place, `/git/${path}`), json("POST", body)); }
export const isOutcome = (value: unknown): value is GitOutcome => !!value && typeof value === "object" && "conflicts" in value;
export const shortHash = (hash: string) => hash.slice(0, 8);
