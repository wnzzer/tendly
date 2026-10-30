<script setup lang="ts">
import { inject } from "vue";
import type { Session } from "@tendly/protocol";
import { useI18n } from "../i18n";
import { isEphemeralSession, resumeCommand } from "./session-list";
import { clientLabel } from "./clients";
import Icon from "./Icon.vue";
import CopyButton from "./CopyButton.vue";
import MenuFlyout from "./MenuFlyout.vue";
import { useMobile } from "./mobile";
import { IS_REMOTE_PANE } from "./remote-session";
const props = defineProps<{ session: Session; keepBusy?: boolean; closeMenu?: () => void }>();
const emit = defineEmits<{ rename: []; environment: []; keep: []; info: [] }>();
const { t } = useI18n();
const remotePane = inject(IS_REMOTE_PANE, false);
const mobile = useMobile();
const hasNativeId = () => !!props.session.provider_session_id && props.session.provider !== 'terminal';
function showRunInfo() { props.closeMenu?.(); emit("info"); }
function act(action: "rename" | "environment" | "keep") {
  props.closeMenu?.();
  if (action === "rename") emit("rename");
  else if (action === "environment") emit("environment");
  else emit("keep");
}
</script>
<template>
  <button v-if="!remotePane" class="session-rename-action" role="menuitem" @click="act('rename')"><Icon name="edit" :size="14" />{{ t('Rename session') }}</button>
  <button v-if="!remotePane" class="session-environment-action" role="menuitem" @click="act('environment')"><Icon name="settings" :size="14" />{{ t('Session environment') }}</button>
  <button v-if="isEphemeralSession(session) && !remotePane" class="session-keep-action" role="menuitem" :disabled="keepBusy" :aria-busy="keepBusy" @click="act('keep')"><Icon name="check" :size="14" />{{ t(keepBusy ? 'Saving…' : 'Keep this session') }}</button>
  <!-- Two copies of the same session, so one row: the menu had grown a line per thing to copy. -->
  <template v-if="mobile">
    <CopyButton v-if="resumeCommand(session)" :text="resumeCommand(session)!" label="Copy resume command" menu-item />
    <CopyButton v-if="session.provider_session_id && session.provider !== 'terminal'" :text="session.provider_session_id" :label="t('Copy {client} session ID', { client: clientLabel(session.provider) })" menu-item />
  </template>
  <MenuFlyout v-else-if="resumeCommand(session) || hasNativeId()" :label="t('Copy')" icon="clipboard" :width="210">
    <CopyButton v-if="resumeCommand(session)" :text="resumeCommand(session)!" label="Resume command" menu-item />
    <CopyButton v-if="session.provider_session_id && session.provider !== 'terminal'" :text="session.provider_session_id" :label="t('{client} session ID', { client: clientLabel(session.provider) })" menu-item />
  </MenuFlyout>
  <button role="menuitem" @click="showRunInfo"><Icon name="info" :size="14" />{{ t('Run information') }}</button>
</template>

<style scoped>
button{display:flex;align-items:center;gap:8px;width:100%;min-height:30px;padding:6px 9px;border:0;border-radius:var(--radius-sm);background:none;text-align:left;font:inherit;font-size:var(--text-sm);color:var(--ink-soft);cursor:pointer}
button:hover:not(:disabled){background:var(--fill)}button:focus-visible{outline:2px solid var(--focus);outline-offset:1px}button:disabled{opacity:.5}
@media(pointer:coarse){button{min-height:44px}}
</style>
