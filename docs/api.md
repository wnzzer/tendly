# Tendly MVP API

Default origin: `http://127.0.0.1:28789`. Node-run Vite proxies the same `/api` surface, including WebSocket upgrades. The development toolchain is Node 24+ and pnpm 10.30.3; see the README for install/run commands.

All mutations require `X-Tendly-Client: web`. Unknown Host and untrusted Origin are rejected. When `TENDLY_TOKEN` is configured, private routes additionally require a bearer token or the HttpOnly login cookie.

```text
GET  /api/health
GET  /api/auth                 -> {required,authenticated}
POST /api/auth {token}         -> HttpOnly SameSite=Strict session cookie
```

## Workspaces and sessions

```text
GET  /api/workspaces
POST /api/workspaces {name,root_path}
GET  /api/workspaces/:id
PATCH /api/workspaces/:id {worktree_setup}
GET  /api/sessions?workspace_id=:id
POST /api/workspaces/:id/sessions {provider,title,endpoint_profile_id?,model?,effort?,environment?,interaction_mode?,ephemeral?,handoff_source_id?}
GET  /api/sessions/:id
PATCH /api/sessions/:id {title}
DELETE /api/sessions/:id
POST /api/sessions/:id/keep
PATCH /api/sessions/:id/environment {environment}
POST /api/sessions/:id/start
POST /api/sessions/:id/stop
POST /api/sessions/:id/launch-plan?force=true   (tendly resume; this machine only)
POST /api/sessions/:id/message {content}
WS   /api/sessions/:id/pty/ws?cols=120&rows=40
```

Providers: `claude_code`, `codex`, `terminal`. Create records a stopped session and an immutable `endpoint_snapshot`; profile environment defaults plus explicit request overrides form the session's effective `environment` map. A new isolated/custom-profile session may override its model; native-reference profiles keep the native model setting. Start is idempotent for a running process. Stop is idempotent for an existing stopped session. Clients cannot PATCH runtime states. The UI calls start when the user opens/reopens a session, coalescing concurrent requests; ordinary view remounts never revive stopped sessions. End is a secondary, confirmed menu action. These lifecycle APIs remain available even though primary Start/Stop buttons were removed. Opening environment settings from the sidebar gear or pane menu only inspects configuration and never starts a session.

`POST /api/sessions/:id/launch-plan` is what `tendly resume <id>` runs: `{program,args,cwd,env,env_remove,title,provider,provider_session_id}` for continuing the session's conversation in a terminal of one's own, built by the same launch layer as an escape-hatch terminal (pinned configuration revision, checkout, endpoint, model, effort, permission and resume selector) without recording a session. Because `env` carries the endpoint's resolved key, it answers only connections from a loopback address, and takes `X-Tendly-Client: cli` (or `web`) plus the usual authentication. A session Tendly is running is refused with 409 unless `force=true`; a client session without a native conversation yet is refused with 409; a terminal session opens its shell in the session's directory.

`PATCH /api/sessions/:id {title}` changes the Tendly display title and returns the same session record. The session ID, workspace, native resume identity, conversation history and live process are unchanged. The title is propagated to every open canvas tab bound to that session. For a newly started or explicitly reopened isolated Claude Code session, Tendly also forwards the title as Claude Code's native `--name`; imported/native-reference sessions keep their original client-owned naming and history identity. Blank or oversized titles are rejected.

`ephemeral` marks a temporary session and can only be set at creation. Closing its
canvas window discards it — the one place where closing a view ends a process, allowed
only because the user opted in up front. A terminal goes with its tab at once; only a
conversation in the middle of a turn asks first. A restart clears temporary sessions
instead of leaving them as stopped records. The persisted layout keeps a temporary
window's pane, marked `metadata.ephemeral`, so a refresh keeps it open and closable; a
marked pane whose session is gone is dropped on load rather than resurrected.

`DELETE /api/sessions/:id` stops the session and removes the record, returning `{id}`.
It returns `409` for a permanent session: a session that was safe to keep can never be
destroyed by closing a window. Archive a permanent session instead.

`POST /api/sessions/:id/keep` promotes a temporary session to a permanent one and
returns the updated record. It is deliberately one-way — there is no request shape that
makes an existing session temporary, so the destructive path can never be opened on a
session the user has been treating as durable.

`handoff_source_id` optionally copies a source conversation into a new structured session. The server persists the history and includes up to 64 KiB of recent conversation text with the first message, using the same mechanism as endpoint changes. Tool output is excluded; `/clear` boundaries are respected.

`interaction_mode` is `"pty"` or `"structured"`. The HTTP default remains `pty` for backwards compatibility; the current Web client requests `structured` for new Agent sessions when the backend advertises it. Terminal cannot use structured mode. `/start` dispatches to the session's selected runtime. `/message {content}` is the legacy PTY input endpoint and rejects structured sessions; use the receipt-bearing conversation endpoint below instead.

The WebSocket only **attaches**; it never starts/restarts a process. Binary frames carry raw terminal data. Client text controls:

```json
{"type":"input","data":"..."}
{"type":"resize","cols":120,"rows":40}
```

Server text controls: `exit` with exit code, `gap` when output was lost, or `error` with message. A reconnect that names the process and last frame it drew resumes from the next frame while the bounded 1 MiB replay still holds it; otherwise the view is reset and redrawn from that replay. Session records/config directories persist across server restarts, not the live PTY. Input is capped at 64 KiB; sizes at 1000×500 cells.

