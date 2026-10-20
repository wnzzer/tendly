<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue';
import type { EndpointProfile } from '@tendly/protocol';
import PageShell from './PageShell.vue';
import Icon from './Icon.vue';
import AgentClientsPanel from './AgentClientsPanel.vue';
import ProfilesDialog from './ProfilesDialog.vue';
import AccountsDialog from './AccountsDialog.vue';
import PreferencesPanel from './PreferencesPanel.vue';
import UpdatePanel from './UpdatePanel.vue';
import RemoteAccessPanel from './RemoteAccessPanel.vue';
import { backendCapabilities } from './backend-capabilities';
import { useI18n } from '../i18n';

import { useRouter } from 'vue-router';
import type { SettingsSection } from '../router';

/**
 * Settings, as a page with an address per section (/settings/endpoints). It
 * was a dialog holding two other dialogs; it is a place you go now, so the
 * browser's back button, a reload and a link all work.
 */
const props = defineProps<{ profiles: EndpointProfile[]; section: SettingsSection }>();
const emit = defineEmits<{ back: []; navigate: [section: SettingsSection]; changed: [profile?: EndpointProfile] }>();
const { t } = useI18n();

const ALL_SECTIONS = [
  { id: 'preferences', icon: 'gauge', label: 'Preferences' },
  { id: 'agents', icon: 'spark', label: 'Agent clients' },
  { id: 'endpoints', icon: 'settings', label: 'Endpoint profiles' },
  { id: 'accounts', icon: 'account', label: 'Official accounts' },
  { id: 'remote', icon: 'globe', label: 'Remote access' },
  { id: 'updates', icon: 'download', label: 'Updates' },
] as const;
/** An older server lacks some APIs, so it gets no page that would fail. */
const SECTIONS = computed(() => ALL_SECTIONS.filter(entry =>
  (entry.id !== 'updates' || backendCapabilities.selfUpdate) && (entry.id !== 'remote' || backendCapabilities.tailnet)));

const section = computed(() => SECTIONS.value.some(entry => entry.id === props.section) ? props.section : 'preferences');
const profilesPage = ref<{ requestLeave: (action: () => void) => void }>();

/**
 * The endpoints page guards unsaved edits, so leaving it, by any route, asks
 * it first. A choice to stay simply never resolves the navigation, and the
 * next one replaces it.
 */
const removeGuard = useRouter().beforeEach((to, from) => {
  if (from.name !== 'settings' || section.value !== 'endpoints' || !profilesPage.value || to.fullPath === from.fullPath) return true;
  const page = profilesPage.value;
  return new Promise<boolean>(resolve => page.requestLeave(() => resolve(true)));
});
onBeforeUnmount(removeGuard);
function select(next: SettingsSection) { if (next !== section.value) emit('navigate', next); }
</script>

<template>
  <PageShell named-above :title="t('Settings')" @back="emit('back')">
    <div class="settings-layout">
      <nav class="settings-nav" data-arrow-nav :aria-label="t('Settings sections')">
        <button v-for="entry in SECTIONS" :key="entry.id" type="button" :class="['settings-nav-item',{selected:section===entry.id}]" :aria-current="section===entry.id?'page':undefined" @click="select(entry.id)">
          <Icon :name="entry.icon" :size="16"/>
          <strong>{{ t(entry.label) }}</strong>
        </button>
      </nav>
      <div class="settings-page">
        <PreferencesPanel v-if="section==='preferences'" :profiles="profiles"/>
        <AgentClientsPanel v-else-if="section==='agents'"/>
        <ProfilesDialog v-else-if="section==='endpoints'" ref="profilesPage" embedded :profiles="profiles" @close="emit('back')" @changed="emit('changed', $event)" @accounts="select('accounts')"/>
        <UpdatePanel v-else-if="section==='updates'"/>
        <RemoteAccessPanel v-else-if="section==='remote'"/>
        <AccountsDialog v-else embedded @close="emit('back')" @changed="emit('changed', $event)"/>
      </div>
    </div>
  </PageShell>
</template>

<style scoped>
/* A page now: the section list stays put beside the content, which scrolls
   with the page. */
.settings-layout{display:grid;grid-template-columns:200px minmax(0,1fr);gap:var(--space-6);align-items:start}
.settings-nav{position:sticky;top:0;display:flex;flex-direction:column;gap:2px}
.settings-nav-item{display:flex;align-items:center;gap:10px;width:100%;min-height:38px;text-align:left;padding:8px 10px;border:0;border-radius:var(--radius-md);background:none;color:var(--ink-soft);cursor:pointer;font:inherit}
.settings-nav-item:hover{background:var(--fill);color:var(--ink)}
.settings-nav-item.selected{background:var(--selected);color:var(--ink)}
.settings-nav-item.selected>svg:first-child{color:var(--accent)}
.settings-nav-item strong{font-size:var(--text-md);font-weight:550;color:inherit}
.settings-nav-item:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
.settings-page{min-width:0;max-width:820px}
@media(max-width:760px){
  .settings-layout{grid-template-columns:minmax(0,1fr);gap:var(--space-3)}
  .settings-nav{position:static;flex-direction:row;border-bottom:1px solid var(--border);padding:0 0 var(--space-2);overflow-x:auto}
  .settings-nav-item{width:auto;flex-shrink:0;min-height:44px}
}
</style>
