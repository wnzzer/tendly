<script setup lang="ts">
/**
 * Requests waiting on you, above the composer where your attention already is,
 * one at a time with a pager: a permission to grant, or an agent's questions
 * to answer. The timeline keeps a one-line record of each.
 *
 * It draws a request and collects the answers (useApprovalAnswers, held by the
 * session); deciding is the session's.
 */
import { computed, reactive } from "vue";
import type { Session } from "@tendly/protocol";
import Icon from "./Icon.vue";
import { MarkdownContent } from "./MarkdownContent";
import type { ApprovalAnswers, ApprovalItem } from "./approval-answers";
import { ACCESS_LABEL, approvalBody, approvalChoices, DECISION_LABEL, type ApprovalDecision } from "./chat-approvals";
import { useMobile } from "./mobile";
import { CHORDS, formatChord } from "./shortcuts";
import { useI18n } from "../i18n";

const props = defineProps<{
  session: Session;
  /** The requests still waiting, oldest first. */
  pending: ApprovalItem[];
  /** Which of them is shown. */
  index: number;
  answers: ApprovalAnswers;
  /** The request being decided, while its answer is on its way. */
  busy: string;
  /** The session's stream is up, so a decision can reach the client. */
  connected: boolean;
  preview: boolean;
  openFile: (reference: { path: string; line?: number }) => void;
}>();
const emit = defineEmits<{
  "update:index": [index: number];
  approve: [item: ApprovalItem, decision: ApprovalDecision];
}>();
const { t } = useI18n();
const mobile = useMobile();
const docked = computed(() => props.pending[props.index]);
/** Requests folded to their title line, to read the conversation above them. A new request arrives open. */
const foldedApprovals = reactive(new Set<string>());
function toggleApprovalFold(id: string) {
  if (!foldedApprovals.delete(id)) foldedApprovals.add(id);
}
const acceptKey = formatChord(CHORDS.confirm),
  declineKey = formatChord(CHORDS.remove);
</script>

