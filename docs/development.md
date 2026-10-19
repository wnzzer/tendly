# Development

Building from source, the development loop and verification. Back to the [README](../README.md).

## Build from source (macOS / Linux / Windows)

Requires Rust stable (1.89+), **Node.js 24+** and **pnpm 10.30.3**. `.nvmrc` / `.node-version` and CI use Node 24; an installed newer Node is also supported by the project engine range. Install Claude Code and/or Codex separately.

```sh
pnpm install --frozen-lockfile
pnpm run build:web
cargo run -p tendly-server
```

Open **http://127.0.0.1:28789/**. The Rust server serves the built Web client and API on the same origin.

For development, run `pnpm run dev` to start Cargo + Vite, then open **http://127.0.0.1:5173/**. If Rust is already running, use only `pnpm run dev:web`. Vite proxies `/api` and WebSockets to Rust. pnpm manages the workspace and dependencies; Vite, Vue SFC typechecking, tests and the native-history bridge run under Node. Rust builds still use Cargo.

The previous Bun lockfile and version pin are retained under `docs/toolchain-history/` as historical evidence, not an active toolchain. Do not use them for current installs. In the observed macOS debugging case, the backend WebSocket opened directly but timed out through Bun-run Vite; running Vite under Node restored the proxy handshake. The existing development frontend on 5173 was switched to Node without restarting the running backend (then on its old port, 8787) or its Agent processes.

Choose an existing host directory in the workspace picker, then create/open a session. New Web-created Agent sessions use structured conversation cards on a capable backend; existing PTY sessions keep their original interface. Tool approvals remain native decisions, rendered by the Web client. **Interrupt** cancels current work, while **End session** remains a separate, confirmed secondary action. Closing a pane never ends its process.

## Repository layout

| Path                     | Contents                                                     |
| ------------------------ | ------------------------------------------------------------ |
| `crates/server`          | HTTP/WebSocket API, daemon, security, accounts, workspace IO |
| `crates/agent-runtime`   | Process and PTY supervision (built as `tendly-runtime`)      |
| `crates/persistence`     | SQLite store and migrations (`migrations/`)                  |
| `crates/domain`          | Shared domain model                                          |
| `packages/native-bridge` | Node bridges: structured chat (the default layer), account sign-in, native history, file search |
| `packages/protocol`      | DTOs and layout model shared with the client                 |
| `apps/web`               | Vue 3 + TypeScript client and layout engine                  |
| `npm/`                   | Packaging for `@wnzzer/tendly` and platform packages      |

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm run check
pnpm run build:web
```

See [API contract](api.md) and [acceptance record](mvp-status.md). Earlier design documents describe the broader roadmap, not finished features.

### Read-only UI preview and deployment caution

With the development frontend running, `http://127.0.0.1:5173/ui-preview.html` renders the actual UI components against clearly labeled local snapshots. It does not call the user's Agent backend. Its 390px container previews narrow-screen layout; it is **not** a real-phone or touch-device acceptance test.

### Screenshots of one component

`http://127.0.0.1:5173/ui-shot.html` renders the model-check panel alone, against a fixture catalogue and a local stand-in for the check, so no request leaves the page. `?mode=done|chosen` picks how much of the panel is filled in, `?api=…` picks the protocol it claims to have used, `?guessed=1` marks that protocol as inferred, and `?all404=1` makes every model fail the way a wrong protocol does. Harness layout lives in the page's own `<style>`, because `apps/web/src/features/design-tokens.test.mjs` walks every file under `src/` and counts literal font sizes.

Neither harness page is reachable from the built app: Vite only builds `index.html`, so `dist/` carries no copy of them. Verify layout claims by measuring the DOM (`getBoundingClientRect`, `scrollWidth > clientWidth`, a bounding-box check for escapes) rather than by describing a PNG.

The existing 5173/8787 development installation has four user-owned active sessions; do not restart its backend or end those sessions merely to expose new capabilities. Use an explicitly separate state/database/port for feature previews (8788 is the designated preview port), and record what was actually verified there. Do not infer live OAuth success, a paid model turn, Linux deployment or mobile-device acceptance from compiled code, fixtures or the read-only preview.
