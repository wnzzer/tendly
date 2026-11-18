<script setup lang="ts">
import { computed, ref } from "vue";
import type { AgentProviderKind, EndpointProfile, Session } from "@tendly/protocol";
import type { ChatEvent, ConversationSnapshot } from "./chat-model";
import ChatSessionPane from "./ChatSessionPane.vue";
import ProviderIcon from "./ProviderIcon.vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";
import { AGENT_CLIENTS, DEFAULT_CLIENT, clientInfo, clientLabel, clientTone } from "./clients";

const { t, locale, setLocale } = useI18n();
const provider = ref<AgentProviderKind>(DEFAULT_CLIENT);
const phone = ref(false);
const profiles = computed<EndpointProfile[]>(() => [
  {
    id: "preview-official-" + provider.value,
    name: "官方账号 · Design lab",
    provider: provider.value,
    endpoint_url: null,
    model: null,
    permission_mode: "native",
    secret_ref: null,
    created_at: "2026-09-09T00:00:00Z",
  },
  {
    id: "preview-work-" + provider.value,
    name: "工作端点 · Preview only",
    provider: provider.value,
    endpoint_url: null,
    model: null,
    permission_mode: "native",
    secret_ref: null,
    created_at: "2026-09-09T00:00:00Z",
  },
]);
const session = computed<Session>(() => ({
  id: "readonly-ui-preview-" + provider.value,
  workspace_id: "preview-workspace",
  provider: provider.value,
  title: "让灵感，变成可以运行的东西",
  status: "running",
  interaction_mode: "structured",
  created_at: "2026-09-09T00:00:00Z",
  updated_at: "2026-09-09T00:00:00Z",
  endpoint_profile_id: profiles.value[0].id,
  endpoint_snapshot: profiles.value[0],
  provider_session_id: null,
  error: null,
}));
const snapshot = computed<ConversationSnapshot>(() => ({
  mode: "structured",
  running: true,
  truncated: false,
  events: [
    { seq: 1, type: "ready", native_session_id: "fixture-only-not-a-real-thread" },
    {
      seq: 2,
      type: "message",
      id: "preview-user",
      role: "user",
      text: "帮我做一个清爽的对话工作区。手机上也能舒服地看代码、确认权限、继续开发。",
    },
    {
      seq: 3,
      type: "message",
      id: "preview-assistant",
      role: "assistant",
      text: "可以。我们把注意力留给 **对话和正在发生的工作**。\n\n- 对话自然展开，工具结果按需查看\n- 输入框始终留在底部，手机上也不用找\n- 权限仍由原生客户端提出，由你决定",
    },
    {
      seq: 4,
      type: "tool",
      id: "preview-read",
      name: "read_file · ChatSessionPane.vue",
      status: "completed",
      text: "UI fixture only — no file was read.\n\nComponent: conversation workspace\nTransport: official client events\nLayout: responsive, fixed composer",
    },
    {
      seq: 5,
      type: "message",
      id: "preview-code",
      role: "assistant",
      text: '### 一个更专注的工作区\n\n把实现细节折叠起来，保留真正重要的信息：\n\n```ts\nconst workspace = {\n  view: "conversation",\n  permissions: "native",\n  layout: "adaptive",\n};\n```',
    },
    {
      seq: 6,
      type: "tool",
      id: "preview-change",
      name: "apply_patch · 3 display changes",
      status: "completed",
      text: "UI fixture only — no source file was modified.\n+ Responsive conversation layout\n+ Native tool cards\n+ Touch-friendly message composer",
    },
    { seq: 7, type: "turn", id: "preview-turn", status: "running" },
    {
      seq: 8,
      type: "approval",
      id: "preview-approval",
      title: "Allow Read?",
      text: JSON.stringify({
        reason: "读取工作区外的 nginx 配置，确认反向代理的端口（只读界面样例，不会授予权限）。",
        input: { file_path: "/etc/nginx/sites-enabled/tendly.conf" },
      }),
      choices: ["accept", "accept_session", "decline", "cancel"],
      scope: { access: "read", outside: ["/etc/nginx/sites-enabled/tendly.conf"] },
    },
    {
      seq: 9,
      type: "approval",
      id: "preview-question",
      title: "在继续之前，确认两个细节",
      text: "这是 **只读界面样例**，不会把回答发送给任何客户端。",
      choices: ["accept", "cancel"],
      questions: [
        {
          id: "layout",
          header: "布局",
          question: "审批卡片放在哪里？",
          options: [
            { label: "跟随对话", description: "卡片出现在时间线里，滚动即可回看" },
            { label: "固定在输入框上方", description: "始终可见，处理完再收起" },
          ],
          isOther: true,
        },
        {
          id: "targets",
          header: "范围",
          question: "这次先打磨哪些组件？",
          multiSelect: true,
          options: [
            { label: "权限请求" },
            { label: "Agent 提问" },
            { label: "工具卡片", description: "折叠态与运行态" },
          ],
        },
      ],
    },
    { seq: 10, type: "usage", input_tokens: 1250, output_tokens: 640, context_tokens: 7200, context_window: 200000 },
  ] as ChatEvent[],
}));
</script>