<template>
  <section class="chat-approval-dock" :aria-label="t('Waiting for you')">
    <div v-if="pending.length > 1" class="chat-approval-pager">
      <span>{{ t("{index} of {count} waiting for you", { index: index + 1, count: pending.length }) }}</span><button
        type="button"
        class="icon-button"
        :disabled="index === 0"
        :aria-label="t('Previous request')"
        @click="emit('update:index', index - 1)"
      >
        <Icon name="chevron" :size="14" class="pager-back" /></button
      ><button
        type="button"
        class="icon-button"
        :disabled="index >= pending.length - 1"
        :aria-label="t('Next request')"
        @click="emit('update:index', index + 1)"
      >
        <Icon name="chevron" :size="14" />
      </button>
    </div>
    <article
      v-for="item in [docked]"
      :key="item.id"
      :class="[
        'chat-approval',
        item.questions.length ? 'is-question' : 'is-permission',
        { resolved: item.resolved, folded: foldedApprovals.has(item.id) },
      ]"
    >
      <header class="chat-approval-head">
        <span class="chat-approval-icon"
          ><Icon :name="item.resolved ? 'check' : item.questions.length ? 'spark' : 'shield'" :size="15"
        /></span>
        <div class="chat-approval-heading">
          <small>{{ t(item.questions.length ? "Question from the agent" : "Permission request") }}</small><strong>{{
            item.title
          }}</strong>
        </div>
        <span v-if="item.resolved" class="chat-approval-state">{{ t("Resolved") }}</span><button
          v-else
          type="button"
          class="icon-button chat-approval-fold"
          :aria-expanded="!foldedApprovals.has(item.id)"
          :aria-label="t(foldedApprovals.has(item.id) ? 'Expand' : 'Collapse')"
          :title="t(foldedApprovals.has(item.id) ? 'Expand' : 'Collapse')"
          @click="toggleApprovalFold(item.id)"
        >
          <Icon name="chevron" :size="14" />
        </button>
      </header>
      <div class="chat-approval-body">
        <template v-if="approvalBody(item.text)"
          ><p v-if="approvalBody(item.text)!.reason">{{ approvalBody(item.text)!.reason }}</p></template
        ><MarkdownContent v-else :text="item.text" :open-file="openFile" />
        <div v-if="item.scope" class="chat-approval-scope">
          <span v-if="item.scope.access" :class="['scope-access', item.scope.access]">{{
            t(ACCESS_LABEL[item.scope.access]!)
          }}</span><template v-if="item.scope.outside?.length"
            ><strong class="scope-outside"><Icon name="info" :size="13" />{{ t("Outside the workspace") }}</strong
            ><code v-for="path in item.scope.outside" :key="path" class="scope-path">{{ path }}</code></template
          >
          <pre v-if="item.scope.command" class="scope-command">{{ item.scope.command }}</pre>
        </div>
        <details v-if="approvalBody(item.text)?.detail" class="chat-approval-detail">
          <summary>{{ t("Details") }}</summary>
          <pre>{{ approvalBody(item.text)!.detail }}</pre>
        </details>
      </div>
      <template v-if="!item.resolved">
        <fieldset
          v-for="question in item.questions"
          :key="question.id"
          class="chat-question"
          :disabled="preview || !!busy"
        >
          <legend>
            <span v-if="question.header" class="chat-question-tag">{{ question.header }}</span><span
              :id="'question-' + session.id + '-' + item.id + '-' + question.id"
              >{{ question.question }}</span
            >
          </legend>
          <div
            v-if="question.options.length && !question.isSecret"
            class="chat-answers"
            :role="question.multiSelect ? 'group' : 'radiogroup'"
            :aria-labelledby="'question-' + session.id + '-' + item.id + '-' + question.id"
          >
            <label
              v-for="option in question.options"
              :key="option.label"
              :class="['chat-answer', { selected: answers.chosen[item.id]?.[question.id]?.includes(option.label) }]"
              ><input
                :type="question.multiSelect ? 'checkbox' : 'radio'"
                :name="'answer-' + session.id + '-' + item.id + '-' + question.id"
                :value="option.label"
                :checked="answers.chosen[item.id]?.[question.id]?.includes(option.label) ?? false"
                @change="
                  question.multiSelect
                    ? answers.toggle(item.id, question.id, option.label, $event)
                    : answers.answer(item.id, question.id, $event)
                "
              /><span class="chat-answer-marker" aria-hidden="true"
                ><Icon v-if="question.multiSelect" name="check" :size="11" /></span
              ><span class="chat-answer-copy"
                ><strong>{{ option.label }}</strong
                ><small v-if="option.description">{{ option.description }}</small></span
              ></label
            ><label
              v-if="question.isOther"
              :class="[
                'chat-answer',
                'chat-answer-other',
                { selected: !!answers.other[item.id]?.[question.id]?.trim() },
              ]"
              ><span class="chat-answer-marker" aria-hidden="true" /><input
                type="text"
                autocomplete="off"
                :placeholder="t('Other answer')"
                :aria-label="t('Other answer')"
                :value="answers.other[item.id]?.[question.id] ?? ''"
                @input="answers.typeOther(item.id, question.id, $event, question.multiSelect)"
            /></label>
          </div>
          <template v-else
            ><input
              class="chat-question-input"
              :aria-labelledby="'question-' + session.id + '-' + item.id + '-' + question.id"
              :type="question.isSecret ? 'password' : 'text'"
              :list="
                !question.isSecret && question.options.length
                  ? 'choices-' + session.id + '-' + item.id + '-' + question.id
                  : undefined
              "
              autocomplete="off"
              :placeholder="t('Type your answer')"
              :value="answers.chosen[item.id]?.[question.id]?.[0] ?? ''"
              @input="answers.answer(item.id, question.id, $event)"
            /><datalist
              v-if="!question.isSecret && question.options.length"
              :id="'choices-' + session.id + '-' + item.id + '-' + question.id"
            >
              <option v-for="option in question.options" :key="option.label" :value="option.label">
                {{ option.description }}
              </option>
            </datalist></template
          >
        </fieldset>
        <footer v-if="approvalChoices(item).length" class="chat-approval-actions">
          <button
            v-for="choice in [...approvalChoices(item)].reverse()"
            :key="choice"
            type="button"
            :class="
              choice === 'accept'
                ? 'allow'
                : choice === 'accept_session'
                  ? 'allow-session'
                  : choice === 'decline'
                    ? 'deny'
                    : 'cancel'
            "
            :disabled="preview || !!busy || !connected || (choice === 'accept' && answers.incomplete(item))"
            @click="emit('approve', item, choice)"
          >
            {{ t(choice === "accept" && item.questions.length ? "Submit answers" : DECISION_LABEL[choice])
            }}<kbd v-if="choice === 'accept' && !mobile" class="chat-approval-key">{{ acceptKey }}</kbd
            ><kbd v-else-if="choice === 'decline' && !mobile" class="chat-approval-key">{{ declineKey }}</kbd>
          </button>
        </footer>
        <p v-else class="chat-approval-note">
          {{ t("This native approval needs the original client. No permission was granted automatically.") }}
        </p>
      </template>
    </article>
  </section>
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
/* The card must not clip here: overflow!=visible would make the card its own
   scroll container and the sticky footer below would stick to the card (which
   never scrolls) instead of the dock. Re-round the accent bar it used to clip. */
