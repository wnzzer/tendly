<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { useI18n } from "../i18n";
import { filterModels } from "./model-choices";
import { validWindow } from "./model-limits";
import {
  checkTally, dropBroken, formatLatency, isBroken, isInconclusive, routeSuspicion,
  PROBE_APIS, PROBE_RUN, apisUsed, keepWorking, matchesCheckFilter, mergeChecks, needsCheck, probeBatches, probeRun,
  type CheckFilter, type ModelProbeApi, type ModelProbePlan, type ModelProbeResult,
} from "./model-probe";
import Icon from "./Icon.vue";

/**
 * Which of an endpoint's models a profile offers, and per model the advanced
 * settings (its context window) — plus the one place a model is *tried*.
 *
 * Discovery reads a list; it cannot say which rows an endpoint will actually
 * run, because gateways list models they do not serve and refuse models they
 * do. So the panel also offers a check: one request per model, asked only when
 * a person presses the button, and the answer decides what stays. Checking,
 * marking and pruning happen here, in the form, so saving is the last step
 * rather than the first — the loop closes in the same place it opened.
 *
 * Choosing none offers every model the endpoint serves, which is why pruning
 * materializes an explicit list: "these ones work" cannot be said with an
 * empty selection.
 */
const props = defineProps<{
  catalog: readonly { id: string; name: string }[];
  selected: readonly string[];
  windowText: (model: string) => string;
  placeholder: (model: string) => string;
  /** Ask the endpoint which of these models run; absent on an older server. */
  probe?: (models: string[]) => Promise<ModelProbeResult[]>;
  /** Which protocol a check would speak, already resolved by the form (it
   * owns the fields that decide it); absent on an older server or before the
   * first answer, and the panel then simply says nothing about it. */
  plan?: ModelProbePlan;
}>();
const emit = defineEmits<{
  "update:selected": [models: string[]];
  window: [model: string, text: string];
  look: [model: string];
}>();
const { t } = useI18n();

const query = ref("");
const filter = ref<CheckFilter>("all");
const open = reactive(new Set<string>());
const checks = ref<Record<string, ModelProbeResult>>({});
const checking = ref(new Set<string>());
const running = ref(false);
const stopRequested = ref(false);
const checkError = ref("");
/** Models a press had to leave for the next one, because a run is capped. */
const leftover = ref(0);
/** How many of the models asked for have an answer, for the progress bar. */
const progress = reactive({ done: 0, total: 0 });

const chosen = computed(() => new Set(props.selected));
const rows = computed(() => {
  const listed = new Set(props.catalog.map(entry => entry.id));
  return [...props.selected.filter(id => !listed.has(id)).map(id => ({ id, name: id })), ...props.catalog];
});
const matched = computed(() => filterModels(rows.value, query.value));
const shown = computed(() => matched.value.filter(entry => matchesCheckFilter(entry.id, filter.value, checks.value, chosen.value)));
const tally = computed(() => checkTally(checks.value));
const canCheck = computed(() => !!props.probe);
const busy = computed(() => running.value);

const counts = computed(() => ({
  all: matched.value.length,
  chosen: matched.value.filter(entry => chosen.value.has(entry.id)).length,
  ok: matched.value.filter(entry => checks.value[entry.id]?.status === "ok").length,
  broken: matched.value.filter(entry => isBroken(checks.value[entry.id]?.status)).length,
  inconclusive: matched.value.filter(entry => isInconclusive(checks.value[entry.id]?.status)).length,
  untested: matched.value.filter(entry => !checks.value[entry.id]).length,
}));
const filters = computed(() => ([
  { id: "all", label: t("All models") },
  { id: "chosen", label: t("Chosen") },
  { id: "ok", label: t("Working") },
  { id: "broken", label: t("Failed") },
  { id: "inconclusive", label: t("Unclear") },
  { id: "untested", label: t("Untested") },
] as const).filter(entry => entry.id === "all" || counts.value[entry.id] > 0));
/** Rows on screen that a check would still say something about: no answer, or
 * an answer that only described the moment (a timeout, a rate limit). */
