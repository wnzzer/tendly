<script setup lang="ts">
/**
 * The endpoint profiles a session can run on, and the other clients it can
 * hand its conversation to. A desktop shows them in the endpoint chip's menu;
 * a phone, in the model chip's card (`card`), where the models are.
 *
 * It only says what was picked; switching is the session's.
 */
import { computed } from "vue";
import type { EndpointProfile, Session } from "@tendly/protocol";
import BalanceBadge from "./BalanceBadge.vue";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";
import { providerLabel } from "./api";
import { balanceFor, balances, refreshBalance } from "./balance-store";
import { profileNote, type HandoffChoice } from "./handoff";
import { useI18n } from "../i18n";

const props = defineProps<{
  session: Session;
  /** The profiles for this session's client. */
  profiles: EndpointProfile[];
  /** The profile chosen, before the switch has gone through. */
  selected: string;
  /** No turn or approval is in the way of a switch. */
  configurable: boolean;
  /** How the session's endpoint is named, for one no longer in the list. */
  name: string;
  handoffs: HandoffChoice[];
  /** The phone's card layout, rather than the chip's menu. */
  card?: boolean;
  preview?: boolean;
}>();
const emit = defineEmits<{ pick: [id: string]; handoff: [choice: HandoffChoice]; manage: [] }>();
const { t } = useI18n();

/** One row per way the session can be configured: how it is now, if no profile says so, then each profile. */
const options = computed(() => {
  const rows: { id: string; title?: string; note: string; profile?: EndpointProfile }[] = [];
  if (props.session.native_source_id && !props.session.endpoint_profile_id)
    rows.push({
      id: "original",
      title: t("Original client configuration"),
      note: t("Loaded history uses the original client settings. It does not take over another running terminal."),
    });
  else if (!props.session.endpoint_profile_id)
    rows.push({
      id: "blank",
      title: t("Blank configuration · separate sign-in"),
      note: t(
        "A blank home of its own, without the host's sign-in or settings. New sessions no longer start this way; switch it to an endpoint profile.",
      ),
    });
  for (const profile of props.profiles) rows.push({ id: profile.id, note: profileNote(profile, t), profile });
  return rows;
});
/** The session's profile, when it is not one this client's list still offers. */
const unlisted = computed(
  () => !!props.session.endpoint_profile_id && !props.profiles.some(p => p.id === props.session.endpoint_profile_id),
);
const current = computed(() => props.session.endpoint_profile_id ?? undefined);
</script>

<template>
  <nav v-if="!card" :aria-label="t('Endpoint profile')">
    <p v-if="!configurable" class="chat-menu-note">
      {{ t("Wait for the current turn and approvals before changing endpoints.") }}
    </p>
    <button
      v-for="option in options"
      :key="option.id"
      type="button"
      :class="{ selected: !option.profile || selected === option.id }"
      :disabled="!option.profile || !configurable"
      @click="option.profile && emit('pick', option.id)"
    >
      <strong v-if="option.profile"
        ><span>{{ option.profile.name }}</span
        ><BalanceBadge :profile-id="option.id" /></strong
      ><strong v-else>{{ option.title }}</strong
      ><small>{{ option.note }}</small></button
    ><button v-if="unlisted" type="button" class="selected" disabled>{{ name }}</button
    ><template v-if="handoffs.length"
      ><p class="chat-menu-section">
        {{ t("Switch to another client")
        }}<small>{{
          t("Opens a new session with this conversation in its message box; this one stays as it is.")
        }}</small>
      </p>
      <button
        v-for="choice in handoffs"
        :key="choice.provider + ':' + (choice.profileId ?? '')"
        type="button"
        class="chat-handoff-choice"
        @click="emit('handoff', choice)"
      >
        <strong
          ><ProviderIcon :provider="choice.provider" :size="12" /><span
            >{{ providerLabel(choice.provider) }} · {{ choice.name }}</span
          ><BalanceBadge :profile-id="choice.profileId" /></strong
        ><small>{{ choice.note }}</small>
      </button></template
    ><button
      v-if="!preview && current && balanceFor(current)"
      type="button"
      class="chat-endpoint-manage"
      :disabled="balances.busy.includes(current)"
      @click="refreshBalance(current)"
    >
      <Icon name="refresh" :size="13" />{{
        t(balances.busy.includes(current) ? "Querying balance…" : "Query this endpoint's balance again")
      }}</button
    ><button type="button" class="chat-endpoint-manage" @click="emit('manage')">
      <Icon name="account" :size="13" />{{ t("Manage endpoint profiles") }}
    </button>
  </nav>
  <div v-else class="chat-card-endpoints">
    <p class="chat-card-group">
      <span>{{ t("Endpoint profile") }}</span>
    </p>
    <p v-if="!configurable" class="chat-menu-note">
      {{ t("Wait for the current turn and approvals before changing endpoints.") }}
    </p>
    <nav :aria-label="t('Endpoint profile')">
      <button
        v-for="option in options"
        :key="option.id"
        type="button"
        :class="{ selected: !option.profile || selected === option.id }"
        :disabled="!option.profile || !configurable"
        @click="option.profile && emit('pick', option.id)"
      >
        <strong v-if="option.profile"
          ><span>{{ option.profile.name }}</span
          ><BalanceBadge :profile-id="option.id" /></strong
        ><strong v-else>{{ option.title }}</strong
        ><small>{{ option.note }}</small>
      </button>
    </nav>
    <details v-if="handoffs.length" class="chat-card-handoff">
      <summary>
        <Icon name="sync" :size="14" /><span>{{ t("Switch to another client") }}</span
        ><Icon name="chevron" :size="12" class="chat-card-handoff-caret" />
      </summary>
      <p>
        {{ t("Opens a new session with this conversation in its message box; this one stays as it is.") }}
      </p>
      <nav>
        <button
          v-for="choice in handoffs"
          :key="choice.provider + ':' + (choice.profileId ?? '')"
          type="button"
          @click="emit('handoff', choice)"
        >
          <strong
            ><ProviderIcon :provider="choice.provider" :size="12" /><span
              >{{ providerLabel(choice.provider) }} · {{ choice.name }}</span
            ><BalanceBadge :profile-id="choice.profileId" /></strong
          ><small>{{ choice.note }}</small>
        </button>
      </nav>
    </details>
    <button type="button" class="chat-card-manage" @click="emit('manage')">
      <Icon name="account" :size="14" />{{ t("Manage endpoint profiles") }}
    </button>
  </div>