The UI distinguishes backend process state from stream state and limits the WebSocket handshake to 8 seconds. A dropped or silent view is re-attached on its own with backoff (0.3 s up to 10 s, eight tries in a row), and a quiet one is probed with a ping every 15 s while the page is in front (`features/session-stream.ts`); after the last try it shows an explicit reconnect action. Only the view is retried: it never starts/stops a process or replays user input. Replaying terminal **output** on an explicit attach is separate from replaying input. In the observed macOS case, Node-run Vite fixed a Bun-run Vite proxy handshake timeout; the frontend runtime change did not require stopping the existing backend or sessions.

The unscoped `/api/pty/ws` and arbitrary state PATCH endpoints have been removed.

## Structured conversations

```text
GET  /api/sessions/:id/conversation
POST /api/sessions/:id/conversation/open
POST /api/sessions/:id/conversation/message {id,content,configuration_revision?}
POST /api/sessions/:id/conversation/interrupt
POST /api/sessions/:id/conversation/approval {request_id,decision,answers?}
PATCH /api/sessions/:id/configuration {endpoint_profile_id,model?,effort?,confirmed:true}
WS   /api/sessions/:id/chat/ws
```

`open` returns a Session. It initializes or attaches the supervised bridge without sending a prompt. Explicitly opting in can convert a stopped/failed Agent PTY record to structured mode; it never takes over a running PTY. Ordinary chat WebSocket attachment only reads events and does not open a native process.

`message` requires a UUID `id` and 1–32768 bytes of nonblank text without NUL. It returns **202 Accepted**, not a guarantee that a model completed or even received the request. The UUID/content-hash receipt and user event are committed atomically before dispatch. An identical retry returns 202 without re-dispatch, including after restart or configuration switch; the same ID with different text returns 409. Dispatch failures remain visible and never cause automatic input replay. A caller must inspect the outcome before explicitly submitting different work with a new UUID. Only one active turn is allowed.

Message, interruption and approval acknowledgements have a JSON body: `{accepted:true,id:string|null,duplicate:boolean}`. A message receipt uses its submitted UUID, so a manual retry can confirm an already-accepted message even after its display event was trimmed. It does not claim model success. The Web client also pins `configuration_revision` on each send/retry; a stale revision returns 409 before a new dispatch, preventing another window's configuration change from redirecting an old draft. Already-accepted identical receipts remain acknowledgeable without dispatch regardless of the current revision. API clients should send the revision from the session record as well.

`interrupt` returns 202 and forwards a native turn interruption; it is not session shutdown or a pause/resume implementation. To terminate the supervised process, use the separately confirmed session end action (`POST /sessions/:id/stop`).

`approval` returns 202 after queueing a response to a currently pending native request. `decision` must be one of that request's offered `accept`, `decline`, `cancel` choices; expired/unsupported decisions return 409. `answers` maps question IDs to arrays of selected/free-text answers. Native rules remain authoritative: Tendly does not grant session-wide permissions or silently accept unsupported interactions. Validation failures do not settle the native request. Do not infer model completion from approval submission.

HTTP conversation responses and the first WebSocket message use:

```json
{"type":"snapshot","mode":"structured","running":true,"events":[],"truncated":false}
```

The WebSocket then emits sequenced stored events. Subscribe-before-snapshot plus monotonic `seq` lets clients ignore duplicates. A lagged consumer receives a replacement snapshot rather than silently missing output. The socket is a read-only event transport (plus Ping/Pong); messages, approvals and interruptions use HTTP mutation routes.

Event bodies are listed below; persisted events also have `seq`:

```ts
{ type: "ready"; native_session_id?: string; commands?: string[] }
{ type: "message"; id: string; role: "user" | "assistant"; text: string; delta?: boolean }
{ type: "tool"; id: string; name: string; status: "running" | "completed" | "failed"; text?: string }
{ type: "approval"; id: string; title: string; text: string; choices: string[];
  questions?: Array<{ id: string; header?: string; question: string;
    options: Array<{ label: string; description?: string }>;
    multiSelect?: boolean; isSecret?: boolean; isOther?: boolean }> }
{ type: "approval_resolved"; id: string }
{ type: "turn"; id?: string; status: "running" | "completed" | "failed" | "interrupted" }
{ type: "usage"; input_tokens?: number; output_tokens?: number; context_tokens?: number; context_window?: number }
{ type: "error"; message: string }
{ type: "exit" }
{ type: "configuration"; id: string; profile_name: string; text: string }
```

`delta:true` appends assistant text; `delta:false` replaces that message's text. Tool text and usage are snapshots, not values to add repeatedly. `configuration` is generated by the backend, not a native CLI event. The bridge emits initial `ready` without a native ID when none exists; a later ready event records the native session ID. Bridge stderr is not forwarded. Text fields are bounded at 64 KiB with a truncation marker, and final serialized Node output frames at 192 KiB. Rust accepts at most 256 KiB per event before its own validation/persistence.

SQLite schema 9 keeps a recent 2,000-event / 8 MiB display window. Latest control anchors and up to 32 unresolved approval cards are retained separately and merged back by sequence, so the complete snapshot may exceed that window. A truncated history is explicitly marked. Submission receipts retain UUIDs and SHA-256 content hashes, not a second permanent plaintext prompt copy, and remain available to prevent replay after display-window trimming. Transcript text itself may contain sensitive user/native output; protect the database and its backups.

