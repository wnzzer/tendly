# Tendly 架构

```text
Browser (Vue 3)
   │ HTTPS + WebSocket
   ▼
Rust API/Session Gateway (crates/server)
   ├── Structured conversations ── Claude Code stream-json / Codex app-server JSON-RPC / Pi RPC
   ├── Terminal sessions ────────── PTY (crates/agent-runtime) ── official CLIs or a shell
   ├── Workspace IO ─────────────── file tree, preview/edit, attachments
   ├── Git ──────────────────────── status/diff/stage/commit/branch/worktree
   └── Persistence ──────────────── SQLite (crates/persistence)
```

## 关键边界

- API 层只负责协议、鉴权和请求编排，不直接执行任意命令。
- 结构化会话通过 `chat_adapter::ClientAdapter` 接入：命令进（`init`、`message`、`steer`、`approval`、`interrupt`、`model`、`permission`、`shutdown`），事件出（`ready`、`message`、`tool`、`approval`、`turn`、`usage`、`exit` 等）。结构化会话统一在 Rust 进程内直连客户端（`codex_chat`、`claude_chat`、`pi_chat`，共用 `native_chat`），不再提供 Node 聊天桥或回退开关。
- 每个客户端的安装、配置目录、历史来源与环境变量清理由 `adapters::ClientAdapter`（`crates/server/src/adapters/mod.rs`）描述，Claude Code、Codex、Pi 各一个实现。
- 终端会话和旧 PTY 会话走 `crates/agent-runtime`：独立进程组、有界回放缓冲，重连时按帧序号续传（见 `pty_routes.rs` 的 `resume_point`）。
- 默认直接使用宿主机真实开发环境，不提供容器或 microVM 运行时。
- 文件与 Git 操作的路径都 canonicalize 并限制在 workspace 根目录（或会话自己的 worktree）内，见 `workspace_io`。
- 前端只依赖 `packages/protocol` 定义的 DTO 和布局模型，不耦合 Rust 内部模型。
- SQLite 只存元数据、对话事件和可恢复状态；workspace 文件、附件和大日志放在宿主机目录，密钥放在 secret store（数据库只存 `secret_ref`）。

Tendly 是编排壳，不复制官方 CLI 的 Agent loop、权限系统或工具实现。保持单个 Rust server + 宿主机 workspace。

## SQLite storage policy

使用单个 SQLite 数据库文件，开启 WAL、foreign keys 和 busy timeout。写入通过 `crates/persistence` 的 `Store` 集中处理。迁移 SQL 在仓库根目录 `migrations/`。

主要表：

```text
workspaces
endpoint_profiles
sessions
session_agent_tools / agent_children     会话的工具开关与子会话关系
layouts / shared_canvas                  每个 workspace 的画布与共享画布
conversation_meta / conversation_events / conversation_submissions
preferences
```

资源采样（`host_samples`、`session_samples`）在单独的指标库里，见 `crates/persistence/src/metrics.rs`。

不把 API key 写进数据库明文。文件内容和大附件不进入 SQLite，只保存路径等元数据。对话事件带可恢复的序号。

## Workspace layout model

Workspace Layout Engine 是 Tendly 的核心产品层。Agent provider、Terminal、Git 和 Preview 都只是被布局引擎承载的 pane，不应反过来决定工作区结构。

可编排工作区使用持久化布局树：

```text
Split(horizontal)
├── Pane(AgentChat, session_id=...)
└── Split(vertical)
    ├── Pane(Editor, file=...)
    └── Pane(Terminal, shell=...)
```

每个 `Pane` 都可以再次 split、替换类型、最大化或关闭。布局引擎负责最小尺寸约束和比例调整：拖拽分隔线时支持像素调整，并吸附到 `1:1`、`1:2`、`2:1` 等规则比例。每个 split 节点保存方向、比例、最小尺寸和折叠状态。

当某个 pane 被挤压到最小可读尺寸以下时，不继续缩小内容，而是将它转换成父级的 `Stack`/tab；恢复空间或点击展开后，恢复原来的比例和 pane 状态。终端、Agent Chat、Editor、Preview、Git Diff 都只是 pane 类型，不应在核心模型中拥有特殊的固定位置。

```text
Split(horizontal, ratio=1:2)
├── Pane(AgentChat, min=280x180)
└── Stack(min=280x180)
    ├── Pane(Editor)
    └── Pane(Terminal, collapsed=true)
```

布局树需要版本化持久化，并支持迁移；前端操作统一抽象为 `split`、`resize`、`move`、`replace`、`collapse`、`restore`、`maximize` 和 `close`。新增功能优先注册新的 pane kind，而不是修改核心布局模型。

```text
PaneKind (packages/protocol/src/layout.ts)
├── agent_chat
├── editor
├── terminal
├── git_diff
├── file_preview
└── stack
```

Git Changes 是工作区的一等入口，不应只作为某个编辑器 tab 的附属功能。Git service 需要返回文件级和 hunk 级 diff，并支持 stage、unstage、discard、commit 前检查；UI 可以把 Changes 固定为导航项，也可以把它编排成 pane。
