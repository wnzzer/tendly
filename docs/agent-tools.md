# Agent tools

Claude Code and Codex sessions can look at Tendly and, with your say, configure it: set up an endpoint, change new-session defaults, add a workspace, start another session in parallel, or point you at a file. Back to the [README](../README.md).

## How agents reach it

Every Claude Code and Codex session Tendly launches carries an `tendly` MCP server, added through that launch's own flags (`--mcp-config` for Claude, `-c mcp_servers.tendly.*` for Codex). Your `~/.claude`, `~/.codex` and any host configuration a profile points at are never written. Terminals get only the environment variables below.

The MCP server is `tendly mcp`, a stdio relay: it forwards tool calls to the running Tendly server, which decides everything. To use the same tools from a client you run yourself, outside Tendly:

```sh
claude mcp add tendly -- tendly mcp
codex mcp add tendly -- tendly mcp
```

Such a caller is *external*: it has no session of its own, so the session tools below are not available to it.

Each launch also gets `TENDLY_URL`, `TENDLY_SESSION_ID` and its own `TENDLY_AGENT_TOKEN`. The token is good only while that session's process runs, and only for `/api/agent/`.

Turn it off for every session under **Settings → Preferences → Tendly tools for agents**, or for one session from its **⋯** menu (**Tendly tools: Default / On / Off**; one session's choice wins over the preference). Either applies from the session's next start, when the tools are injected. A session cannot change its own setting: the switch is not reachable with its agent token.

## Tools

| Tool | What it does | Asks you first |
| --- | --- | --- |
| `tendly_status` | Which session, workspace, directory, branch and endpoint is calling, and which other sessions share that directory | — |
| `tendly_list` | Workspaces, sessions (metadata only), endpoint profiles (whether a key is set, never the key), installed clients | — |
| `tendly_endpoint` | Create, change, delete, test, or import a host configuration as an endpoint profile | Yes, except `test` |
| `tendly_preferences` | New-session defaults per client: endpoint, effort, permission; the interface and terminal theme | Only a switch to running tools without asking |
| `tendly_workspace` | Add a directory as a workspace | — |
| `tendly_spawn` | Start a Claude Code or Codex session with a first message, optionally as an isolated task | The first time per session |
| `tendly_session` | Follow sessions it started (`list`, `result`, `wait`, `message`); rename or keep its own | — |
| `tendly_show` | Open a file at a line, the Git changes, or a session in your window | — |
| `tendly_usage` | The quota its official account last reported | — |
| `tendly_canvas` | List the tabs on the canvas; close some or all | Only when it would discard a temporary session still working |
| `tendly_machines` | The other machines this one is [paired with](security.md#paired-machines), whether each answers now, and what each lets it do | — |

`tendly_list`, `tendly_spawn` and `tendly_session` take an optional `machine`: the call then runs on that paired machine. A start there takes `workspace` (by name or id; the default is the one named like the caller's own), and asks you here the first time per session; that machine allowed it when it made the invite. The session runs there, in that machine's copy of the repository, and is kept in its list with the caller's machine in its title. A session can follow only the ones it started there.

## What you see

A change that asks you first appears in the Tendly window as a card: who is asking, what would change, and **Approve** / **Decline**. The call waits up to five minutes for your answer; with no Tendly window open it fails at once instead.

When an endpoint needs an API key, the card has a key field. The key goes from that field to Tendly's key store (see [Configuration](configuration.md#state-and-credentials)) and never through the agent or its conversation. Agents are told never to ask for a key in chat.

Everything an agent changes without asking is shown as a notice, with **Undo** for endpoint profiles and defaults, and **Open** for a session it started.

## Isolated tasks

`tendly_spawn` with `isolated: true` (or a `branch` name) gives the new session a worktree and branch of its own, started from the branch the caller is on: the two never edit the same files. The card asking you says so. The worktree is prepared as the workspace's setup says before the task is sent (the call returns at once and says it is preparing). The new branch must be checked out nowhere else; agents are told not to make worktrees or switch branches themselves for this. Merging the task back, or throwing it away, is yours: the session's branch chip opens **Review and finish**.

`tendly_status` reports `checkout`: the directory, branch, `isolated_from` (the base, for an isolated task) and `shared_with`, the other sessions working in the same directory, with a note on what not to do there.

## Limits

- A session can read and message only the sessions it started, and a session started this way cannot start more. At most four it started may run at once. Started sessions are temporary unless the agent asks to keep them.
- Agents cannot answer or even see the cards: those live under `/api/agent-activity/`, which an agent token does not open.
- A call from another machine never reaches a third, and a session another machine started cannot start sessions, there or anywhere.
- Agents cannot read other sessions' conversations, approve their own tool calls, change their own permission mode, or sign accounts in or out.

The token identifies the caller; it is not the security boundary. On a deployment without an access token (the local default), any process running as you can already call Tendly's whole API. What protects the changes that matter is the confirmation in your window.
