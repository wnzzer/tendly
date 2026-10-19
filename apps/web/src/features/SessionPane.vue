<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { EndpointProfile, ProviderKind, Session } from "@tendly/protocol";
import ChatSessionPane from "./ChatSessionPane.vue";
import NativeSessionPane from "./NativeSessionPane.vue";
import SessionCommonActions from "./SessionCommonActions.vue";
import { useI18n } from "../i18n";
import { sessionView, setSessionView, type SessionView } from "./session-view";
import SessionRunInfo from "./SessionRunInfo.vue";
import { liveEscapeHatch } from "./terminal-reopen";

const props = defineProps<{
  paneId?: string;
  session: Session;
  sessions: Session[];
  profiles: EndpointProfile[];
}>();

const emit = defineEmits<{
  changed: [];
  renameRequest: [id: string];
  environment: [id: string];
  profiles: [];
  create: [provider?: ProviderKind];
  select: [session: Session];
  reveal: [id: string];
  keep: [id: string];
  leave: [];
  openFile: [reference: { path: string; line?: number; checkout?: string | null }];
}>();

const { t } = useI18n();
const view = ref<SessionView>(sessionView(props.session));
const isTerminal = computed(() => props.session.provider === "terminal");
const isChat = computed(() => !isTerminal.value && view.value === "chat");
// An escape hatch has no chat view of its own: it reopens a conversation that
// is already on screen in the structured session it came from, so offering one
// here would show the same transcript twice under two different session ids.
const isReopen = computed(() => !!props.session.resume_source_id);
const canSwitch = computed(() => !isTerminal.value && !isReopen.value && props.session.interaction_mode !== "structured");
const showInfo = ref(false);
function showRunInfo(closeMenu: () => void) { closeMenu(); showInfo.value = true; }
watch(() => props.session.id, () => { showInfo.value = false; });

watch(
  () => [props.session.id, props.session.provider, props.session.interaction_mode] as const,
  () => { view.value = sessionView(props.session); },
  { immediate: true },
);

function choose(next: SessionView): void {
  if (!canSwitch.value || next === view.value) return;
  view.value = setSessionView(props.session, next);
}
function openNative(): void { choose("native"); }
function openChat(): void {
  // This only changes the view. It deliberately does not call /start or
  // enable structured mode; the native process and its session id are kept.
  choose("chat");
}
function onStructured(id: string): void {
  if (id === props.session.id) openChat();
}
function renameSession(id: string): void { emit("renameRequest", id); }
// The terminal this conversation was reopened in, while it runs: the two
// clients do not share what is said in either, so the view says where it is.
const liveTerminal = computed(() => isChat.value ? liveEscapeHatch(props.session, props.sessions) : undefined);
</script>

<template>
  <section class="session-shell" :data-session-id="session.id" :data-view="view" :aria-label="t('Session view for {session}', { session: session.title })">
    <div v-if="liveTerminal" class="inline-notice session-terminal-notice"><span>{{ t('This conversation is open in a terminal. Continue there, or leave the terminal to come back here.') }}</span><button type="button" class="small-button" @click="emit('select', liveTerminal)">{{ t('Go to terminal') }}</button></div>
    <div class="session-shell-content" :data-session-id="session.id">
      <ChatSessionPane
        v-if="isChat"
        :key="`chat:${session.id}`"
        :pane-id="paneId"
        :session="session"
        :profiles="profiles"
        :consume-open-intent="session.interaction_mode === 'structured'"
        @changed="emit('changed')"
        @rename-request="renameSession"
        @environment="emit('environment', $event)"
        @profiles="emit('profiles')"
        @legacy="openNative"
        @open-session="emit('select', $event)"
        @open-file="emit('openFile', $event)"
      >
        <template #session-actions="{ closeMenu }">
          <SessionCommonActions :session="session" :close-menu="closeMenu" @rename="renameSession(session.id)" @environment="emit('environment', session.id)" @keep="emit('keep', session.id)" @info="showInfo = true" />
        </template>
      </ChatSessionPane>
      <NativeSessionPane
        v-else
        :key="`native:${session.id}`"
        :pane-id="paneId"
        :session="session"
        :sessions="sessions"
        :profiles="profiles"
        :terminal="isTerminal"
        :consume-open-intent="isTerminal"
        @changed="emit('changed')"
        @rename-request="renameSession"
        @create="emit('create', $event)"
        @select="emit('select', $event)"
        @environment="emit('environment', $event)"
        @structured="onStructured"
        @profiles="emit('profiles')"
        @leave="emit('leave')"
      >
        <template #session-actions="{ closeMenu }">
          <SessionCommonActions :session="session" :close-menu="closeMenu" @rename="renameSession(session.id)" @environment="emit('environment', session.id)" @keep="emit('keep', session.id)" @info="showInfo = true" />
          <button v-if="canSwitch" type="button" class="session-shell-action" @click="closeMenu(); openChat()">{{ t('Chat') }}</button>
        </template>
      </NativeSessionPane>
    </div>

    <SessionRunInfo v-if="showInfo" :session="session" @close="showInfo = false" />
  </section>
</template>

<style scoped>
.session-shell{display:flex;flex:1;min-height:0;min-width:0;flex-direction:column;overflow:hidden;background:var(--surface,var(--surface))}
.session-terminal-notice{display:flex;align-items:center;justify-content:space-between;gap:10px;flex:none;margin:8px 8px 0}
.session-shell-content{display:flex;flex:1;min-height:0;min-width:0}.session-shell-content>*{flex:1 1 auto;width:auto;min-width:0;min-height:0}
</style>