<template>
  <main :class="['preview-page', { phone }]">
    <header class="preview-top">
      <div class="preview-brand">
        <span>AD<span /></span><strong>Tendly</strong><small>Design lab</small>
      </div>
      <span class="preview-disclaimer">{{ t("UI preview · no client or model") }}</span>
      <div class="preview-viewport" :aria-label="t('Preview viewport')">
        <button :class="{ active: !phone }" :aria-pressed="!phone" @click="phone = false">
          {{ t("Desktop") }}</button><button :class="{ active: phone }" :aria-pressed="phone" @click="phone = true">
          {{ t("Phone · 390px") }}
        </button>
      </div>
      <button @click="setLocale(locale === 'zh-CN' ? 'en' : 'zh-CN')">{{ locale === "zh-CN" ? "EN" : "中文" }}</button>
    </header>
    <section class="preview-frame">
      <aside class="preview-sidebar">
        <div class="preview-space">
          <span class="preview-space-icon"><Icon name="folder" :size="21" /></span>
          <div><strong>Tendly</strong><small>Workspace preview</small></div>
          <span class="preview-count">03</span>
        </div>
        <p class="preview-group-label">{{ t("Preview conversations") }}</p>
        <button
          v-for="client in AGENT_CLIENTS"
          :key="client"
          :class="['preview-session', { active: provider === client }]"
          @click="provider = client"
        >
          <span :class="['preview-provider', client]" :style="clientTone(client)"
            ><ProviderIcon :provider="client" :size="19"
          /></span>
          <div>
            <strong>{{ clientLabel(client) }}</strong><small>{{ t("Conversation and tools") }}</small>
          </div>
          <span class="preview-session-dot" />
        </button>
        <div class="preview-divider" />
        <p class="preview-group-label">{{ t("Workspace context") }}</p>
        <div class="preview-static-row"><Icon name="git" :size="17" /><span>main</span><small>+24 −8</small></div>
        <div class="preview-static-row"><Icon name="file" :size="17" /><span>ChatSessionPane.vue</span></div>
        <div class="preview-sidebar-foot">
          <span>✦</span>
          <p>{{ t("Local fixtures only. Nothing in your workspace is changed.") }}</p>
        </div>
      </aside>
      <div class="preview-chat">
        <nav class="preview-mobile-tabs">
          <button
            v-for="client in AGENT_CLIENTS"
            :key="client"
            :class="{ active: provider === client }"
            @click="provider = client"
          >
            <ProviderIcon :provider="client" :size="16" />{{ clientInfo(client).shortLabel }}</button
          ><span>{{ t("Read-only UI preview") }}</span>
        </nav>
        <ChatSessionPane :key="session.id" :session="session" :profiles="profiles" :preview-snapshot="snapshot" />
      </div>
    </section>
    <footer class="preview-footer">
      {{
        t("Type locally, explore tool cards, and resize the page. Send, approvals and endpoint changes are disabled.")
      }}
    </footer>
  </main>
</template>