.chat-approval-dock .chat-approval {
  flex-shrink: 0;
  margin: 0;
  max-width: none;
  overflow: visible;
  box-shadow: var(--shadow-md), var(--shadow-card);
}
.chat-approval-dock .chat-approval::before {
  border-top-left-radius: var(--radius-lg);
  border-bottom-left-radius: var(--radius-lg);
}
/* Pin the actions to the bottom of the scrolling dock so Submit stays in view
   however short the pane is. Opaque background hides content scrolling under it. */
.chat-approval-dock .chat-approval-actions {
  position: sticky;
  bottom: 0;
  background: var(--surface);
  padding-bottom: var(--space-2);
}
.chat-approval-pager {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 4px;
  font-size: var(--text-xs);
  color: var(--warn-ink);
}
.chat-approval-pager > span {
  flex: 1;
}
.chat-approval-pager .pager-back {
  transform: rotate(180deg);
}
.chat-approval-key {
  margin-left: 8px;
  padding: 0 5px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--on-accent) 18%, transparent);
  font: var(--text-xs) var(--font-body);
}
.chat-pane.is-phone .chat-approval-dock {
  margin: 0 10px 8px;
  max-height: 60vh;
}
.chat-approval-scope {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  margin-top: var(--space-2);
}
.scope-access {
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--fill);
  color: var(--ink-soft);
  font-size: var(--text-xs);
  font-weight: 600;
}
.scope-access.write,
.scope-access.execute {
  background: var(--warn-soft);
  color: var(--warn-ink);
}
.scope-access.network {
  background: var(--info-soft);
  color: var(--info-ink);
}
.scope-outside {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--danger-ink);
  font-size: var(--text-xs);
  font-weight: 600;
}
.chat-approval-body .scope-path {
  padding: 1px 6px;
  border-radius: var(--radius-xs);
  background: var(--danger-soft);
  color: var(--danger-ink);
  font-family: var(--mono);
  font-size: var(--text-xs);
  overflow-wrap: anywhere;
}
.scope-command {
  flex-basis: 100%;
  margin: 2px 0 0;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  background: var(--sunken);
  border: 1px solid var(--border);
  font: var(--text-xs)/1.6 var(--mono);
  color: var(--ink);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 160px;
  overflow: auto;
}
.chat-approval-detail {
  margin-top: var(--space-2);
  font-size: var(--text-xs);
  color: var(--muted);
}
.chat-approval-detail summary {
  cursor: pointer;
}
.chat-approval-detail pre {
  margin: 6px 0 0;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  background: var(--sunken);
  font: var(--text-xs)/1.6 var(--mono);
  color: var(--ink-soft);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 200px;
  overflow: auto;
}
.chat-approval-actions button.allow-session {
  border-color: var(--accent);
  color: var(--accent-ink);
  background: var(--surface);
}
@media (hover: hover) {
  .chat-approval-actions button.allow-session:hover:not(:disabled) {
    background: var(--accent-soft);
  }
}
.chat-pane.is-phone .chat-approval-actions button.allow-session {
  grid-column: 1/-1;
  order: -1;
}
/* Approval and question cards: a calm surface with one coloured voice (amber
   for a permission, teal for a question) on the icon and the left edge, so the
   request reads as part of the conversation rather than a warning banner. */
