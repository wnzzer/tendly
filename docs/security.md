# Security and boundaries

Remote access and what this server can and cannot reach. Back to the [README](../README.md).

## Private remote access

```sh
tendly --lan
```

Binds every interface, generates an access token if `TENDLY_TOKEN` does not
provide one, and prints it; `tendly status` prints it again. The token is
kept in the state directory, readable only by its owner — a background gateway
cannot show it to you any other way, and deleting the file issues a new one.

Reaching the machine at its own address needs nothing further: a `Host` that is
an IP literal on the bound port is answered for, because DNS rebinding requires
a *name* and cannot produce one. `TENDLY_ALLOWED_ORIGINS` remains for the
hostnames a reverse proxy serves under. Cross-site requests stay blocked by the
Origin and `sec-fetch-site` checks either way.

Traffic is plain HTTP, so on a network you do not control, prefer SSH port
forwarding — `ssh -L 28789:127.0.0.1:28789 user@server` — or put an HTTPS
reverse proxy in front that preserves Host and WebSocket upgrades. Do not expose
this trusted-host workspace to untrusted users: whoever holds the token can run
agents, read and write files, and open terminals on that machine.

## Boundaries

This is a **single trusted user's host workspace**, not a multi-tenant sandbox. Native tool permissions remain native. A working-directory check or config directory is not an OS isolation boundary.

### What this server can reach

- **Network**: loopback only by default. A non-loopback bind refuses to start without an access token (`TENDLY_TOKEN`, or one it generates), and Origin/Host must be allow-listed.
- **Directory picker**: limited to `TENDLY_BROWSE_ROOTS`, defaulting to the working directory and the server user's home.
- **New workspaces**: limited to `TENDLY_WORKSPACE_ROOTS`, defaulting to the browsing roots. Workspaces created before this rule keep working; it governs new ones.
- **File read/write**: confined to a workspace root. Paths are canonicalised and anything resolving outside is refused, as are `.git`, `.tendly` and client configuration files. Deleting never follows a symlink out of the tree.
- **Agent processes are not confined by any of the above.** `claude` and `codex` run with your full user permissions. Tendly sets their working directory and forwards their permission prompts; it never answers one for you, and it never reads their credentials. What an Agent can reach is decided by that client's own permission settings, not by this server.

Anyone who can reach the API has the authority of the user running the server. Treat the token as that user's credentials.

- Browser disconnection preserves processes; server shutdown stops managed processes. Restart marks old running records stopped. It does not revive a lost PTY.
- State and database ownership locks prevent two new-version servers from managing the same sessions. Port binding and ownership checks happen before migration/reconciliation. `init` also respects these locks and never resets running-session status. Keep state on a local filesystem with working OS file locks; do not remove lock files while a process is alive.
- Terminal replay is the most recent 1 MiB in memory, not a durable transcript. After a server restart use the native client's history/resume workflow.
- Model-generation and paid API requests are not part of the automated tests; sign in or supply your own provider reference.
- No custom Agent reasoning loop, token-exchange server, provider reverse proxy, Keychain editor, automatic account import, strong cross-process credential isolation, hunk staging, Git network/PR workflow, LSP or computer-desktop control in this MVP.
- Media playback depends on browser codecs. Text editing limit: 2 MiB. Large Git output is rejected; no silent truncation.
- Common proxy support is HTTP/HTTPS without inline passwords. Claude Code does not support SOCKS. Empty proxy follows host settings; explicit profiles override inherited proxy variables for that process only.
- Model listing never performs inference. HTTP requests have time/body limits and do not follow redirects with credentials; non-compatible endpoints keep manual model entry as fallback. A separate, explicitly pressed model check (`probe-models`) does send one minimal request per model, at the endpoint's expense; it is capped, it is never scheduled, and it never runs from discovery or from opening a profile.
- Resource guardrails bound layout depth/nodes, event windows and runtime replay. Managed process groups receive bounded cleanup; deliberately detached daemons remain outside that group boundary.
- macOS has been run locally. Linux has a CI target, not a claim of a completed deployment here.