const recheckable = computed(() => shown.value.filter(entry => needsCheck(checks.value[entry.id]?.status)).length);
const recheckableChosen = computed(() => props.selected.filter(id => needsCheck(checks.value[id]?.status)).length);
/**
 * What "keep working only" keeps: the chosen models if anything is chosen,
 * and otherwise every model that answered — so checking a fresh list and
 * pruning it to the working ones is one click, not two, and the result is the
 * explicit list that "these models work" needs.
 */
const keepScope = computed(() => props.selected.length ? props.selected : rows.value.map(entry => entry.id));
const keepable = computed(() => keepWorking(keepScope.value, checks.value));
/** The rows "remove failed" applies to: the chosen models if any are chosen,
 * and otherwise every listed model — because the point is to keep offering
 * everything except the models the endpoint refused. */
const pruneScope = computed(() => props.selected.length ? props.selected : rows.value.map(entry => entry.id));
const prunable = computed(() => pruneScope.value.filter(id => isBroken(checks.value[id]?.status)).length);

/** The protocols the answers on screen used, for the summary line. */
const protocols = computed(() => apisUsed(Object.values(checks.value)));
/** Which protocol a check would speak now, said before anything is billed.
 * A guessed API is named as a guess: on Pi it decides the request, so it is
 * worth knowing before the run rather than after a batch of failures. */
const planned = computed(() => props.plan);

/** What a row's pill says: no answer yet, being asked, or what came back. */
function statusOf(id: string): { kind: string; text: string; title?: string } {
  if (checking.value.has(id)) return { kind: "checking", text: t("Checking…") };
  const result = checks.value[id];
  if (!result) return { kind: "untested", text: t("Untested") };
  const detail = result.detail ? { title: result.detail } : {};
  const latency = formatLatency(result.latency_ms);
  switch (result.status) {
    case "ok": return { kind: "ok", text: latency ? t("Works · {latency}", { latency }) : t("Works"), ...detail };
    case "not_found": return { kind: "broken", text: t("Model not found"), ...detail };
    case "unauthorized": return { kind: "warn", text: t("Key refused"), ...detail };
    case "rate_limited": return { kind: "warn", text: t("Rate limited"), ...detail };
    case "timeout": return { kind: "warn", text: t("Timed out"), ...detail };
    default: return { kind: "broken", text: t("Failed"), ...detail };
  }
}

function toggle(id: string) {
  emit("update:selected", chosen.value.has(id) ? props.selected.filter(model => model !== id) : [...props.selected, id]);
}
function chooseShown() { emit("update:selected", [...new Set([...props.selected, ...shown.value.map(entry => entry.id)])]); }
function clearShown() { const hidden = new Set(shown.value.map(entry => entry.id)); emit("update:selected", props.selected.filter(id => !hidden.has(id))); }
function toggleAdvanced(id: string) { if (!open.delete(id)) { open.add(id); emit("look", id); } }

/**
 * Check models in batches so the list fills in as answers arrive and stopping
 * means stopping at the next batch. A failure to reach the server ends the
 * run with one message; per-model answers are already on their rows.
 */
async function run(models: readonly string[], again = false) {
  const probe = props.probe;
  if (!probe || running.value) return;
  const wanted = probeRun(models, checks.value, PROBE_RUN, again);
  const batches = probeBatches(wanted.models);
  if (!batches.length) return;
  running.value = true; stopRequested.value = false; checkError.value = ""; leftover.value = wanted.skipped;
  progress.done = 0; progress.total = batches.reduce((sum, batch) => sum + batch.length, 0);
  try {
    for (const batch of batches) {
      if (stopRequested.value) break;
      checking.value = new Set(batch);
      try {
        const results = await probe(batch);
        // A stop that landed while this batch was in flight still keeps its
        // answers: they are already paid for, and dropping them would leave
        // rows saying nothing about a check that ran.
        checks.value = mergeChecks(checks.value, results);
        progress.done += batch.length;
      } catch (cause) {
        checkError.value = cause instanceof Error ? cause.message : String(cause);
        break;
      } finally {
        checking.value = new Set();
      }
    }
  } finally {
    running.value = false; stopRequested.value = false; checking.value = new Set();
  }
}