### Confirmed configuration changes

`PATCH /sessions/:id/configuration` requires `confirmed:true`, a profile from the same CLI (or `endpoint_profile_id:null` for isolation), and optional model/effort overrides only where supported. A running PTY is rejected. Structured sessions must have no active turn or unresolved approval; the old bridge is stopped before switching. Native-reference profiles keep their original model and reasoning settings.

The transaction resets the native conversation/history association, increments `configuration_revision`, and replaces the effective environment with the new profile's defaults (or empty defaults). Generated isolated configuration lives in the new revision's directory, not the old credential home. Earlier UI history and receipts remain, but are not sent to the new endpoint; the next explicit message starts a fresh native context. No account is switched silently, and this endpoint does not start a new model turn.

Already-imported native history currently supplies CLI resume context only. The structured API **does not backfill its older full transcript**. See [the implementation boundary](structured-agent-ui.md).

## Load native history

```text
GET  /api/native-history/sources
GET  /api/workspaces/:id/native-history?source_id=codex-default
POST /api/workspaces/:id/native-history/import
     {source_id,native_id,confirmed_original_config:true,environment?}
```

Sources are server-configured, not arbitrary client-supplied paths. Default IDs are `codex-default` and `claude-default`. Source views return `{id,provider,label,path,available}`. Listings return `{items:[{id,provider,title,cwd,updated_at,imported_session_id}],truncated}`. Search is fuzzy matching on these loaded metadata fields, not transcript search.

Codex uses native app-server `thread/list`; Claude uses the official Agent SDK's metadata-only `listSessions` with the exact workspace directory. No inference, native resume or account copying occurs during listing/import. Import revalidates source membership and canonical workspace path, is idempotent, and requires explicit original-configuration confirmation. Optional `environment` sets explicit overrides for a newly imported record. Omitting it preserves the effective map of an existing import; passing a different explicit map returns 409 without changing that session. An empty object is an explicit map, not equivalent to omission. The UI omits the field when no history overrides are entered.

The imported stopped session has `native_source_id`, `provider_session_id` and its explicit `environment`; its pinned configuration path is internal and excluded from session JSON. PTY opening uses `codex resume ID` or `claude --resume ID` with the original native configuration, without adding permission/model/endpoint overrides. Structured mode adapts the original client to stdio transport; Codex calls `thread/resume` only for the first explicit message. Explicit environment overrides affect the child process, not native configuration files. Changing the source directory fails closed rather than applying another account to the old association. This links a saved native context, not an external live PTY, and older full message bodies are not yet imported into the structured Web transcript. SQLite schema 3 added the association; schema 6 adds environment maps with empty defaults.

