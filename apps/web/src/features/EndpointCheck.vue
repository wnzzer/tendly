<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from "vue";
import { useI18n } from "../i18n";
import { PROBE_APIS, chooseApi, probeAnswer, type ModelProbeApi, type ModelProbePlan, type ModelProbeResult } from "./model-probe";

/**
 * The form's one check: does this endpoint run this client at all, and over
 * which protocol.
 *
 * Claude Code and Codex each speak a single protocol, so there is nothing to
 * choose: the default model is tried over it, and then the chosen models.
 * Pi can speak several, and a wrong guess makes every model fail at once, so
 * the default model is tried over each first, the protocol that worked is
 * written into the profile, and only then are the chosen models checked over
 * it. One press, one loop: nothing is left for the person to work out from a
 * list of HTTP codes.
 */
const props = defineProps<{
  client: string;
  /** The protocols this client can be told to use; empty when it has one of its own. */
  apis: readonly ModelProbeApi[];
  /** The one it speaks when it has no choice. */
  fixedApi?: ModelProbeApi;
  /** The protocol the profile names now, if any. */
  named?: ModelProbeApi;
  plan?: ModelProbePlan;
  /** The model to try: the default one, aliases resolved. */
  model: string;
  /** Whether a model list is there for the follow-up check to run on. */
  batch: boolean;
  /** Why the form cannot be checked as it stands, if it cannot. */
  blocked?: string;
  probe: (models: string[], api?: ModelProbeApi) => Promise<ModelProbeResult[]>;
}>();
const emit = defineEmits<{ "use-api": [api: ModelProbeApi]; checked: [] }>();
const { t } = useI18n();

const running = ref(false);
const answers = ref<ModelProbeResult[]>([]);
const outcome = ref<{ tone: "ok" | "warn"; text: string } | undefined>();
const error = ref("");
let alive = true;
onBeforeUnmount(() => { alive = false; });

const choosing = computed(() => props.apis.length > 0);
const tried = computed(() => choosing.value ? props.apis : props.fixedApi ? [props.fixedApi] : []);
const help = computed(() => choosing.value
  ? t("Tries the default model over each of the {count} protocols {client} can speak, keeps the one that works, then checks the chosen models over it. Each check is one very short request, billed by the endpoint.", { count: props.apis.length, client: props.client })
  : t("{client} speaks only {api}. Checks the default model, then the chosen models. Each check is one very short request, billed by the endpoint.", { client: props.client, api: props.fixedApi ? PROBE_APIS[props.fixedApi].label : "" }));

async function run() {
  if (running.value || !props.model || props.blocked) return;
  running.value = true; answers.value = []; outcome.value = undefined; error.value = "";
  try {
    // A client with a choice is asked once per protocol, at the same time: four
    // short requests, and the answers come back side by side.
    const results = (await Promise.all(tried.value.map(api => props.probe([props.model], choosing.value ? api : undefined)))).flat();
    if (!alive) return;
    answers.value = results;
    let api = props.fixedApi;
    if (choosing.value) {
      api = chooseApi(results, props.apis, props.named, props.plan?.guessed ? props.plan.api : undefined);
      if (!api) { outcome.value = { tone: "warn", text: t("No protocol ran {model}. Check the key, the endpoint URL and the model before anything else.", { model: props.model }) }; return; }
      if (api !== props.named) { emit("use-api", api); await nextTick(); }
    } else if (results[0]?.status !== "ok") {
      // The chosen models would go the same way; say so once rather than per row.
      outcome.value = { tone: "warn", text: t("{model} did not run over {api}, so the chosen models were not checked.", { model: props.model, api: api ? PROBE_APIS[api].label : "" }) };
      return;
    }
    if (!alive) return;
    const using = api ? PROBE_APIS[api].label : "";
    outcome.value = { tone: "ok", text: props.batch
      ? t("Using {api}. Checking the chosen models over it below.", { api: using })
      : t("Using {api}. Load the endpoint's models to check the rest.", { api: using }) };
    emit("checked");
  } catch (cause) {
    if (alive) error.value = cause instanceof Error ? cause.message : String(cause);
  } finally {
    if (alive) running.value = false;
  }
}
</script>

<template>
  <div class="endpoint-check">
    <button type="button" class="small-button" :disabled="running || !model || !!blocked" :title="blocked" @click="run">
      {{ t(running ? 'Checking…' : choosing ? 'Check protocols and models' : 'Check models') }}
    </button>
    <p class="form-help">{{ help }}</p>
    <p v-if="!model" class="form-help">{{ t('Choose a default model to check.') }}</p>
    <ul v-if="answers.length" aria-live="polite">
      <li v-for="answer in answers" :key="answer.api">
        <span class="endpoint-check-api">{{ PROBE_APIS[answer.api].label }}</span>
        <span :class="['endpoint-check-pill', probeAnswer(answer, t).kind]" :title="probeAnswer(answer, t).title">{{ probeAnswer(answer, t).text }}</span>
        <template v-if="choosing">
          <small v-if="answer.api === named" class="endpoint-check-used">{{ t('in use') }}</small>
          <button v-else-if="answer.status === 'ok'" type="button" class="text-button" :disabled="running" @click="emit('use-api', answer.api)">{{ t('Use this') }}</button>
        </template>
      </li>
    </ul>
    <p v-if="outcome" :class="['form-help', outcome.tone]">{{ outcome.text }}</p>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>
  </div>
</template>

<style scoped>
.endpoint-check{padding:12px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken)}
.endpoint-check ul{margin:8px 0;padding:0;list-style:none;display:grid;gap:6px}
.endpoint-check li{display:flex;align-items:center;flex-wrap:wrap;gap:6px 8px;font-size:var(--text-sm);color:var(--ink-soft)}
.endpoint-check-api{min-width:0;overflow-wrap:anywhere}
.endpoint-check-pill{flex-shrink:0;display:inline-flex;align-items:center;padding:2px 8px;border:1px solid transparent;border-radius:var(--radius-full);font-size:var(--text-xs);font-variant-numeric:tabular-nums;white-space:nowrap}
.endpoint-check-pill.ok{border-color:var(--ok-line);background:var(--ok-soft);color:var(--ok-ink)}
.endpoint-check-pill.warn{border-color:var(--warn-line);background:var(--warn-soft);color:var(--warn-ink)}
.endpoint-check-pill.broken{border-color:var(--danger-line);background:var(--danger-soft);color:var(--danger-ink)}
.endpoint-check-used{color:var(--muted);font-size:var(--text-xs)}
.endpoint-check .form-help.ok{color:var(--ok-ink)}
.endpoint-check .form-help.warn{color:var(--warn-ink)}
</style>