/** The models to check: what the filter is showing, so "check these" means
 * the rows on screen rather than a hidden list. A row that already answered
 * is skipped — a working model is not asked again, and a failed one keeps its
 * answer until someone checks it deliberately — but a timeout or a rate limit
 * is retried, because it described the moment rather than the model. */
function checkShown() { void run(shown.value.filter(entry => needsCheck(checks.value[entry.id]?.status)).map(entry => entry.id)); }
function checkChosen() { void run(props.selected.filter(id => needsCheck(checks.value[id]?.status))); }
/** One row, asked again on purpose: a failure can be a bad minute, and the
 * only way to learn otherwise is to ask once more. */
function checkOne(id: string) { void run([id], true); }
function stop() { stopRequested.value = true; }

function keepWorkingOnly() {
  if (keepable.value.length) emit("update:selected", keepable.value);
}
/** Pruning everything would emit an empty list, and empty means "offer all" —
 * the exact opposite. Refuse and say why instead of quietly undoing the work. */
function removeBroken() {
  const kept = dropBroken(pruneScope.value, checks.value);
  if (!kept.length) { checkError.value = t("Every model here failed. Change the key or the endpoint rather than pruning them all."); return; }
  checkError.value = "";
  emit("update:selected", kept);
}

/** Every model was 404: the route is the likelier culprit, not the catalogue. */
const routeSuspect = computed(() => !running.value && routeSuspicion(Object.values(checks.value)));

/** Every answer was "the key was refused": one problem, not many, and nothing
 * here can be concluded about the models themselves. */
const keyRefused = computed(() => {
  const answers = Object.values(checks.value);
  return !running.value && answers.length > 0 && answers.every(result => result.status === "unauthorized");
});

// A catalog that changes (a different endpoint, a re-discovery) describes
// different models; answers about the old ones would mislead. The IDs, not the
// array, are watched: the form rebuilds the array on every render.
watch(() => props.catalog.map(entry => entry.id).join("\n"), () => { checks.value = {}; checkError.value = ""; leftover.value = 0; });
// A chip whose count fell to zero is gone; showing its empty result set would
// read as "no models" rather than "nothing failed".
watch(filters, entries => { if (!entries.some(entry => entry.id === filter.value)) filter.value = "all"; });
</script>

