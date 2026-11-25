import type { AgentProviderKind, ProviderKind } from "@tendly/protocol";
// Type only: icon-palette.ts reads this registry, so a value import would be circular.
import type { ICON_PALETTE } from "./icon-palette";

/**
 * Everything the interface knows about one agent client, in one place, so a
 * new client is one entry here rather than an edit to every menu, label, icon
 * and branch that names one. The server decides what a client can do; these
 * only shape how the web app offers and explains it.
 */
export interface ClientInfo {
  /** The client's own name, never translated. */
  label: string;
  /** Where room is short, as on the preview's phone tabs. */
  shortLabel: string;
  /** Its mark on a 24×24 grid. Source: @lobehub/icons-static-svg 1.95.0, MIT; see docs/third-party-notices.md. */
  glyph: { path: string; evenOdd?: boolean };
  /** Its colour in icons and tabs, from the light palette, and the tint behind its tab. */
  iconTone: keyof typeof ICON_PALETTE;
  tabTint: string;
  /** The theme token in styles.css that retints its marks: `--{token}` and `--{token}-soft`. */
  colorToken: string;
  /** The permission modes a session can switch between, in menu order. */
  permissionModes: readonly string[];
  /** Whether it asks before running tools at all; one that never does has no permission intent to choose. */
  approvals: boolean;
  /** Whether a profile can ask for `plan` as its permission intent. */
  profilePlan: boolean;
  /** For a model that advertises no levels: every level (depth is a client option), or none (levels belong to the model). */
  effortLadder: boolean;
  /** Opus, Sonnet and Haiku slots, kept as environment variables (claude-slots.ts). */
  modelSlots: boolean;
  /** How a profile's context window reaches the client: through `variable`, listed with the environment, or as a launch option. */
  contextWindow: { variable?: string; help: string };
  /** Said under a profile's proxy, for a proxy the client cannot use. */
  proxyNote?: string;
  /** Said on an empty chat, where the headless client behaves unlike its terminal. */
  chatNote?: string;
  /** The APIs a profile's endpoint can speak to it, when there is a choice: the value the server takes, and how to name it. */
  apis?: readonly { value: string; label: string }[];
  /** How the client names an endpoint's model, when it puts the endpoint's provider in front of the ID. */
  endpointModelPrefix?: string;
  /** A model list the client publishes without a profile: the catalog source it reports, and what to say about it. */
  nativeCatalog?: { source: string; note: string };
  /** Hosts its sign-in links may point at, and its place in the account form (lowest first). */
  account: { loginDomains: readonly string[]; order: number };
  /** Variables that locate its home or mark a nested run; a session cannot override them. */
  reservedEnvironment: readonly string[];
  /** How it is installed, for a client Tendly cannot install from npm. */
  installNote?: string;
}