.chat-approval {
  --approval-tone: var(--warn);
  --approval-tone-soft: var(--warn-soft);
  --approval-tone-ink: var(--warn-ink);
  position: relative;
  max-width: 48rem;
  margin: var(--space-4) auto;
  padding: var(--space-4) var(--space-4) var(--space-4) calc(var(--space-4) + 3px);
  background: var(--surface);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-card);
  font-size: var(--text-md);
  color: var(--ink);
  overflow-wrap: anywhere;
  overflow: hidden;
}
.chat-approval::before {
  content: "";
  position: absolute;
  inset: 0 auto 0 0;
  width: 3px;
  background: var(--approval-tone);
}
.chat-approval.is-question {
  --approval-tone: var(--accent);
  --approval-tone-soft: var(--accent-soft);
  --approval-tone-ink: var(--accent-ink);
}
.chat-approval.resolved {
  --approval-tone: var(--line-strong);
  --approval-tone-soft: var(--fill);
  --approval-tone-ink: var(--muted);
  padding-block: var(--space-3);
  box-shadow: none;
  border: 1px solid var(--border);
  background: var(--sunken);
}
.chat-approval.resolved .chat-approval-body {
  display: none;
}
.chat-approval.folded {
  padding-block: var(--space-3);
}
.chat-approval.folded .chat-approval-body,
.chat-approval.folded .chat-question,
.chat-approval.folded .chat-approval-actions,
.chat-approval.folded .chat-approval-note {
  display: none;
}
.chat-approval.folded .chat-approval-heading strong {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: var(--text-md);
}
.chat-approval-fold {
  flex-shrink: 0;
  color: var(--muted);
}
.chat-approval-fold :deep(svg) {
  transform: rotate(90deg);
  transition: transform var(--duration-fast) var(--ease-out);
}
.chat-approval.folded .chat-approval-fold :deep(svg) {
  transform: rotate(-90deg);
}
.chat-approval-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.chat-approval-icon {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  border-radius: var(--radius-md);
  background: var(--approval-tone-soft);
  color: var(--approval-tone-ink);
}
.chat-approval-heading {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.chat-approval-heading small {
  font-size: var(--text-xs);
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--approval-tone-ink);
}
.chat-approval-heading strong {
  font-size: var(--text-lg);
  font-weight: 600;
  line-height: 1.35;
  color: var(--ink);
}
.chat-approval.resolved .chat-approval-heading strong {
  font-size: var(--text-md);
  font-weight: 500;
  color: var(--ink-soft);
}
.chat-approval-state {
  flex-shrink: 0;
  border-radius: var(--radius-full);
  background: var(--fill);
  color: var(--muted);
  font-size: var(--text-xs);
  padding: 2px var(--space-2);
}
.chat-approval-body {
  margin: var(--space-3) 0 0 calc(30px + var(--space-3));
  color: var(--ink-soft);
  line-height: 1.7;
}
.chat-approval-body :deep(p) {
  margin: 0 0 var(--space-2);
}
.chat-approval-body :deep(p:last-child) {
  margin-bottom: 0;
}
.chat-approval-body :deep(code) {
  font-family: var(--mono);
  font-size: 0.92em;
  background: var(--fill);
  border-radius: var(--radius-sm);
  padding: 1px 5px;
  color: var(--ink);
}
.chat-approval-body :deep(pre) {
  background: var(--sunken);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--space-3);
  margin: var(--space-2) 0;
  overflow: auto;
}
.chat-question {
  border: 0;
  margin: var(--space-4) 0 0 calc(30px + var(--space-3));
  padding: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.chat-question legend {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--space-2);
  padding: 0;
  margin-bottom: var(--space-2);
  font-size: var(--text-md);
  font-weight: 600;
  line-height: 1.5;
  color: var(--ink);
}
.chat-question-tag {
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--accent-ink);
  background: var(--accent-soft);
  border-radius: var(--radius-full);
  padding: 1px var(--space-2);
}
.chat-answers {
  display: grid;
  gap: 6px;
}
.chat-answer {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 44px;
  padding: 9px var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
  cursor: pointer;
  transition:
    border-color var(--duration-fast),
    background var(--duration-fast);
}
.chat-answer > input[type="radio"],
.chat-answer > input[type="checkbox"] {
  position: absolute;
  opacity: 0;
  width: 1px;
  height: 1px;
  pointer-events: none;
}
@media (hover: hover) {
  .chat-answer:hover {
    background: var(--sunken);
    border-color: var(--line-strong);
  }
}
.chat-answer:has(input:focus-visible) {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.chat-answer.selected {
  border-color: var(--accent);
  background: var(--accent-soft);
}
.chat-question:disabled .chat-answer {
  cursor: default;
  opacity: 0.7;
}
.chat-answer-marker {
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  flex-shrink: 0;
  border: 1.5px solid var(--line-strong);
  border-radius: var(--radius-full);
  color: var(--on-accent);
  background: var(--surface);
  transition:
    border-color var(--duration-fast),
    background var(--duration-fast),
    box-shadow var(--duration-fast);
}
.chat-answer:has(input[type="checkbox"]) .chat-answer-marker {
  border-radius: var(--radius-xs);
}
.chat-answer:has(input[type="checkbox"]) .chat-answer-marker svg {
  opacity: 0;
}
.chat-answer.selected .chat-answer-marker {
  border-color: var(--accent);
  background: var(--accent);
}
.chat-answer.selected:has(input[type="radio"]) .chat-answer-marker {
  background: var(--surface);
  box-shadow:
    inset 0 0 0 3px var(--surface),
    inset 0 0 0 8px var(--accent);
}
.chat-answer.selected:has(input[type="checkbox"]) .chat-answer-marker svg {
  opacity: 1;
}
.chat-answer-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.chat-answer-copy strong {
  font-size: var(--text-md);
  font-weight: 500;
  line-height: 1.4;
  color: var(--ink);
}
.chat-answer-copy small {
  font-size: var(--text-sm);
  line-height: 1.5;
  color: var(--ink-soft);
}
.chat-answer-other {
  cursor: text;
}
.chat-answer-other .chat-answer-marker {
  border-style: dashed;
}
.chat-answer-other.selected .chat-answer-marker {
  border-style: solid;
  background: var(--surface);
  box-shadow:
    inset 0 0 0 3px var(--surface),
    inset 0 0 0 8px var(--accent);
}
.chat-answer-other input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: 0;
  background: transparent;
  padding: 0;
  font-size: var(--text-md);
  color: var(--ink);
}
.chat-answer-other input::placeholder {
  color: var(--muted);
}
/* The card shows the focus; a second ring on the field inside it doubles it. */
.chat-answer.chat-answer-other input:focus-visible {
  outline: 0;
}
.chat-question-input {
  width: 100%;
  min-width: 0;
  min-height: 44px;
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
  padding: 9px var(--space-3);
  background: var(--surface);
  font-size: var(--text-md);
  color: var(--ink);
}
.chat-question-input:focus {
  border-color: var(--accent);
  outline: 0;
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.chat-approval-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-4);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}
.chat-approval-actions button {
  min-height: 36px;
  padding: 7px 14px;
  border-radius: var(--radius-md);
  border: 1px solid var(--line);
  background: var(--surface);
  color: var(--ink-soft);
  font-size: var(--text-sm);
  font-weight: 550;
  transition:
    background var(--duration-fast),
    border-color var(--duration-fast),
    color var(--duration-fast);
}
.chat-approval-actions button.cancel {
  border-color: transparent;
  background: transparent;
  color: var(--muted);
}
.chat-approval-actions button.deny {
  color: var(--danger-ink);
}
.chat-approval-actions button.allow {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--on-accent);
  box-shadow: var(--shadow-sm);
}
@media (hover: hover) {
  .chat-approval-actions button.cancel:hover:not(:disabled) {
    background: var(--fill);
    color: var(--ink-soft);
  }
  .chat-approval-actions button.deny:hover:not(:disabled) {
    background: var(--danger-soft);
    border-color: var(--danger-line);
  }
  .chat-approval-actions button.allow:hover:not(:disabled) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }
}
.chat-approval-note {
  margin: var(--space-3) 0 0 calc(30px + var(--space-3));
  font-size: var(--text-sm);
  color: var(--muted);
}
.chat-pane.is-phone .chat-approval {
  padding: var(--space-3) var(--space-3) var(--space-3) calc(var(--space-3) + 3px);
}
.chat-pane.is-phone .chat-approval-body,
.chat-pane.is-phone .chat-question,
.chat-pane.is-phone .chat-approval-note {
  margin-left: 0;
}
.chat-pane.is-phone .chat-approval-actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
}
.chat-pane.is-phone .chat-approval-actions button {
  min-height: 44px;
}
.chat-pane.is-phone .chat-answer-other input,
.chat-pane.is-phone .chat-question-input {
  font-size: var(--input-text);
}
.chat-pane.is-phone .chat-approval-actions button.allow {
  grid-column: 1/-1;
  order: -1;
}
</style>