<template>
  <section class="model-panel">
    <header class="model-panel-head">
      <div class="model-panel-title">
        <strong>{{ t('Model catalog') }}</strong>
        <span class="model-panel-summary">
          <template v-if="tally.tested">
            <template v-if="tally.inconclusive">
              {{ t('{working} working · {broken} failed · {unclear} unclear · {count} models', { working: tally.ok, broken: tally.broken, unclear: tally.inconclusive, count: rows.length }) }}
            </template>
            <template v-else>
              {{ t('{working} working · {broken} failed · {count} models', { working: tally.ok, broken: tally.broken, count: rows.length }) }}
            </template>
            <template v-if="protocols.length">
              · {{ t('over {apis}', { apis: protocols.map(api => PROBE_APIS[api].short).join(', ') }) }}
            </template>
          </template>
          <template v-else>{{ t('{chosen} of {count} chosen', { chosen: selected.length, count: rows.length }) }}</template>
        </span>
      </div>
      <div class="model-panel-actions">
        <button v-if="canCheck && !running" type="button" class="small-button" :disabled="!recheckable" :title="t(recheckable ? 'Sends one very short request per model, billed by the endpoint.' : 'Nothing on screen still needs a check.')" @click="checkShown">
          <Icon name="refresh" :size="14" />{{ t('Check shown') }}
        </button>
        <button v-if="canCheck && running" type="button" class="small-button" @click="stop">{{ t('Stop') }}</button>
        <button v-if="canCheck && !running && selected.length" type="button" class="small-button" :disabled="!recheckableChosen" :title="t(recheckableChosen ? 'Sends one very short request per chosen model.' : 'Every chosen model has an answer already.')" @click="checkChosen">{{ t('Check chosen') }}</button>
      </div>
    </header>

    <div v-if="running" class="model-progress" role="progressbar" :aria-valuenow="progress.done" :aria-valuemin="0" :aria-valuemax="progress.total" :aria-valuetext="t('Checking {done} of {total}…', { done: progress.done, total: progress.total })" :aria-label="t('Checking models')">
      <div class="model-progress-bar" :style="{ width: `${progress.total ? Math.round((progress.done / progress.total) * 100) : 0}%` }" />
    </div>
    <p v-if="canCheck && (running || tally.tested)" class="model-panel-note">
      {{ running ? t('Checking {done} of {total}…', { done: progress.done, total: progress.total }) : t('Each check is one minimal request per model and is billed by the endpoint. Nothing is sent unless you press check.') }}
    </p>
    <p v-else-if="canCheck" class="model-panel-note">{{ t('Some endpoints list models they cannot run. Check them to find out which ones work here, then keep only those.') }}</p>
    <p v-if="canCheck && planned" class="model-panel-note protocol">
      <span class="model-api strong">{{ PROBE_APIS[planned.api].label }}</span>
      <code>{{ planned.path }}</code>
      <span v-if="planned.guessed" class="model-api-guess">{{ t('guessed from the URL; name the API above to be sure') }}</span>
    </p>
    <p v-if="checkError" class="inline-error" role="alert">{{ checkError }}</p>
    <p v-else-if="leftover && !running" class="model-panel-note">{{ t('Checked {count} models; {rest} are left. Press again to continue.', { count: progress.done, rest: leftover }) }}</p>
    <p v-if="routeSuspect" class="model-panel-note warn">{{ t('Every model answered "not found", which usually means this endpoint does not serve {api} rather than that the models are gone. Check the endpoint and the API before pruning.', { api: protocols.length ? PROBE_APIS[protocols[0]].label : t('this protocol') }) }}</p>
    <p v-if="keyRefused" class="model-panel-note warn">{{ t('The endpoint refused the key, so the models were not really tried. Fix the key before pruning.') }}</p>

    <div class="model-toolbar">
      <input v-model="query" type="search" autocomplete="off" spellcheck="false" :placeholder="t('Filter by name or ID')" :aria-label="t('Filter by name or ID')" />
      <div class="model-filters" role="group" :aria-label="t('Show models')">
        <button v-for="entry in filters" :key="entry.id" type="button" :aria-pressed="filter === entry.id" :class="{ active: filter === entry.id }" @click="filter = entry.id">
          {{ entry.label }}<span class="model-filter-count">{{ counts[entry.id] }}</span>
        </button>
      </div>
    </div>

    <ul class="model-rows">
      <li v-for="entry in shown" :key="entry.id" :class="{ chosen: chosen.has(entry.id), broken: isBroken(checks[entry.id]?.status) }">
        <div class="model-row">
          <label class="model-row-pick">
            <input type="checkbox" :checked="chosen.has(entry.id)" @change="toggle(entry.id)" />
            <span class="model-row-name">{{ entry.name }}</span>
            <code v-if="entry.id.toLowerCase() !== entry.name.toLowerCase()">{{ entry.id }}</code>
          </label>
          <!-- One batch speaks one protocol, so the chip only earns its place
               when the rows on screen disagree; otherwise the header says it once. -->
          <span v-if="protocols.length > 1 && checks[entry.id]?.api" class="model-api" :title="t('Checked over the {api} API', { api: PROBE_APIS[checks[entry.id].api].label })">{{ PROBE_APIS[checks[entry.id].api].short }}</span>
          <span :class="['model-pill', statusOf(entry.id).kind]" :title="statusOf(entry.id).title">{{ statusOf(entry.id).text }}</span>
          <button v-if="canCheck && !running && checks[entry.id] && checks[entry.id].status !== 'ok'" type="button" class="icon-button model-row-retry" :title="t('Check this model again')" :aria-label="t('Check this model again')" @click="checkOne(entry.id)"><Icon name="refresh" :size="13" /></button>
          <button v-if="chosen.has(entry.id)" type="button" class="text-button model-row-more" :aria-expanded="open.has(entry.id)" @click="toggleAdvanced(entry.id)">{{ t(open.has(entry.id) ? 'Hide' : 'Advanced') }}</button>
        </div>
        <div v-if="chosen.has(entry.id) && open.has(entry.id)" class="model-row-advanced">
          <label>{{ t('Context window') }}<input :value="windowText(entry.id)" inputmode="numeric" autocomplete="off" spellcheck="false" :placeholder="placeholder(entry.id)" @input="emit('window', entry.id, ($event.target as HTMLInputElement).value)" /></label>
          <p v-if="!validWindow(windowText(entry.id))" class="inline-error">{{ t('A context window is between 1,000 and 100,000,000 tokens.') }}</p>
        </div>
      </li>
      <li v-if="!shown.length" class="model-rows-empty">{{ t('No model matches.') }}</li>
    </ul>

    <footer class="model-panel-foot">
      <div class="model-prune">
        <button type="button" class="text-button" :disabled="!shown.length || busy" @click="chooseShown">{{ t(query.trim() || filter !== 'all' ? 'Choose matches' : 'Choose all') }}</button>
        <button type="button" class="text-button" :disabled="!shown.some(entry => chosen.has(entry.id)) || busy" @click="clearShown">{{ t(query.trim() || filter !== 'all' ? 'Clear matches' : 'Clear') }}</button>
        <template v-if="tally.tested">
          <span class="model-prune-divider" aria-hidden="true" />
          <button type="button" class="text-button" :disabled="busy || !keepable.length" :title="t('Keeps only the models that answered. Chosen ones if any are chosen, otherwise every listed model that answered.')" @click="keepWorkingOnly">{{ t('Keep working only') }}</button>
          <button type="button" class="text-button danger-text" :disabled="busy || !prunable" :title="t('Removes the models the endpoint refused. Chosen models if any are chosen, otherwise from the whole list; unchecked models stay.')" @click="removeBroken">{{ t('Remove failed') }}</button>
        </template>
      </div>
      <p class="model-panel-hint">{{ selected.length ? t("A session's model menu offers only the chosen models.") : t("None chosen: a session's model menu offers every model the endpoint serves.") }}</p>
    </footer>
  </section>