<style scoped>
.preview-page {
  height: 100dvh;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--ink);
  padding: 0 26px 10px;
  font-family: var(--font-body);
}
.preview-top {
  height: 77px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 14px;
  max-width: 1440px;
  width: 100%;
  margin: 0 auto;
}
.preview-brand {
  display: flex;
  align-items: center;
  gap: 11px;
}
.preview-brand > span {
  position: relative;
  background: var(--accent);
  color: var(--surface);
  width: 35px;
  height: 35px;
  display: grid;
  place-items: center;
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  font-weight: 650;
  letter-spacing: 0.8px;
}
.preview-brand > span > span {
  position: absolute;
  width: 7px;
  height: 7px;
  background: var(--teal-soft);
  border: 2px solid var(--bg);
  border-radius: 50%;
  right: -2px;
  bottom: -2px;
}
.preview-brand strong {
  font-size: var(--text-xl);
  letter-spacing: -0.5px;
  font-weight: 600;
}
.preview-brand small {
  font-size: var(--text-xs);
  color: var(--muted);
  border-left: 1px solid var(--border);
  padding-left: 11px;
}
.preview-disclaimer {
  font-size: var(--text-xs);
  color: var(--teal);
  background: var(--teal-soft);
  border: 1px solid var(--border);
  border-radius: 30px;
  padding: 8px 12px;
  margin-left: auto;
}
.preview-top > button {
  min-width: 44px;
  min-height: 44px;
  border: 0;
  background: none;
  color: var(--ink-soft);
  font-size: var(--text-sm);
  cursor: pointer;
}
.preview-frame {
  display: flex;
  flex: 1;
  min-height: 0;
  max-width: 1440px;
  width: 100%;
  margin: 0 auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-xl);
  overflow: hidden;
  box-shadow: var(--shadow-xl);
  background: var(--surface);
}
.preview-sidebar {
  display: flex;
  flex-direction: column;
  width: 236px;
  flex-shrink: 0;
  background: var(--sunken);
  border-right: 1px solid var(--border);
  padding: 25px 14px;
}
.preview-space {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 8px 24px;
}
.preview-space-icon {
  width: 37px;
  height: 37px;
  display: grid;
  place-items: center;
  border-radius: var(--radius-lg);
  background: var(--teal-soft);
  color: var(--teal);
}
.preview-space > div {
  min-width: 0;
  flex: 1;
}
.preview-space strong {
  display: block;
  font-size: var(--text-md);
  font-weight: 600;
}
.preview-space small {
  display: block;
  font-size: var(--text-xs);
  color: var(--muted);
  margin-top: 5px;
}
.preview-count {
  font-size: var(--text-xs);
  color: var(--teal);
  background: var(--teal-soft);
  padding: 4px 6px;
  border-radius: var(--radius-sm);
}
.preview-group-label {
  font-size: var(--text-xs);
  color: var(--muted);
  margin: 6px 9px 13px;
  letter-spacing: 0.6px;
}
.preview-session {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  min-height: 64px;
  padding: 12px 10px;
  margin: 3px 0;
  border: 1px solid transparent;
  border-radius: var(--radius-lg);
  background: none;
  text-align: left;
  cursor: pointer;
}
.preview-session.active {
  background: var(--teal-soft);
  border-color: var(--border);
}
.preview-provider {
  display: grid;
  place-items: center;
  width: 33px;
  height: 33px;
  border-radius: var(--radius-md);
  background: var(--client-soft);
  color: var(--client-ink);
}
.preview-session > div {
  min-width: 0;
  flex: 1;
}
.preview-session strong {
  display: block;
  font-size: var(--text-xs);
  font-weight: 550;
  color: var(--ink);
}
.preview-session small {
  display: block;
  font-size: var(--text-xs);
  color: var(--muted);
  margin-top: 5px;
}
.preview-session-dot {
  width: 5px;
  height: 5px;
  background: var(--teal);
  border-radius: 50%;
}
.preview-divider {
  border-top: 1px solid var(--border);
  margin: 24px 6px 14px;
}
.preview-static-row {
  display: flex;
  align-items: center;
  gap: 9px;
  min-height: 41px;
  padding: 7px 11px;
  font-size: var(--text-xs);
  color: var(--ink-soft);
}
.preview-static-row small {
  margin-left: auto;
  font-size: var(--text-xs);
  color: var(--teal);
}
.preview-sidebar-foot {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  padding: 18px 9px 0;
  margin-top: auto;
  color: var(--ink-soft);
}
.preview-sidebar-foot p {
  font-size: var(--text-xs);
  line-height: 1.8;
  margin: 0;
}
.preview-chat {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.preview-chat > :deep(.chat-pane) {
  flex: 1;
  min-height: 0;
  height: auto;
}
.preview-mobile-tabs {
  display: none;
}
.preview-footer {
  max-width: 1440px;
  width: 100%;
  margin: 0 auto;
  padding: 11px 10px 1px;
  font-size: var(--text-xs);
  text-align: center;
  color: var(--muted);
}
@media (max-width: 700px) {
  .preview-page {
    padding: 0 7px 7px;
  }
  .preview-top {
    height: 62px;
    gap: 6px;
    padding: 0 4px;
  }
  .preview-brand {
    gap: 8px;
  }
  .preview-brand strong {
    font-size: var(--text-lg);
  }
  .preview-brand small {
    display: none;
  }
  .preview-brand > span {
    width: 29px;
    height: 29px;
    border-radius: var(--radius-md);
    font-size: var(--text-xs);
  }
  .preview-disclaimer {
    font-size: var(--text-xs);
    max-width: 46%;
    padding: 5px 8px;
    text-align: center;
    line-height: 1.5;
  }
  .preview-top > button {
    font-size: var(--text-xs);
  }
  .preview-frame {
    border-radius: var(--radius-lg);
  }
  .preview-sidebar {
    display: none;
  }
  .preview-mobile-tabs {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 9px;
    background: var(--sunken);
    border-bottom: 1px solid var(--border);
    min-height: 50px;
    flex-shrink: 0;
  }
  .preview-mobile-tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 44px;
    border: 0;
    border-radius: var(--radius-md);
    padding: 8px 11px;
    background: none;
    color: var(--ink-soft);
    font-size: var(--text-sm);
  }
  .preview-mobile-tabs button.active {
    background: var(--teal-soft);
    color: var(--teal);
  }
  .preview-mobile-tabs > span {
    margin-left: auto;
    color: var(--muted);
    font-size: var(--text-xs);
  }
  .preview-footer {
    font-size: var(--text-xs);
    line-height: 1.5;
    padding: 7px 10px 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  * {
    scroll-behavior: auto;
  }
}
.preview-viewport {
  display: flex;
  align-items: center;
  gap: 3px;
  padding: 3px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
}
.preview-viewport button {
  min-height: 44px;
  padding: 6px 12px;
  border: 0;
  border-radius: var(--radius-md);
  background: none;
  font-size: var(--text-xs);
  color: var(--ink-soft);
  cursor: pointer;
}
.preview-viewport button.active {
  background: var(--teal-soft);
  color: var(--teal);
}
.preview-page.phone .preview-frame {
  max-width: 390px;
  border-radius: var(--radius-xl);
}
.preview-page.phone .preview-sidebar {
  display: none;
}
.preview-page.phone .preview-mobile-tabs {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 5px 9px;
  background: var(--sunken);
  border-bottom: 1px solid var(--border);
  min-height: 50px;
  flex-shrink: 0;
}
.preview-page.phone .preview-mobile-tabs button {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 44px;
  border: 0;
  border-radius: var(--radius-md);
  padding: 8px 11px;
  background: none;
  color: var(--ink-soft);
  font-size: var(--text-sm);
}
.preview-page.phone .preview-mobile-tabs button.active {
  background: var(--teal-soft);
  color: var(--teal);
}
.preview-page.phone .preview-mobile-tabs > span {
  margin-left: auto;
  color: var(--muted);
  font-size: var(--text-xs);
}
@media (max-width: 700px) {
  .preview-top {
    height: auto;
    min-height: 99px;
    flex-wrap: wrap;
    padding: 7px 4px;
    column-gap: 5px;
    row-gap: 2px;
  }
  .preview-disclaimer {
    order: 4;
    width: 100%;
    max-width: none;
    margin: 0;
    border: 0;
    background: none;
    padding: 3px 0;
  }
  .preview-viewport {
    margin-left: auto;
    padding: 1px;
    gap: 0;
  }
  .preview-viewport button {
    padding: 6px 7px;
    font-size: var(--text-xs);
  }
  .preview-top > button {
    min-width: 44px;
  }
}
</style>
