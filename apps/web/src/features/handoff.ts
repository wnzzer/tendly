/** Cross-client destinations. Conversation transfer is persisted by the server. */
import type { EndpointProfile, ProviderKind } from '@tendly/protocol';
import { AGENT_CLIENTS, providerLabel } from './clients';

type Translate = (text: string, values?: Record<string, string | number>) => string;

/** Focus the composer when a handoff opens in a different pane. */
export const handoffArrivals = new Set<string>();

/**
 * What choosing each configuration actually means.
 *
 * "Isolated" read as the careful, better choice, when what it does is start
 * the client with an empty home: no sign-in, none of the host's settings. The
 * menu is the only place that choice is made, so it is where that is said.
 */
export function profileNote(profile: EndpointProfile, t: Translate): string {
  if (profile.native_config) return t('Uses the sign-in and settings in {path}', { path: profile.native_config.config_dir });
  if (profile.endpoint_url) return t('Custom endpoint · {url}', { url: profile.endpoint_url });
  return t('Custom endpoint profile');
}

/** Another client, and the configuration it would start with. */
export interface HandoffChoice { provider: ProviderKind; profileId: string | null; name: string; note: string; workspaceId?: string; branch?: string }

/**
 * Where a conversation with `provider` can be carried: every other client,
 * under each of its profiles and with a fresh isolated home. Offered with an
 * empty conversation too: then it is simply the same work in another client.
 * A terminal has no conversation to carry.
 */
export function handoffChoices(provider: ProviderKind, profiles: readonly EndpointProfile[], t: Translate): HandoffChoice[] {
  if (provider === 'terminal') return [];
  return AGENT_CLIENTS.filter(other => other !== provider).flatMap(other => [
    ...profiles.filter(profile => profile.provider === other).map(profile => ({ provider: other, profileId: profile.id, name: profile.name, note: profileNote(profile, t) })),
    { provider: other, profileId: null, name: t('Native · isolated configuration'), note: t('A fresh, empty client home: no host sign-in or settings, so it may ask you to log in.') },
  ]);
}

/** The new session's title: the old one, with the branch or client it moved to. */
export function handoffTitle(title: string, choice: HandoffChoice, t: Translate): string {
  return t('{title} · {provider}', { title, provider: choice.branch ?? providerLabel(choice.provider) });
}