</template>

<style scoped>
/* The pane's defaults for its controls (ChatSessionPane). Its scoped rules do
   not reach into this component, so they are repeated here, first, where the
   pane has them. */
.chat-pane button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
.chat-pane button:focus-visible,
.chat-pane summary:focus-visible,
.chat-pane select:focus-visible,
.chat-pane input:focus-visible,
.chat-pane textarea:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 3px;
}
@media (prefers-reduced-motion: reduce) {
  .chat-pane * {
    scroll-behavior: auto;
  }
}
/* The list looks the same in the endpoint chip's menu and in the model chip's
   card on a phone, so the model chip's list rules are repeated for it here. */
.chat-model nav button strong {
  display: block;
  font-size: var(--text-sm);
  font-weight: 600;
}
.chat-model nav button small {
  display: block;
  margin-top: 2px;
  font-size: var(--text-xs);
  color: var(--muted);
  white-space: normal;
  line-height: 1.5;
}
.chat-model nav button {
  white-space: normal;
}
.chat-endpoint nav button,
.chat-model nav button {
  display: block;
  width: 100%;
  min-height: 38px;
  padding: 8px 10px;
  border: 0;
  border-radius: var(--radius-md);
  background: none;
  text-align: left;
  font-size: var(--text-sm);
  color: var(--ink-soft);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.chat-endpoint nav button:hover:not(:disabled),
.chat-model nav button:hover:not(:disabled) {
  background: var(--fill);
}
.chat-endpoint nav button.selected,
.chat-model nav button.selected {
  color: var(--accent-ink);
  font-weight: 600;
  background: var(--selected);
}
.chat-endpoint nav button {
  white-space: normal;
}
.chat-endpoint nav button strong {
  display: flex;
  align-items: baseline;
  gap: 12px;
  font-size: var(--text-sm);
  font-weight: 600;
  white-space: nowrap;
}
.chat-endpoint nav button strong > span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.chat-endpoint nav button strong > .balance-badge {
  margin-left: auto;
}
.chat-endpoint nav button small {
  display: block;
  margin-top: 2px;
  font-size: var(--text-xs);
  line-height: 1.5;
  color: var(--muted);
  font-weight: 400;
  overflow-wrap: anywhere;
}

.chat-menu-section {
  margin: 6px 8px 3px;
  padding-top: 8px;
  border-top: 1px solid var(--line);
  font-size: var(--text-xs);
  font-weight: 600;
  letter-spacing: 0.3px;
  color: var(--muted);
}
.chat-menu-section small {
  display: block;
  margin-top: 2px;
  font-weight: 400;
  letter-spacing: 0;
  line-height: 1.45;
}
.chat-endpoint nav button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.chat-endpoint nav button.chat-endpoint-manage {
  display: flex;
  align-items: center;
  gap: 7px;
  min-height: 32px;
  margin-top: 4px;
  border-top: 1px solid var(--line);
  border-radius: 0 0 var(--radius-md) var(--radius-md);
  font-size: var(--text-xs);
  color: var(--ink-soft);
}
.chat-handoff-choice strong {
  display: flex !important;
  align-items: center;
  gap: 6px;
}
.chat-card-endpoints {
  border-top: 1px solid var(--line);
  margin-top: 6px;
  padding-top: 2px;
}
.chat-card-endpoints nav button {
  display: block;
  width: 100%;
  border: 0;
  background: none;
  text-align: left;
  color: var(--ink-soft);
  cursor: pointer;
}
.chat-card-endpoints nav button strong {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-weight: 600;
}
.chat-card-endpoints nav button strong > span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chat-card-endpoints nav button small {
  display: block;
  margin-top: 2px;
  color: var(--muted);
  overflow-wrap: anywhere;
}
.chat-card-endpoints nav button.selected {
  background: var(--selected);
}
.chat-card-handoff {
  border-top: 1px solid var(--line);
  margin-top: 4px;
}
.chat-card-handoff > summary {
  list-style: none;
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 48px;
  padding: 0 12px;
  font-size: var(--text-base);
  color: var(--ink-soft);
  cursor: pointer;
}
.chat-card-handoff > summary::-webkit-details-marker {
  display: none;
}
.chat-card-handoff > summary > span {
  flex: 1;
}
.chat-card-handoff-caret {
  transform: rotate(90deg);
  opacity: 0.55;
  transition: transform 0.15s;
}
.chat-card-handoff[open] .chat-card-handoff-caret {
  transform: rotate(-90deg);
}
.chat-card-handoff > p {
  margin: 0 12px 6px;
  font-size: var(--text-sm);
  line-height: 1.5;
  color: var(--muted);
}
.chat-card-handoff nav button strong {
  display: flex;
  align-items: center;
  gap: 6px;
}
.chat-card-manage {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-height: 48px;
  padding: 0 12px;
  border: 0;
  border-top: 1px solid var(--line);
  background: none;
  font: inherit;
  font-size: var(--text-base);
  color: var(--ink-soft);
  cursor: pointer;
}
</style>