</template>

<style scoped>
.model-panel{display:flex;flex-direction:column;gap:10px;min-width:0;border:1px solid var(--border);border-radius:var(--radius-xl);background:var(--surface);padding:14px}
.model-panel-head{display:flex;align-items:flex-start;justify-content:space-between;flex-wrap:wrap;gap:8px 12px}
.model-panel-title{display:flex;flex-direction:column;gap:2px;min-width:0}
.model-panel-title strong{font-size:var(--text-md);font-weight:600;color:var(--ink)}
.model-panel-summary{font-size:var(--text-xs);color:var(--muted);font-variant-numeric:tabular-nums}
.model-panel-actions{display:flex;flex-wrap:wrap;gap:6px}
.model-panel-note{margin:0;font-size:var(--text-xs);line-height:1.7;color:var(--muted)}
.model-panel-note.warn{color:var(--warn-ink)}
.model-progress{height:4px;border-radius:var(--radius-full);background:var(--fill-hover);overflow:hidden}
.model-progress-bar{height:100%;border-radius:var(--radius-full);background:var(--accent);transition:width .2s ease}
.model-toolbar{display:flex;align-items:center;flex-wrap:wrap;gap:8px 12px}
.model-toolbar>input{flex:1 1 160px;min-width:0}
.model-filters{display:inline-flex;flex-wrap:wrap;gap:4px}
.model-filters button{display:inline-flex;align-items:center;gap:5px;padding:4px 9px;border:1px solid var(--border);border-radius:var(--radius-full);background:var(--surface);font:inherit;font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer}
.model-filters button:hover{border-color:var(--accent-line);color:var(--ink)}
.model-filters button.active{border-color:var(--accent-line);background:var(--accent-soft);color:var(--accent-ink);font-weight:550}
.model-filter-count{font-size:var(--text-xs);color:var(--muted);font-variant-numeric:tabular-nums}
.model-filters button.active .model-filter-count{color:var(--accent-ink)}
.model-rows{list-style:none;margin:0;padding:2px;max-height:320px;overflow:auto;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken)}
.model-rows li{border-radius:var(--radius-sm)}
.model-rows li.chosen{background:var(--accent-soft)}
.model-rows li.broken .model-row-name,.model-rows li.broken code{color:var(--muted)}
.model-row{display:flex;align-items:center;gap:8px;padding:5px 8px}
.model-row-pick{flex:1;min-width:0;display:flex;flex-direction:row;align-items:baseline;gap:8px;cursor:pointer;font-size:var(--text-sm);color:var(--ink)}
.model-row-pick input{flex-shrink:0;align-self:center;width:15px;height:15px;min-height:0;margin:0;padding:0}
.model-row-name{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.model-row-pick code{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-family:var(--mono);font-size:var(--text-xs);color:var(--muted)}
.model-panel-note.protocol{display:flex;align-items:center;flex-wrap:wrap;gap:4px 8px}
.model-panel-note.protocol code{font-family:var(--mono);font-size:var(--text-xs);color:var(--ink-soft);background:var(--sunken);border:1px solid var(--border);border-radius:var(--radius-sm);padding:1px 6px}
.model-api-guess{color:var(--warn-ink)}
.model-api{flex-shrink:0;padding:1px 6px;border:1px solid var(--border);border-radius:var(--radius-sm);font-size:var(--text-xs);color:var(--muted);font-variant-numeric:tabular-nums}
.model-api.strong{color:var(--ink-soft);font-weight:550;border-color:var(--accent-line);background:var(--accent-soft)}
.model-pill{flex-shrink:0;display:inline-flex;align-items:center;padding:2px 8px;border:1px solid transparent;border-radius:var(--radius-full);font-size:var(--text-xs);font-variant-numeric:tabular-nums;white-space:nowrap}
.model-pill.untested{border-color:var(--border);color:var(--muted)}
.model-pill.checking{border-color:var(--info-line);background:var(--info-soft);color:var(--info-ink)}
.model-pill.ok{border-color:var(--ok-line);background:var(--ok-soft);color:var(--ok-ink)}
.model-pill.warn{border-color:var(--warn-line);background:var(--warn-soft);color:var(--warn-ink)}
.model-pill.broken{border-color:var(--danger-line);background:var(--danger-soft);color:var(--danger-ink)}
.model-row-more{flex-shrink:0}
.model-row-retry{width:22px;height:22px;padding:2px}

.model-row-advanced{padding:2px 8px 8px 32px;display:flex;flex-direction:column;gap:4px}
.model-row-advanced label{display:flex;flex-direction:column;gap:4px;font-size:var(--text-xs);color:var(--muted)}
.model-rows-empty{padding:10px;font-size:var(--text-sm);color:var(--muted)}
.model-panel-foot{display:flex;flex-direction:column;gap:6px}
.model-prune{display:flex;align-items:center;flex-wrap:wrap;gap:4px 12px}
.model-prune-divider{width:1px;height:12px;background:var(--border)}
.model-panel-hint{margin:0;font-size:var(--text-xs);line-height:1.7;color:var(--muted)}
@media(max-width:520px){.model-panel{padding:10px}.model-row-advanced{padding-left:24px}}
/* Narrow screens: the status pill keeps its place at the right and the ID goes
   under the name, so neither the name nor the pill has to be cut short. */
@media(max-width:520px){
  .model-row-pick{flex-wrap:wrap;gap:2px 8px;align-items:center}
  .model-row-pick input{order:0}
  .model-row-name{order:1;flex:1 1 auto;min-width:0}
  .model-row-pick code{order:2;flex:1 0 100%;padding-left:23px;white-space:normal;overflow-wrap:anywhere}
}
</style>