The helper runs under **Node.js** by default. `TENDLY_JS_RUNTIME` may override the executable; the legacy `TENDLY_NODE_BIN` is used only when the new override is unset/empty. The helper has time/output/page limits and returns explicit unsupported-client errors. See the [Codex app-server API](https://developers.openai.com/codex/app-server/) and [Claude Agent SDK metadata helpers](https://code.claude.com/docs/en/agent-sdk/typescript).

## Explicit environment overrides

`EndpointProfile.environment` is a template; `Session.environment` is the effective explicit map, not a snapshot of the host environment. Both return `{}` for migrated records without overrides. The map has this JSON-compatible shape:

```ts
type EnvironmentValue =
  | { kind: "literal"; value: string }
  | { kind: "secret_ref"; reference: string }
  | { kind: "unset" };
type EnvironmentOverrides = Record<string, EnvironmentValue>;
```

New sessions merge profile defaults first and request overrides second, validate the combined result, and snapshot it. Later profile changes do not alter that effective map. The original endpoint snapshot remains unchanged when session environment settings are edited.

`PATCH /api/sessions/:id/environment {environment}` requires a complete map and **replaces**, rather than merges, the existing effective map. Saving is permitted only for `stopped` / `failed` records whose runtime is not live. The handler shares the start/stop operations lock and checks persisted status again during the update; conflicts return 409. It never starts/stops/restarts a process. The sidebar settings gear and pane menu can open this editor without opening/launching the session; live sessions can inspect it read-only.

The native/custom/history launch builders receive this overlay after constructing the base launch configuration:

- `literal` assigns the exact string without shell expansion. `$PATH`, `${NAME}`, `~` and command-looking text remain literal environment contents. Plain values are stored in local metadata and returned to the editor: they are **not a secret store**.
- `secret_ref` accepts only `env:TENDLY_SECRET_` followed by one or more uppercase ASCII letters, digits or underscores. The backend resolves that reference only at launch. Missing or invalid resolved values fail launch without returning the value. References, not their resolved secrets, are persisted and exposed through the API.
- `unset` removes the variable from the child launch environment. It differs from deleting an override: deleting a new-session override reveals the profile default, while deleting an entry from an existing session's replacement map leaves the base launch environment to apply. Native config files may still supply their own settings according to the CLI's precedence.

Validation is shared by profile, session, import and launch paths:

- At most 64 entries and 64 KiB of serialized JSON. Keys match `[A-Za-z_][A-Za-z0-9_]{0,127}`. Literal and resolved secret values are at most 8192 UTF-8 bytes, with no NUL.
- Reserved keys, case-insensitively: `HOME`, `USERPROFILE`, `PWD`, `OLDPWD`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `CLAUDECODE`, and every `TENDLY_*` key. A secret may be referenced from that namespace, but an override cannot assign to it.
- Key names containing `TOKEN`, `SECRET`, `PASSWORD`, `PRIVATE_KEY`, `API_KEY` or `AUTHORIZATION` reject literal values; use `secret_ref` or `unset`. `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` and names ending in `_BASE_URL` also reject literal URL user information. Detection is case-insensitive. Do not put other credentials into benign-looking literal keys; validation is not a general secret detector.
- Invalid keys/value variants are rejected rather than interpreted as shell code. Profile defaults plus session overrides must satisfy limits after merging as well as individually.

Inheritance means the **backend process's environment**, subject to the existing native/isolated-account filtering and explicit removals. Tendly does not expose or persist that inherited environment, source shell rc files, import another terminal's aliases/functions/temporary exports, or reconstruct the unrecorded environment of an old history entry. Native source files are never rewritten by this API, and their precedence remains native-client behavior. Tendly's secret-reference namespace and access token are removed from child inheritance except for the explicitly resolved destination variable.

Optional interactive Terminal sessions still follow their shell's own startup-file behavior; that is separate from Tendly importing or reconstructing a native-history environment.

SQLite schema 6 adds `environment` JSON columns with `{}` defaults without rewriting old endpoint snapshots or moving credentials. `/api/health` advertises `session_environment`; clients must disable editing and avoid these payload fields/routes if the capability is absent. A newer frontend does not upgrade a running older backend. Enabling the API on that deployment requires an orderly backend upgrade/restart after preserving or ending active sessions; a frontend refresh must not terminate them.

## Files

`GET /api/host/directories?root=0&path=relative/folder` browses configured host locations for the workspace picker. It returns roots, current absolute/relative path, parent and directory entries only. Protected directories, traversal and symlinks are rejected. `TENDLY_BROWSE_ROOTS` limits the picker; manual registration remains a trusted-admin workflow, not an OS sandbox.

```text
GET /api/workspaces/:id/files?path=src
GET /api/workspaces/:id/file?path=src/main.rs
PUT /api/workspaces/:id/file?path=src/main.rs {content,expected_version}
GET /api/workspaces/:id/asset?path=assets/preview.png
WS  /api/workspaces/:id/files/ws
```

The files socket reports changes on disk so the tree and open files follow edits made outside the browser. It is read-only (plus Ping/Pong). The first message is `{"type":"ready"}` once the host watches the workspace, or `{"type":"unavailable","message":...}` when it cannot (for example an exhausted inotify limit), after which the socket closes and clients fall back to polling. Changes arrive in batches of about 200 ms as `{"type":"changed","paths":["src/app.ts"],"git":false}`. `paths` is `null` when anything may have changed, such as an overflowing batch or a lagged reader. `git` means repository metadata moved, so status may differ. Protected paths, `.git/objects`, lock files and dependency or build directories (`node_modules`, `target` and similar) are not reported. All sockets for one workspace share one watcher, which stops with the last of them.

Text replies: `{path,content,version}`. Existing-file writes require the last-read version and return 409 on conflict. Creates omit the version. Files are atomically replaced; application writes are serialized. Independent host processes can still modify files.

Paths are relative to the registered workspace; traversal and outward symlinks are rejected. Repository/private CLI metadata are protected. Media uses Range/HEAD support through the file service and a sandbox CSP. Browser codec support still applies.

## Git

```text
GET  /api/workspaces/:id/git/status
GET  /api/workspaces/:id/git/diff?path=src/main.rs&staged=false
POST /api/workspaces/:id/git/stage   {paths:["src/main.rs"]}
POST /api/workspaces/:id/git/unstage {paths:["src/main.rs"]}
POST /api/workspaces/:id/git/discard {paths:["src/main.rs"]}
POST /api/workspaces/:id/git/apply   {path,patch,cached?,reverse?}
POST /api/workspaces/:id/git/commit  {message:"...",amend?}
POST /api/workspaces/:id/git/ignore  {path}
POST /api/workspaces/:id/git/init

GET  /api/workspaces/:id/git/log?rev=&path=&all=&skip=&limit=
GET  /api/workspaces/:id/git/commit/detail?rev=&stash=
GET  /api/workspaces/:id/git/commit/diff?rev=&path=&original_path=&untracked=

GET  /api/workspaces/:id/git/branches
POST /api/workspaces/:id/git/switch  {branch,create?,start_point?,detach?,stop_sessions?}
POST /api/workspaces/:id/git/merge   {rev,no_ff?,squash?}
POST /api/workspaces/:id/git/rebase  {rev}
POST /api/workspaces/:id/git/cherry-pick {rev}
POST /api/workspaces/:id/git/revert  {rev}
POST /api/workspaces/:id/git/reset   {rev,mode:"soft"|"mixed"|"hard"}
POST /api/workspaces/:id/git/operation {action:"continue"|"abort"|"skip"}
POST /api/workspaces/:id/git/resolve {path,side:"ours"|"theirs"}

GET  /api/workspaces/:id/git/remotes
POST /api/workspaces/:id/git/remotes        {name,url}
POST /api/workspaces/:id/git/remotes/edit   {name,to?,url?}
POST /api/workspaces/:id/git/remotes/remove {name}
POST /api/workspaces/:id/git/fetch {remote?}
POST /api/workspaces/:id/git/pull  {mode?:"merge"|"rebase"|"ff-only"}
POST /api/workspaces/:id/git/push  {remote?,branch?,set_upstream?,force?,tag?,delete?}

GET  /api/workspaces/:id/git/stashes
POST /api/workspaces/:id/git/stashes       {message?,include_untracked?,keep_index?,paths?}
POST /api/workspaces/:id/git/stashes/apply {index,hash,pop?}
POST /api/workspaces/:id/git/stashes/drop  {index,hash}
GET  /api/workspaces/:id/git/tags
POST /api/workspaces/:id/git/tags        {name,rev?,message?}
POST /api/workspaces/:id/git/tags/delete {name}
```

Status includes `branch`, `files[{index,worktree,path,original_path?}]`, optional ahead/behind counts, `upstream` (`origin/main`) and `operation` when a merge, rebase, cherry-pick or revert stopped part-way. Branches add `local` and `remote` with each tip's hash, subject, date and, for local branches, upstream and ahead/behind. Diff supports staged, deleted and untracked files. No shell interpolation, external diff or textconv; output is bounded at 8 MiB per command and timeouts are enforced. Git hooks/signing remain native for explicit commits. No editor is ever opened: continuing an operation keeps the message Git prepared.

Every revision, branch, tag and remote name is checked by Git (`rev-parse --verify`, `check-ref-format`) before use, and nothing shaped like an option reaches a command line as one. Remote URLs using `ext::` or `fd::` are refused. `apply` takes one hunk of a diff the client showed, for exactly one named file. Merge, rebase, cherry-pick, revert, pull, stash apply and continue return `{conflicts,message}`: conflicts are an outcome, not an error, and the conflicted files appear in status to resolve. Stash apply and drop name both the index and the hash the client saw, and are refused if the stash list moved. Fetch, pull and push use the host's Git credentials (SSH agent or credential helper); with prompts disabled, a remote that needs a password fails with Git's message instead of waiting.

## Checkouts and isolated tasks

```text
POST /api/sessions/:id/checkout {branch?,create?,path?}          (move by hand)
POST /api/sessions/:id/checkout {isolated:true,branch?,base?}    (isolated task)
GET  /api/sessions/:id/task
GET  /api/sessions/:id/task/diff?path=
POST /api/sessions/:id/task/merge {squash?,message?}
POST /api/sessions/:id/task/close {discard?}
```

A session works in the workspace directory or in a Git worktree of the same repository beside it (`<repo>.worktrees/<branch>`), recorded as `checkout_path`/`checkout_branch`. Moving a session stops it if it is idle and starts a new native context there; a turn in progress refuses. A move by hand goes to the branch's existing checkout -- the workspace directory, or a worktree other sessions may share -- or makes a worktree.

`isolated:true` is the one way to a directory nobody else uses: a **new** worktree on a branch checked out nowhere else, named `branch` or, when absent, `tendly/<title>` (numbered on when taken; a piece of the session id when the title spells nothing). A new branch starts at `base`, a local branch, by default the one the session is on. A branch already checked out anywhere -- the workspace directory's included -- is refused with 409. The session records `checkout_base`; any later move by hand clears it.

`worktree_setup` on a workspace, `{copy:[relative paths], command?}` or `null`, is set only by the person. When Tendly makes a worktree for a session, each `copy` path present in the workspace directory is copied in (as a clone on APFS and reflink file systems), then `command` runs in the worktree through the user's shell (`TENDLY_SHELL`, else `$SHELL` as a login, interactive shell) with `TENDLY_WORKSPACE_ROOT`, `TENDLY_WORKTREE` and `TENDLY_BRANCH` set, stdin closed, a 20-minute limit and Tendly's own secrets removed from its environment. Progress is one conversation event, `{type:"setup",id,status:"running"|"completed"|"failed",copy,command,log?}`, sent again under the same id when it ends, with the last 16 KiB of output. Starting the session's client waits for the setup; a failed setup does not keep it from starting.

`task` reports `{branch,base,worktree,ahead,behind,uncommitted,files[{path,added?,removed?,untracked?}],files_truncated,base_checkout,base_is_workspace,upstream,compare_url,preparing}`; `files` and `task/diff` cover everything since the merge base, committed or not (untracked files are listed, not diffed). `merge` requires an idle session and a clean worktree. It checks with `git merge-tree` first and refuses conflicts with 409 before anything changes. Where the base is checked out it merges there (`--squash` plus a commit with `message`, default the session title, when `squash`; a squash refuses if changes are staged there); where it is checked out nowhere it writes the merge from trees alone and moves the base ref only if it has not moved. Then, like `close`, it removes the worktree and branch, moves the session (and any other stopped session bound there) back to the workspace directory and archives it. `close` without `discard` refuses while commits or uncommitted changes are not on the base. Discarding a temporary isolated session removes its worktree and branch only when they hold nothing.

## Profiles

```text
GET    /api/endpoint-profiles
POST   /api/endpoint-profiles {name,provider,endpoint_url?,model?,effort?,permission_mode?,secret_ref?,environment?}
GET    /api/endpoint-profiles/:id
PATCH  /api/endpoint-profiles/:id
DELETE /api/endpoint-profiles/:id
POST   /api/endpoint-profiles/discover-models <profile fields>
GET    /api/host/native-configurations
POST   /api/endpoint-profiles/import-native {source_id,name?,confirmed_shared_config:true}
```

PATCH supports null to clear optional scalar fields; `environment` must be an object and `{}` clears its defaults. Provider type cannot change in place. Deleting an ordinary template preserves existing session snapshots. A profile owned by the official-account manager cannot be independently deleted into a dangling account association.

`host/native-configurations` returns server-configured sources `{id,provider,label,path,available,config_env}`. An available source has a canonical `path` and can be referenced; it does not mean authentication is valid. The import operation returns 200 and an `EndpointProfile` with `native_config: {source_id,config_dir,config_env}`. The directory and its original launch context are pinned by the server. `config_env: null` means the native directory variable stays unset; a string preserves the original absolute directory environment value, including spelling. This is path metadata, not a credential. Relative directory variables are rejected. The import request accepts a source ID, optional display name and confirmation, not arbitrary paths, native directory-environment context or credential/configuration contents. Import is idempotent for provider/source/directory/environment context and does not start a client or read credentials.

For these native-reference profiles, PATCH allows only `name` and `environment` (either or both). Direct endpoint, secret, model, effort, alias, proxy and permission fields remain rejected; `permission_mode` remains `native`. The optional environment template is a process-only overlay, not an edit to source settings. New sessions still use `endpoint_profile_id`, but model and effort overrides are rejected. The binding and environment defaults are snapshotted in the session; source settings themselves are **live** and are not copied. Provider/path/native-directory-environment-context changes fail closed at creation and launch. The original CLI keeps the referenced `CODEX_HOME`/`CLAUDE_CONFIG_DIR` value or its original absence. A fresh context does not receive login/model/permission-policy overrides; structured mode adds transport/control flags, and an established session may resume by its recorded native ID. Removing an ordinary host reference preserves existing snapshots and source files; managed-account profiles cannot be independently unlinked. SQLite schema 4 added native references; schema 6 adds environment maps, and schema 9 adds reasoning-effort defaults while preserving older profiles and snapshots.

Profiles additionally accept `proxy_url?: string|null` and `model_aliases?: {alias: actual_model_id}`. Model alias resolution is one step and never translates provider protocols. Proxies support HTTP/HTTPS; inline credentials and SOCKS are rejected. SQLite schema version 2 migrates profiles without changing existing session snapshots.

`discover-models` accepts unsaved profile settings and returns `{models:[{id,name,efforts?}],source_url,has_more}`. `efforts` is present only when the source advertises supported reasoning levels. Native Codex without a custom endpoint/key reference uses its isolated app-server catalogue (`codex://model/list`). Custom endpoints call their models API; credentials are resolved on the server, redirects are disabled, timeout/body limits apply, and no inference is requested. Claude Code's known native effort vocabulary is available in its picker; Codex choices remain model-catalog driven.

`probe-models` is the only route that runs inference, and only because a person pressed a button: `POST {profile: <same draft fields as discover-models>, models: [id, …]}` returns `[{model,status,latency_ms?,detail?}]` in the order asked. Each model gets **one minimal request** in the shape its client would send — `max_tokens`/`max_output_tokens` of 1, `store:false` where the API takes it, path per client and per Pi `api` (`/v1/messages`, `/v1/responses`, `/v1/chat/completions`, `:generateContent`) — and the reply is discarded. The endpoint bills each one as a very short call, so the interface states the cost before running and a request is capped at 24 models, 4 in flight, 20 s each. The Web UI asks in batches of 8 so rows fill in and Stop takes effect at once, and caps one press at 24 models: anything beyond that is reported as remaining rather than sent. Each row carries `api`, the wire protocol the check spoke (`anthropic-messages`, `openai-responses`, `openai-completions`, `google-generative-ai`): the same model can answer on one API and fail on another, so an answer without its protocol would be ambiguous. Claude Code always speaks Anthropic Messages and Codex always speaks Responses; Pi speaks whichever `api` the profile names, or the one its URL suggests. `status` is `ok`, `unauthorized` (401/403 — the key, not the model), `not_found` (404), `rate_limited` (429), `timeout`, or `failed`; a 2xx whose body carries a non-empty `error` counts as `failed`. `detail` is the server's own fixed wording, never the endpoint's text, which may echo the credential. Redirects stay blocked, response bodies are read to 64 KiB, and a profile with a `native_config` is refused: an official account's models run through its own client. Pruning is a client-side edit of `models`, saved by the ordinary PATCH.

`probe-plan` answers what a check *would* send without sending it: `POST` the same draft profile fields as `discover-models`, and it returns `{client, api, path, guessed}`. It contacts no endpoint, so the form may call it on every keystroke; `guessed` is true when the profile names no API and the server inferred one from the URL, which on Pi decides the request and is worth knowing before a batch of failures rather than after.

`effort` is one of `low`, `medium`, `high`, `xhigh` or `max`. A profile stores a default; a new session may override it, and the effective value is captured in that session's endpoint snapshot. Native-reference profiles cannot receive model or effort overrides. The Claude adapter forwards `--effort`; the Codex adapter writes `model_reasoning_effort` to its generated per-session configuration. Empty/omitted means the provider default.

Secret references must use `env:TENDLY_SECRET_NAME`. Resolved secret values are not stored in profile metadata or returned to the Web UI; explicit environment literals, by contrast, are plaintext metadata and must not contain secrets. Generated native configuration uses per-session directories; it does not silently replace global client config.

Permission intents are not equivalent security guarantees: native uses CLI defaults; interactive uses native conservative prompts; trusted uses Claude acceptEdits and Codex on-request + workspace-write; Claude-only `plan` forwards Claude Code's native plan mode. Codex keeps its native approval model and rejects the `plan` intent rather than pretending it has an equivalent. Blocked refuses launch. Tendly never adds a bypass flag.

Codex configuration follows [official configuration documentation](https://developers.openai.com/codex/config-advanced/) and [credential storage](https://developers.openai.com/codex/auth/); CLI version differences still require adapter testing.

## Layouts

```text
GET /api/workspaces/:id/layout
PUT /api/workspaces/:id/layout <LayoutDocument>
```

Version 1: recursive `split` (id, direction, ratio, first, second), `stack` (id, panes, activePaneId), and `pane` (id, kind, title?, metadata?). IDs must be unique; type/ratio/depth/body-size guards apply. Responsive collapse is a view projection, never destructive persistence of the canonical layout.

### Shared cross-workspace canvas

```text
GET /api/canvas/layout
    -> {layout: LayoutDocument|null, revision: number}
PUT /api/canvas/layout {layout: LayoutDocument, expected_revision: number}
    -> {revision: number}
```

SQLite schema 5 adds an independent singleton with initial `layout:null, revision:0`. It does not merge or rewrite legacy workspace layouts, sessions or profiles. Saves atomically compare the expected revision, increment it on success, and return 409 without modification for a stale revision. GET never silently replaces invalid stored data with a default layout.

The shared canvas uses the same canonical split/stack format. `metadata.workspace_id` must identify a registered workspace. Bound files (`metadata.path`), Git and preview panes require that scope. Paths must be relative without traversal or NUL. Session panes additionally carry `metadata.session_id`; the ID must exist and belong to the specified workspace, and kind/provider must agree with the registered session. Empty agent/editor/terminal placeholders may remain unbound. Nonempty legacy `collapsed` snapshots must be restored to the canonical tree before saving.

Current `/api/health` capabilities include `shared_canvas`, `native_configurations`, `native_history`, `host_directories`, `endpoint_models`, `session_environment`, `structured_chat`, `official_accounts` and `session_configuration`. A running older server does not acquire these routes from a frontend refresh. Gate each feature independently: missing `shared_canvas` selects browser-local layout storage; missing `structured_chat` retains PTY; missing account/configuration/environment capabilities disables those operations. Legacy `/workspaces/:id/layout` remains available for migration and older clients.

## Official accounts

```text
GET  /api/accounts
POST /api/accounts {name,provider}
GET  /api/accounts/:id
POST /api/accounts/:id/refresh {refresh_token:false}
POST /api/accounts/:id/login {mode:"device"|"browser"}
POST /api/accounts/:id/cancel-login
POST /api/accounts/:id/logout {confirmed:true}
POST /api/accounts/:id/reset-quota {confirmed:true,idempotency_key,credit_id?}
POST /api/accounts/:id/usage
```

These routes require `official_accounts` support and use `Cache-Control: no-store`. Account creation makes a private account record/configuration home and its associated native-reference profile; it does not log in or start a conversation. There is no raw-credential upload/download API and no account DELETE endpoint.

Responses are `AccountView` objects (a list for GET `/accounts`): `id`, `name`, `provider`, `profile_id`, `storage_path`, `status`, nullable `email`, `plan`, `checked_at`, `error`, `login`, `limits`, `guidance`, `reset_outcome`, optional `usage`, plus `capabilities:{login,refresh_token,quota,reset_quota,logout,usage}`. A pending official login may expose `{url,id,user_code}`. Limits may contain `primary`/`secondary` windows (`used_percent`, nullable `window_minutes`/`resets_at`) and `reset_credits`. Missing/unknown data is not zero or a promise of available quota.

GET reads metadata/cached state. Explicit refresh invokes the official native status APIs; `refresh_token:true` asks Codex to refresh its own token. Codex login/cancel/logout, status and quota use official app-server operations. Authentication changes and reset-credit consumption require no active session using that account; they return a conflict rather than automatically stopping sessions. Pending login/maintenance also blocks conflicting new account use.

Quota reset is only the official **earned reset-credit consumption** operation when supported and eligible. It is not a way to reset arbitrary account limits. The server durably records the idempotency authorization before the bridge can consume a credit. Keep the same key for an uncertain retry; do not blindly issue a new key. Only `reset`, `alreadyRedeemed`, `nothingToReset` and `noCredit` are accepted terminal outcomes. Unsupported or failed requests remain explicit errors, not fabricated success.

Claude keeps login and token refresh under the official client. `POST /accounts/:id/usage` is different: it is an explicit, user-triggered read that uses the official native OAuth credential in memory to call `https://api.anthropic.com/api/oauth/usage` with the same `oauth-2025-04-20` beta header used by CPA/CLIProxyAPI. The token is never returned to the browser or logs, there is no background polling, and quota reset is still not offered for Claude. See [official account storage and maintenance](official-accounts.md) for the security and verification boundaries.

## Attachments

```text
POST /api/workspaces/:id/attachments?name=<file name>
```

Raw request body (`application/octet-stream`), so a phone upload costs no base64 inflation. Max 10 MiB per file; the route carries its own body limit rather than the shared 3 MiB JSON cap. Returns `{path, name, bytes}`.

The file is written inside the workspace under `.tendly-files/<date>/`, and the workspace-relative `path` is what a session is given. Both native clients can already read workspace files with their own tools, so this works for every provider and every file type without depending on one client's inline-content protocol.

Only the base name of `name` is used, so a path from a file picker cannot steer the write; the stored file name is slugified (no spaces) because an agent receives it as text, while the returned `name` keeps the user's own spelling. Each upload gets a unique file name, so two photos with the same camera name both survive. A scoped `.tendly-files/.gitignore` containing `*` is created on first use, so uploads do not appear as pending Git changes; no `.gitignore` belonging to the user is edited.

## Agent clients

```text
GET  /api/clients
POST /api/clients/:provider/install {confirmed:true}
```

Requires `agent_clients`. `GET` is read-only discovery: for Claude Code and Codex it reports the `program` a session would launch, where it came from (`source`: `override` | `path` | `managed` | `missing`), the `version` the client reports, its `npm_package`, whether npm exists on the server (`install_available`), and whether a Tendly copy exists but is outranked (`shadowed`). Looking installs nothing.

Resolution order is `TENDLY_*_BIN` override, then the server's `PATH`, then Tendly's own copy. A client already on the host always wins, so installing here can never silently repoint existing sessions at a different binary.

`install` runs `npm install --prefix <state_dir>/clients <package>`. It is not a global install and needs no elevated permissions. It downloads and runs package code, so `confirmed:true` is required and one install runs at a time. When the result is `shadowed`, the install succeeded but sessions keep using the host copy — the response says so rather than implying a change.

### Claude usage check

`POST /api/accounts/:id/usage` is a Claude-only, user-triggered read; Codex returns a bad-request and should use `refresh` instead. It first tries the official OAuth usage endpoint, then falls back to the optional status-line capture. It reports `capabilities.usage` and a `usage` object with `availability`, `source`, and optional capture metadata:

| `availability` | Meaning |
| --- | --- |
| `capture_disabled` | Capture is not enabled on this server, so nothing is ever recorded. |
| `capture_missing` | Capture is enabled but no session has written a payload yet. |
| `credential_unavailable` | The native Claude OAuth credential was not available to the server process. |
| `query_failed` | A credential was found but the official usage endpoint could not be reached; a local capture may still be shown. |
| `not_reported` | A payload exists without usage fields — Claude Code adds `rate_limits` only after a session's first API response, and only for subscription accounts. |
| `stale` | The stored payload predates its own reset time. |
| `available` | Usage windows are present and current. |

The preferred source is the official Claude OAuth usage response (`five_hour`, `seven_day`, and optional model windows), using `utilization` and `resets_at`. The same response shape is used by [CPA Usage Keeper](https://github.com/Willxup/cpa-usage-keeper) through [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI); Tendly only adopts its read-only request and normalization, not its proxy, account pool, or routing logic. OAuth provenance is carried by `usage.source: "claude_oauth_usage"`; the two named windows map to `primary`/`secondary`. The status-line fallback uses `mapping_basis: "status_line_window"`. Nothing polls, and an absent or malformed response is never rendered as zero usage.

Recording that payload requires adding a status line command to the Claude sessions Tendly starts, which overrides the account's own configuration for those sessions, so it is opt-in: set `TENDLY_CLAUDE_USAGE_CAPTURE=1` on the server. The overlay is passed per launch with `--settings` — no settings file belonging to the user is written — and a status line the account already configured is run with the same payload and its output passed through unchanged. Sessions started outside Tendly are never modified and record nothing.

## Paired machines

```text
GET    /api/peers                      this machine's name; each paired machine and whether it answers now
POST   /api/peers                      { invite, grant } redeem another machine's invite link
                                       { url, grant, asked } or pair with a machine of the same Tailscale account
POST   /api/peers/invite               { grant } a one-time invite link to this machine
PUT    /api/peers/{id}                 { grant } what that machine may do here
DELETE /api/peers/{id}                 unpair, on both sides
GET    /api/peers/{id}/sessions        its workspaces and sessions
POST   /api/peers/{id}/open            { session_id? } a link to its window, signed in once
GET    /api/peers/discover             machines on the tailnet, whether they run Tendly and whether they are this account's (mine)
```

Requires `peers`. `grant` is `view`, `spawn` or `control`; `granted` in a listing is what the other machine allows this one. Machines call each other under `/api/peer/` (`pair`, `hello`, `granted`, `unpair`, `sessions`, `ticket`, `call`) with `X-Tendly-Client: peer` and the key the called machine issued; nothing else accepts that key. `POST /api/auth` takes `{ ticket }` as well as `{ token }`. See [Security → Paired machines](security.md#paired-machines).

## Updates

```text
GET  /api/update[?refresh=true]
POST /api/update
POST /api/update/restart
```

Requires `self_update`. `GET` reports `current`, the `latest` release npm sees (cached for ten minutes; `refresh=true` asks again), whether it is `available`, a version already `installed` and waiting for a restart, the install `method` (`npm` or `manual`), whether the server's user can replace it (`writable`), the `command` to run by hand when it cannot, how a restart would happen (`restart`: `daemon` | `supervisor` | `manual`), how many sessions a restart would stop (`running_sessions`), and `check_error` when the lookup failed.

`POST /api/update` installs the latest release over this npm install and verifies the binary now reports it; it answers with the new status. It is refused with 409 for a non-npm or unwritable install, when already current, or when that version is already installed; npm failures answer 502 with npm's last lines. Nothing restarts.

`POST /api/update/restart` answers first and then restarts after a moment; 409 when `restart` is `manual`. The page then polls `/api/health` until it reports the new version. See [Configuration → Updating](configuration.md#updating).
