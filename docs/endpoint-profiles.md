# Endpoint Profiles 与会话隔离

## 目标

Tendly 的端点设置不应该是全局单例。每个 Agent session 在启动时绑定一个 `EndpointProfile`，并保存一份不可变的 effective config snapshot。

```text
EndpointProfile
  ├── provider: claude_code | codex
  ├── endpoint_url
  ├── protocol: anthropic_messages | openai_responses | openai_chat
  ├── model
  ├── auth_ref          # 只保存 secret store 引用，不保存明文
  ├── extra_headers     # 加密保存或引用 secret
  └── capability_flags
  └── permission_mode: native | interactive | trusted | plan (Claude Code) | blocked

AgentSession
  ├── provider
  ├── endpoint_profile_id
  ├── config_snapshot
  ├── workspace_id
  └── runtime_handle
```

## 为什么不能只做一个统一 URL

Claude Code 和 Codex 都是 CLI Agent，但它们的配置模型、认证方式、协议和模型能力不完全相同：

- Claude Code profile 通常需要 Anthropic 侧的 base URL、认证环境变量或第三方云凭证。
- Codex profile 需要 model provider、wire API、base URL、模型和认证策略；其 CLI 也支持通过配置覆盖启动参数。
- 某个网关可能同时暴露 Anthropic Messages 和 OpenAI Responses 兼容入口，但这不是默认保证，不能只靠 URL 猜测。

因此 UI 可以统一，但 provider adapter 必须分别生成启动配置：

```text
ClaudeCodeAdapter  -> session env / temporary settings
CodexAdapter       -> temporary CODEX_HOME + config.toml / -c overrides
```

## 会话隔离策略

1. 用户在启动会话时选择 `Provider + Endpoint Profile + Model`。
2. Tendly 将 profile 解析成 effective snapshot，写入 session metadata。
3. 启动子进程时只注入当前 session 的环境和临时配置目录。
4. 不修改宿主机全局 `~/.claude` 或 `~/.codex/config.toml`。
5. profile 修改只影响新会话；已有会话需要明确“重启并应用”。

## Provider 原生权限

Tendly 不重新实现 Claude Code 或 Codex 的工具授权、确认、sandbox 和 trust 语义。启动 session 时由对应 adapter 透传 provider 支持的参数和配置；运行中将 CLI 的确认提示作为事件或 PTY 输出展示，并把用户的确认/拒绝输入回传给原生 CLI。

诸如跳过权限检查的危险启动选项不应被 Tendly 默默启用；可以作为明确标记的透传选项显示，但不能被产品默认策略替换。Tendly 自己只负责非语义性的宿主机边界：工作目录、进程组、资源限制、凭证注入和生命周期。

Codex 建议每个 session 使用独立的 `CODEX_HOME`；Claude Code 也使用 session-scoped env/settings 文件，避免并发会话互相覆盖。

## UI 形态

启动 Agent 时显示一个紧凑的配置条：

```text
[Claude Code] [Personal Anthropic] [claude-sonnet] [Connected]  ▾
```

点击后可查看或切换：端点、模型、认证引用、协议、超时、代理和能力开关。密钥只显示名称和末四位，不回显明文。

### 模型清单：检测、标记、剔除、保存

端点的模型列表只是「它说自己有哪些」，并不等于「它真的能跑哪些」：网关常常列出并不支持的模型，也会拒绝本来支持的模型。所以模型区是一个闭环，四步都在同一个表单里完成：

1. **拉取**（`discover-models`）：只读列表，不跑推理。
2. **检测**（`probe-models`）：用户点按钮才会发出，对每个模型发一次最小请求（1 个 token），端点按调用计费；单次请求最多 24 个、并发 4、单个 20 秒；界面按 8 个一批发送，边发边出结果，可随时停止。一次点击同样最多检测 24 个模型，多出来的会提示「还剩 N 个」，再点一次继续——一次点击就是一份用户已经同意的账单。
3. **标记**：每行给出结果——可用（带耗时）、模型不存在、失败、密钥被拒、被限流、超时、未检测。
4. **剔除**：「只保留可用」只留下已选中且检测通过的模型；「剔除失败项」只移除被端点明确拒绝的模型。限流、超时、密钥被拒不算模型的问题，不会被剔除——否则会误删本来能用的模型。

### 协议（protocol）

「能不能用」和「用哪种协议问」是两件事，而且答案不同：同一个模型可能在 `openai-completions` 上正常返回、在 `openai-responses` 上直接报错。所以检测结果带着协议，界面也把协议说出来：

- **Claude Code 只有一种协议**：无论端点是谁，都是 Anthropic Messages（`POST {base}/v1/messages`）。表单里不再出现协议下拉框，改为一句说明。端点 URL 末尾写不写 `/v1` 都行：检测和启动都会去掉它，否则请求会打到 `/v1/v1/messages`。
- **Codex 只有一种协议**：OpenAI Responses（`POST {base}/v1/responses`）。
- **Pi 可以选四种**：OpenAI Chat Completions / OpenAI Responses / Anthropic Messages / Google Generative AI，由 profile 的 `api` 字段决定；留空则按 URL 猜（含 `anthropic` 的主机猜 Messages，其余猜 Chat Completions）。

一个 404 有两种含义，而它们要的修法相反：模型不在清单里，或者端点根本没有这条路由。只讲一种协议时猜不出来——Claude Code 打 `/v1/messages`、Codex 打 `/v1/responses`，端点不提供该路由就会对每个模型都返回 404。所以当整批结果都是「未找到」时，面板会先提示「更像是端点不提供该协议」，而不是让人去删模型清单。

面板在检测前就显示「会用哪种协议、打到哪个路径」，数据来自 `probe-plan`——它只做规划、不发请求，所以表单可以随输入实时询问。协议是猜出来的时候会明确标注，因为猜错会表现为「整批模型都失败」，而不是一个明显的错误。检测完成后，摘要行会写明这批结果用的是哪种协议；只有当屏幕上的行来自不同协议时才逐行标注。

表单只有一个检测入口，按客户端走不同的流程。Claude Code 和 Codex 只有一种协议可用，所以先用默认模型试一次；默认模型都跑不通，就不再检测其余模型，因为结果只会一样。Pi 则先用默认模型把四种协议并发各试一次。检测时优先保留 profile 已填的协议，其次用按 URL 猜出的协议，最后取第一个跑通的。选定的协议会写回 `api`，再用它检测已选模型。每种协议的结果都会列出来，可以手动「改用这个」。

剔除后保存即生效：`models` 变成一份显式清单，会话的模型菜单只提供这些。不选任何模型仍然表示「提供全部」，所以剔除必须落成显式列表。官方账号（`native_config`）的模型由客户端自己验证，此接口会直接拒绝，提示改用会话验证。

## 兼容性分级

- L1：官方 CLI + 官方 endpoint，完整支持。
- L2：协议兼容网关，支持基础对话、工具调用和流式输出。
- L3：仅文本兼容或自定义网关，显示能力缺失，不承诺完整 Agent 行为。

Tendly 应在启动前做 capability probe，并明确告诉用户当前 endpoint 是否支持 tools、vision、streaming、structured output 和长上下文。

## 成本与风险

- Profile CRUD、session 绑定和启动配置生成：低到中等成本。
- 安全存储、并发会话、日志脱敏和配置快照：中等成本，必须早做。
- 让同一个自定义 endpoint 同时兼容 Claude Code 和 Codex：中到高成本，取决于网关是否实现两套协议和工具语义。

首版建议只保证“一个 provider 对应一个 profile”，不要承诺任意 endpoint 双向兼容。
