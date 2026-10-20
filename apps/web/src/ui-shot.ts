// A throwaway entry for taking screenshots of the model panel. Fixtures only:
// the "check" below is a local function, so no request ever leaves the page.
import { createApp, defineComponent, h, ref } from "vue";
import "@fontsource-variable/instrument-sans";
import "@fontsource-variable/jetbrains-mono";
import "./features/theme";
import "./features/appearance";
import ModelSelection from "./features/ModelSelection.vue";
import type { ModelProbeApi, ModelProbeResult } from "./features/model-probe";
import "./styles.css";

const catalog = [
  { id: "claude-sonnet-4-6", name: "Claude Sonnet 4.6" },
  { id: "claude-opus-4-6", name: "Claude Opus 4.6" },
  { id: "claude-haiku-4-5", name: "Claude Haiku 4.5" },
  { id: "gpt-5.2-codex", name: "GPT-5.2 Codex" },
  { id: "gpt-5.2", name: "GPT-5.2" },
  { id: "o4-mini", name: "o4-mini" },
  { id: "gemini-3-pro", name: "Gemini 3 Pro" },
  { id: "deepseek-v3.2", name: "DeepSeek V3.2" },
  { id: "qwen3-max", name: "Qwen3 Max" },
  { id: "llama-4-405b", name: "Llama 4 405B" },
];
// What a real endpoint answers: most work, some are listed but missing, one
// is rate limited, one times out — the mixed case the panel has to read well.
const API: ModelProbeApi = (new URLSearchParams(location.search).get("api") as ModelProbeApi) || "anthropic-messages";
const ALL_404 = new URLSearchParams(location.search).has("all404");
const answers: Record<string, Omit<ModelProbeResult, "model" | "api">> = {
  "claude-sonnet-4-6": { status: "ok", latency_ms: 820 },
  "claude-opus-4-6": { status: "ok", latency_ms: 1640 },
  "claude-haiku-4-5": { status: "ok", latency_ms: 410 },
  "gpt-5.2-codex": { status: "ok", latency_ms: 1180 },
  "gpt-5.2": { status: "ok", latency_ms: 960 },
  "o4-mini": { status: "not_found", detail: "The endpoint has no such model" },
  "gemini-3-pro": { status: "rate_limited", detail: "Rate limited; try again shortly" },
  "deepseek-v3.2": { status: "timeout", detail: "The endpoint did not answer in time" },
  "qwen3-max": { status: "failed", detail: "The endpoint rejected the check" },
};
const mode = new URLSearchParams(location.search).get("mode") ?? "done";

const Harness = defineComponent(() => {
  const selected = ref<string[]>(mode === "chosen" ? ["claude-sonnet-4-6", "claude-opus-4-6"] : []);
  const probe = async (models: string[]) =>
    models.map(model => ({ model, api: API, ...(ALL_404 ? { status: "not_found" as const } : answers[model] ?? { status: "ok" as const, latency_ms: 700 }) }));
  const plan = {
    client: "Pi",
    api: API,
    path: API === "google-generative-ai" ? "/v1beta/models/model:generateContent" : API === "anthropic-messages" ? "/v1/messages" : API === "openai-responses" ? "/v1/responses" : "/v1/chat/completions",
    guessed: new URLSearchParams(location.search).has("guessed"),
  };
  const panel = () => h(ModelSelection, {
    catalog,
    selected: selected.value,
    probe,
    plan,
    windowText: (model: string) => (model === "qwen3-max" ? "262144" : ""),
    placeholder: (model: string) => (model.startsWith("claude") ? "200000" : "128000"),
    // What an endpoint says, what is published, and nothing at all.
    knownWindow: (model: string) => model.startsWith("claude") ? { tokens: 200_000, source: "endpoint" as const }
      : model.startsWith("gpt") ? { tokens: 400_000, source: "published" as const } : undefined,
    "onUpdate:selected": (value: string[]) => { selected.value = value; },
    onWindow: () => {},
    onLook: () => {},
  });
  return () => h("div", { class: "shot-frame" }, [h("h1", { class: "shot-title" }, "端点模型"), panel()]);
});

createApp(Harness).mount("#root");
