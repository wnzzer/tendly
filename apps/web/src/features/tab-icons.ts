import type { AgentProviderKind, ProviderKind } from "@tendly/protocol";
import type { LeafPaneKind } from "@tendly/protocol/layout";
import { AGENT_CLIENTS, clientInfo, isProviderKind } from "./clients";
import { ICON_PALETTE } from "./icon-palette";
import { mediaKind } from "./media-kind";

const CLIENT_TABS = Object.fromEntries(
  AGENT_CLIENTS.map(id => [id, { color: ICON_PALETTE[clientInfo(id).iconTone], background: clientInfo(id).tabTint }]),
) as Record<AgentProviderKind, { color: string; background: string }>;
export const TAB_ICON_PALETTE = {
  ...CLIENT_TABS,
  agent: { color: "#6958A6", background: "#F1ECFA" },
  git: { color: ICON_PALETTE.orange, background: "#FFF0E5" },
  text: { color: ICON_PALETTE.blue, background: "#E8F1FD" },
  image: { color: "#9053B0", background: "#F4E9FB" },
  video: { color: "#AD456E", background: "#FBE8EF" },
  audio: { color: "#986316", background: "#FFF3D9" },
  pdf: { color: "#B54E4D", background: "#FDECE8" },
  terminal: { color: ICON_PALETTE.blue, background: "#E8F1FD" },
} as const;

export type TabIconType = keyof typeof TAB_ICON_PALETTE;
export interface TabIconAppearance {
  type: TabIconType;
  icon: "spark" | "git" | "file" | "image" | "play" | "terminal";
  provider?: ProviderKind;
  color: string;
  background: string;
}

/** Authoritative session data wins over legacy serialized metadata. Never guess from a title. */
export function tabIconProvider(
  metadata?: Record<string, unknown>,
  sessionProviders?: Record<string, ProviderKind>,
): ProviderKind | undefined {
  const sessionId = metadata?.session_id;
  const actual =
    typeof sessionId === "string" && sessionProviders && Object.hasOwn(sessionProviders, sessionId)
      ? sessionProviders[sessionId]
      : undefined;
  return isProviderKind(actual) ? actual : isProviderKind(metadata?.provider) ? metadata.provider : undefined;
}

export function resolveTabIcon(
  kind: LeafPaneKind,
  metadata?: Record<string, unknown>,
  sessionProviders?: Record<string, ProviderKind>,
): TabIconAppearance {
  function appearance(type: TabIconType, icon: TabIconAppearance["icon"], provider?: ProviderKind): TabIconAppearance {
    return { type, icon, ...TAB_ICON_PALETTE[type], ...(provider ? { provider } : {}) };
  }
  if (kind === "agent_chat" || kind === "terminal") {
    const provider = tabIconProvider(metadata, sessionProviders);
    if (provider) return appearance(provider, provider === "terminal" ? "terminal" : "spark", provider);
    return kind === "terminal" ? appearance("terminal", "terminal") : appearance("agent", "spark");
  }
  if (kind === "git_diff") return appearance("git", "git");
  const path = typeof metadata?.path === "string" ? metadata.path : "";
  const media = mediaKind(path);
  if (media === "image" || media === "converted") return appearance("image", "image");
  if (media === "video") return appearance("video", "play");
  if (media === "audio") return appearance("audio", "play");
  if (media === "pdf") return appearance("pdf", "file");
  return kind === "file_preview" && !path ? appearance("image", "image") : appearance("text", "file");
}