const CLIENTS: Record<AgentProviderKind, ClientInfo> = {
  claude_code: {
    label: "Claude Code",
    shortLabel: "Claude",
    glyph: {
      path: "M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z",
    },
    iconTone: "orange",
    tabTint: "#FFF0E5",
    colorToken: "claude",
    permissionModes: ["ask", "plan", "accept_edits", "danger"],
    approvals: true,
    profilePlan: true,
    effortLadder: true,
    modelSlots: true,
    contextWindow: {
      variable: "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
      help: "Claude Code is told it through CLAUDE_CODE_MAX_CONTEXT_TOKENS, listed with the environment variables below. Empty lets Claude Code decide.",
    },
    proxyNote: "Claude Code does not support SOCKS.",
    chatNote:
      "Claude headless mode skips the interactive workspace-trust prompt. Send messages only for directories you trust; supported tool approvals still come from the native client.",
    account: { loginDomains: ["claude.ai", "anthropic.com"], order: 1 },
    reservedEnvironment: ["CLAUDE_CONFIG_DIR", "CLAUDECODE"],
  },
  codex: {
    label: "Codex",
    shortLabel: "Codex",
    glyph: {
      path: "M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z",
      evenOdd: true,
    },
    iconTone: "purple",
    tabTint: "#F0E9FF",
    colorToken: "codex",
    permissionModes: ["ask", "danger"],
    approvals: true,
    profilePlan: false,
    effortLadder: false,
    modelSlots: false,
    contextWindow: { help: "Codex is told it as model_context_window at launch. Empty lets Codex decide." },
    nativeCatalog: {
      source: "codex://model/list",
      note: "Loaded from the native Codex catalog. Account access may differ.",
    },
    // The account form has always opened on Codex.
    account: { loginDomains: ["openai.com", "chatgpt.com"], order: 0 },
    reservedEnvironment: ["CODEX_HOME"],
  },
  pi: {
    label: "Pi",
    shortLabel: "Pi",
    // Pi's own mark (https://pi.dev/logo-auto.svg), its three blocks scaled from
    // an 800 to a 24 grid and drawn in one colour like the other marks.
    glyph: {
      path: "M4.96 4.96H15.52V12H12V8.48H4.96ZM4.96 8.48H8.48V12H12V15.52H8.48V19.04H4.96ZM15.52 12H19.04V19.04H15.52Z",
    },
    iconTone: "blue",
    tabTint: "#E8F0FA",
    colorToken: "pi",
    // The provider Tendly writes for a profile's endpoint (server adapters/pi.rs).
    endpointModelPrefix: "tendly/",
    apis: [
      { value: "openai-completions", label: "OpenAI Chat Completions" },
      { value: "openai-responses", label: "OpenAI Responses" },
      { value: "anthropic-messages", label: "Anthropic Messages" },
      { value: "google-generative-ai", label: "Google Generative AI" },
    ],
    permissionModes: [],
    approvals: false,
    profilePlan: false,
    effortLadder: false,
    modelSlots: false,
    contextWindow: {
      help: "Pi is told it per model, in the models.json Tendly writes for the endpoint. Empty lets Pi decide.",
    },
    chatNote:
      "Pi runs its tools without asking. Send messages only for work you are happy for it to carry out in this directory.",
    account: { loginDomains: [], order: 2 },
    reservedEnvironment: ["PI_CODING_AGENT_DIR"],
  },
  gemini: {
    label: "Gemini",
    shortLabel: "Gemini",
    // Gemini's spark. Source: @lobehub/icons-static-svg 1.95.0, MIT; see docs/third-party-notices.md.
    glyph: {
      path: "M20.616 10.835a14.147 14.147 0 01-4.45-3.001 14.111 14.111 0 01-3.678-6.452.503.503 0 00-.975 0 14.134 14.134 0 01-3.679 6.452 14.155 14.155 0 01-4.45 3.001c-.65.28-1.318.505-2.002.678a.502.502 0 000 .975c.684.172 1.35.397 2.002.677a14.147 14.147 0 014.45 3.001 14.112 14.112 0 013.679 6.453.502.502 0 00.975 0c.172-.685.397-1.351.677-2.003a14.145 14.145 0 013.001-4.45 14.113 14.113 0 016.453-3.678.503.503 0 000-.975 13.245 13.245 0 01-2.003-.678z",
      evenOdd: true,
    },
    iconTone: "teal",
    tabTint: "#E5F4F1",
    colorToken: "gemini",
    // `danger` runs agy in its auto mode and Tendly approves what it still asks (server gemini_chat.rs).
    permissionModes: ["ask", "plan", "accept_edits", "danger"],
    approvals: true,
    profilePlan: true,
    effortLadder: false,
    modelSlots: false,
    contextWindow: { help: "agy decides the context window itself; Tendly has no way to tell it one." },
    nativeCatalog: {
      source: "agy://models",
      note: "Loaded from agy for this Google account. Quota and access follow the account's plan.",
    },
    chatNote:
      "Gemini runs through Google's Antigravity CLI (agy acp). Messages sent while it works wait and go as the next turn.",
    account: { loginDomains: ["accounts.google.com"], order: 3 },
    reservedEnvironment: ["TENDLY_GEMINI_DIR"],
    installNote:
      "Gemini runs through Google's Antigravity CLI (agy), which installs from Google rather than npm. Once it is on the PATH, agy update keeps it current.",
  },
};

/** Every agent client in menu order; the first is the default wherever one has to be picked. */
export const AGENT_CLIENTS = Object.keys(CLIENTS) as AgentProviderKind[];
export const DEFAULT_CLIENT: AgentProviderKind = AGENT_CLIENTS[0];
/** What a session can run: every agent client, then the plain terminal. */
export const PROVIDER_KINDS: readonly ProviderKind[] = [...AGENT_CLIENTS, "terminal"];
/** The account form keeps an order of its own. */
export const ACCOUNT_CLIENTS = [...AGENT_CLIENTS].sort((a, b) => CLIENTS[a].account.order - CLIENTS[b].account.order);

export function isAgentClient(value: unknown): value is AgentProviderKind {
  return typeof value === "string" && Object.hasOwn(CLIENTS, value);
}
export function isProviderKind(value: unknown): value is ProviderKind {
  return value === "terminal" || isAgentClient(value);
}
export function clientInfo(id: AgentProviderKind): ClientInfo {
  return CLIENTS[id];
}
/** The entry behind a session's provider; nothing for the terminal or an unknown one. */
export function agentClient(provider: string): ClientInfo | undefined {
  return isAgentClient(provider) ? CLIENTS[provider] : undefined;
}
export function clientLabel(id: AgentProviderKind): string {
  return CLIENTS[id].label;
}
/** A provider's name as stored with a session; an unknown one reads as itself. */
export function providerLabel(provider: string): string {
  return agentClient(provider)?.label ?? (provider === "terminal" ? "Terminal" : provider);
}
/** Inline custom properties that tint a client's mark, read as `var(--client-ink)` and `var(--client-soft)`. */
export function clientTone(provider: string): Record<string, string> {
  const token = agentClient(provider)?.colorToken;
  return token ? { "--client-ink": `var(--${token})`, "--client-soft": `var(--${token}-soft)` } : {};
}
/** What to say about a catalog a client published itself, found by the source it reports. */
export function nativeCatalogNote(source?: string): string | undefined {
  return source
    ? AGENT_CLIENTS.map(id => CLIENTS[id].nativeCatalog).find(catalog => catalog?.source === source)?.note
    : undefined;
}
