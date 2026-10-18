<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import Icon from './Icon.vue';
import CopyButton from './CopyButton.vue';
import { errorMessage } from './api';
import { readTailnet, recheckTailnet, switchTailnet, tailnetLoginCode, tailnetSteps, tokenFromHash, type TailnetStatus, type TailnetStep } from './tailnet';
import { useI18n } from '../i18n';

/**
 * Remote access through Tailscale, as a guide rather than a form.
 *
 * Every way it can fail is a step with its own explanation and the one thing
 * that fixes it, so the page is never a dead end: the server reports where it
 * stopped, the page says what to do there, and both check again by themselves
 * once it has been done.
 */
const { t } = useI18n();
const status = ref<TailnetStatus>();
const error = ref(''), busy = ref(false), confirmingOff = ref(false), showDetails = ref(false);
const code = ref<{ link: string; svg: string }>(), codeError = ref('');

const steps = computed(() => status.value ? tailnetSteps(status.value) : []);
const verifying = computed(() => status.value?.phase === 'live' && status.value.verified === null);
const problem = computed(() => status.value?.problem ?? null);
const token = computed(() => code.value ? tokenFromHash(new URL(code.value.link).hash) : null);
const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || navigator.userAgent);

const STEP_LABELS: Record<TailnetStep, string> = {
  install: 'Install Tailscale on this machine',
  connect: 'Sign in and connect',
  name: "Get this machine's tailnet name",
  publish: 'Publish an HTTPS address',
  verify: 'Check it from this machine',
};
/** What each problem means, in the person's terms; Tailscale's own words go under it. */
const PROBLEMS: Record<string, { title: string; body: string }> = {
  not_installed: { title: 'Tailscale is not installed here', body: 'Install it and sign in. Use the same account on your phone later.' },
  outdated: { title: 'This Tailscale is too old', body: 'Publishing needs Tailscale 1.52 or newer. Update it, then check again.' },
  daemon_down: { title: 'Tailscale is installed but not running', body: 'Open the Tailscale app; it lives in the menu bar.' },
  not_running: { title: 'Tailscale is signed out or disconnected', body: isMac ? 'Open the Tailscale app from the menu bar, sign in and connect. Or run:' : 'Sign in and connect:' },
  no_magic_dns: { title: 'MagicDNS is off for your tailnet', body: 'Turn on MagicDNS on the DNS page of the Tailscale admin console. It gives this machine the name the address is built on.' },
  serve_consent: { title: 'Allow Serve and HTTPS for your tailnet', body: 'Tailscale asks once per tailnet. Open the link, approve it, and come back. Tendly notices by itself within a minute.' },
  no_https: { title: 'HTTPS certificates are off for your tailnet', body: 'Enable HTTPS Certificates on the DNS page of the Tailscale admin console, then check again.' },
  port_taken: { title: 'Another service already uses this HTTPS address', body: 'Tendly will not replace it. If it is no longer needed, remove it and check again:' },
  access_denied: { title: 'This user may not change Tailscale settings', body: 'Allow it once by making yourself the Tailscale operator:' },
  unreachable: { title: 'Published, but this machine could not open it', body: 'The first certificate can take a minute, and some machines do not resolve their own tailnet name. Try it on your phone; Tendly checks again every minute.' },
  failed: { title: 'Tailscale could not publish the address', body: 'What Tailscale said:' },
};
const explained = computed(() => {
  if (!problem.value) return null;
  // A daemon with a start command is a service (Linux, Homebrew); without one it is the app.
  if (problem.value.code === 'daemon_down' && problem.value.command) return { ...PROBLEMS.daemon_down, body: 'Start its background service:' };
  return PROBLEMS[problem.value.code] ?? PROBLEMS.failed;
});
const linkLabel = computed(() => {
  const link = problem.value?.link ?? '';
  if (link.includes('/admin/dns')) return 'Open DNS settings';
  if (link.includes('tailscale.com/download')) return 'Download Tailscale';
  return 'Open in Tailscale';
});

let timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
/** Fast while an attempt runs, so each step lights up as it passes; slow otherwise. */
function schedule() {
  clearTimeout(timer);
  if (disposed) return;
  timer = setTimeout(() => void load(true), verifying.value || status.value?.phase === 'starting' ? 1500 : 15000);
}
async function load(quiet = false) {
  try {
    const next = await readTailnet();
    const wasLive = status.value?.phase === 'live';
    status.value = next;
    if (!quiet) error.value = '';
    if (next.phase === 'live' && !wasLive) code.value = undefined;
  } catch (cause) { if (!quiet) error.value = errorMessage(cause); }
  schedule();
}
onMounted(() => load());
onBeforeUnmount(() => { disposed = true; clearTimeout(timer); });

async function act(action: () => Promise<TailnetStatus>) {
  busy.value = true; error.value = '';
  try { status.value = await action(); } catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; schedule(); }
}
function turnOn() { void act(() => switchTailnet(true)); }
function turnOff() {
  // From the phone itself, switching off is switching this page off too.
  if (status.value?.via_tailnet && !confirmingOff.value) { confirmingOff.value = true; return; }
  confirmingOff.value = false; code.value = undefined;
  void act(() => switchTailnet(false));
}
function toggle(on: boolean) { if (on) turnOn(); else turnOff(); }
const recheck = () => void act(recheckTailnet);

async function revealCode() {
  codeError.value = '';
  try { code.value = await tailnetLoginCode(); } catch (cause) { codeError.value = errorMessage(cause); }
}
const checkedAt = computed(() => status.value?.checked_at ? new Date(status.value.checked_at).toLocaleTimeString() : '');
</script>

<template>
  <section class="remote-page">
    <div class="settings-lead"><p>{{ t('Open this workspace from your phone or another computer, anywhere. Tailscale connects only the devices signed in to your own tailnet, over HTTPS, without opening any port to the internet.') }}</p></div>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>

    <article class="preference-card remote-card">
      <header>
        <span class="preference-mark general"><Icon name="globe" :size="17"/></span>
        <strong>Tailscale</strong>
        <span v-if="status" :class="['remote-pill', status.phase]">
          <span class="remote-dot" aria-hidden="true"/>{{ t(status.phase === 'live' ? 'Live' : status.phase === 'starting' ? 'Publishing…' : status.phase === 'error' ? 'Needs attention' : 'Off') }}
        </span>
        <input v-if="status" type="checkbox" class="preference-switch remote-switch" role="switch" :checked="status.enabled" :disabled="busy" :aria-label="t('Remote access through Tailscale')" @change="toggle(($event.target as HTMLInputElement).checked)" />
      </header>

      <p v-if="confirmingOff" class="preference-note danger remote-confirm" role="alert">
        {{ t('You are using this page through the tailnet. Turning it off disconnects this device; turn it on again from the computer itself.') }}
        <span class="remote-actions"><button type="button" class="small-button danger" @click="turnOff">{{ t('Turn off anyway') }}</button><button type="button" class="small-button" @click="confirmingOff = false">{{ t('Cancel') }}</button></span>
      </p>

      <!-- Off: what it does, and the one button that starts it. -->
      <div v-if="status && !status.enabled" class="remote-intro">
        <ul>
          <li><Icon name="shield" :size="15"/><span>{{ t('Only devices on your tailnet can connect, and each one signs in with this workspace\'s access token once.') }}</span></li>
          <li><Icon name="globe" :size="15"/><span>{{ t('A stable HTTPS address such as https://your-mac.tailnet.ts.net, with a real certificate.') }}</span></li>
          <li><Icon name="check" :size="15"/><span>{{ t('Nothing changes for this machine: it keeps using Tendly without a token.') }}</span></li>
        </ul>
        <button type="button" class="primary-button" :disabled="busy" @click="turnOn"><Icon name="globe" :size="14"/>{{ t('Turn on remote access') }}</button>
        <p class="remote-hint">{{ t('Tendly walks you through anything missing. You need a free Tailscale account.') }} <a href="https://tailscale.com/download" target="_blank" rel="noopener">{{ t('Get Tailscale') }}</a></p>
      </div>

      <!-- On: the steps, where it stopped, and what to do there. -->
      <template v-else-if="status">
        <ol class="remote-steps">
          <li v-for="step in steps" :key="step.id" :class="['remote-step', step.state]">
            <span class="remote-step-mark" aria-hidden="true">
              <Icon v-if="step.state === 'done'" name="check" :size="13"/>
              <span v-else-if="step.state === 'active'" class="remote-spinner"/>
              <span v-else-if="step.state === 'failed' || step.state === 'warning'">!</span>
            </span>
            <span class="remote-step-label">{{ t(STEP_LABELS[step.id]) }}</span>
          </li>
        </ol>

        <p v-if="verifying" class="remote-hint" role="status">{{ t('Checking that the address opens. The first time, Tailscale issues its HTTPS certificate, which can take a minute or two.') }}</p>

        <div v-if="problem && explained" :class="['remote-problem', { warning: status.phase === 'live' }]" role="status">
          <strong>{{ t(explained.title) }}</strong>
          <p>{{ t(explained.body) }}</p>
          <div v-if="problem.command" class="update-command"><code>{{ problem.command }}</code><CopyButton :text="problem.command" :label="t('Copy command')"/></div>
          <template v-if="problem.code === 'not_installed' && problem.link?.endsWith('/windows')">
            <p>{{ t('On Windows, run the install command above in PowerShell and approve the installer when prompted.') }}</p>
            <p>{{ t('If no matching package is found, update the WinGet sources, then retry the install command:') }}</p>
            <div class="update-command"><code>winget source update</code><CopyButton text="winget source update" :label="t('Copy command')"/></div>
            <p>{{ t('If WinGet is unavailable or installation still fails, use Download Tailscale below to get the official Windows installer.') }}</p>
            <p>{{ t('After installing, reopen PowerShell. If Tendly still cannot find Tailscale, restart Tendly, then check again.') }}</p>
          </template>
          <p v-if="problem.code === 'not_running'">{{ t('Run the command on this computer, then open the sign-in link it prints and authorize this device in your browser. You can also open the Tailscale app and sign in. Tendly checks again automatically after you connect.') }}</p>
          <p v-if="problem.code === 'port_taken' || problem.code === 'failed' || problem.code === 'not_installed'" class="remote-detail">{{ problem.detail }}</p>
          <div class="remote-actions">
            <a v-if="problem.link" class="primary-button" :href="problem.link" target="_blank" rel="noopener"><Icon name="arrow" :size="14"/>{{ t(linkLabel) }}</a>
            <button type="button" :class="problem.link ? 'secondary-button' : 'primary-button'" :disabled="busy || status.phase === 'starting'" @click="recheck"><Icon name="refresh" :size="14"/>{{ t(status.phase === 'live' ? 'Check again' : 'Done, check again') }}</button>
          </div>
        </div>

        <!-- Live: the address, and the fastest way onto a phone. -->
        <div v-if="status.phase === 'live' && status.url" class="remote-live">
          <div class="remote-url">
            <a :href="status.url" target="_blank" rel="noopener">{{ status.url }}</a>
            <CopyButton :text="status.url" :label="t('Copy link')"/>
          </div>
          <div class="remote-phone">
            <div class="remote-phone-steps">
              <strong>{{ t('On your phone') }}</strong>
              <ol>
                <li>{{ t('Install Tailscale and sign in with the same account.') }} <a href="https://tailscale.com/download" target="_blank" rel="noopener">{{ t('Get Tailscale') }}</a></li>
                <li>{{ t('Scan the sign-in code with the camera. It opens Tendly already signed in.') }}</li>
                <li>{{ t('Optional: add it to the home screen to open it like an app.') }}</li>
              </ol>
              <p class="remote-hint">{{ t('Signed-in devices stay signed in for 30 days.') }}</p>
            </div>
            <div class="remote-qr-box">
              <template v-if="code">
                <div class="remote-qr" v-html="code.svg"/>
                <p class="remote-qr-note">{{ t('Contains the access token. Do not share a screenshot of it.') }}</p>
                <div class="remote-actions">
                  <CopyButton v-if="token" :text="token" :label="t('Copy access token')"/>
                  <button type="button" class="small-button" @click="code = undefined">{{ t('Hide') }}</button>
                </div>
              </template>
              <button v-else type="button" class="remote-qr-reveal" @click="revealCode">
                <Icon name="grid" :size="22"/><span>{{ t('Show sign-in code') }}</span>
              </button>
              <p v-if="codeError" class="inline-error" role="alert">{{ codeError }}</p>
            </div>
          </div>
        </div>

        <p v-if="status.forced" class="remote-hint">{{ t('Started with --tailscale or tailscale = true in config.toml, so the next start turns it on again whatever this switch says.') }}</p>

        <div v-if="status.machine" class="remote-details">
          <button type="button" class="remote-details-toggle" :aria-expanded="showDetails" @click="showDetails = !showDetails"><Icon name="chevron" :size="13" :class="{ open: showDetails }"/>{{ t('Details') }}</button>
          <dl v-if="showDetails">
            <dt>{{ t('Name') }}</dt><dd>{{ status.machine.name }}</dd>
            <template v-if="status.machine.ip"><dt>{{ t('Tailnet IP') }}</dt><dd>{{ status.machine.ip }} <small>{{ t('for troubleshooting only; open the name, not the IP') }}</small></dd></template>
            <template v-if="status.machine.login"><dt>{{ t('Signed in as') }}</dt><dd>{{ status.machine.login }}</dd></template>
            <template v-if="status.machine.tailnet"><dt>{{ t('Tailnet') }}</dt><dd>{{ status.machine.tailnet }}</dd></template>
            <dt>{{ t('Forwards to') }}</dt><dd>{{ status.target }}</dd>
            <template v-if="checkedAt"><dt>{{ t('Last checked') }}</dt><dd>{{ checkedAt }}</dd></template>
          </dl>
          <p v-if="showDetails" class="remote-hint">{{ t('The name is issued by Tailscale and appears in public certificate logs. Knowing it does not let anyone outside your tailnet connect.') }}</p>
        </div>
      </template>
      <p v-else class="remote-hint">{{ t('Loading…') }}</p>
    </article>
    <p class="update-hint">{{ t('From a terminal: tendly --tailscale starts with it on, and tendly status prints the address and token.') }}</p>
  </section>
</template>

<style scoped>
.remote-page{min-width:0}
.settings-lead{margin:2px 0 16px}
.settings-lead p{margin:0;font-size:var(--text-sm);line-height:1.7;color:var(--ink-soft)}
.remote-card{border:1px solid var(--border);border-radius:var(--radius-lg);padding:13px 14px 14px;background:var(--surface)}
.remote-card header{display:flex;align-items:center;gap:10px;margin-bottom:6px}
.remote-card header strong{font-size:var(--text-md);font-weight:600;color:var(--ink)}
.preference-mark{display:grid;place-items:center;width:30px;height:30px;border-radius:var(--radius-md);background:var(--fill);color:var(--ink-soft);flex-shrink:0}
.remote-pill{display:inline-flex;align-items:center;gap:6px;padding:2px 9px;border-radius:999px;background:var(--fill);font-size:var(--text-xs);font-weight:550;color:var(--muted)}
.remote-dot{width:7px;height:7px;border-radius:50%;background:currentColor}
.remote-pill.live{color:var(--teal);background:color-mix(in srgb,var(--teal) 12%,transparent)}
.remote-pill.starting{color:var(--accent)}
.remote-pill.error{color:var(--danger-ink);background:color-mix(in srgb,var(--danger-ink) 10%,transparent)}
.remote-switch{margin-left:auto}
.preference-switch{appearance:none;flex:none;position:relative;width:38px;height:22px;border-radius:var(--radius-md);background:var(--line-strong);cursor:pointer;transition:background .15s}
.preference-switch::after{content:"";position:absolute;top:2px;left:2px;width:18px;height:18px;border-radius:50%;background:var(--surface);box-shadow:var(--shadow-sm);transition:transform .15s}
.preference-switch:checked{background:var(--teal)}
.preference-switch:checked::after{transform:translateX(16px)}
.preference-switch:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
.preference-note{margin:6px 0 8px;font-size:var(--text-xs);line-height:1.5}
.preference-note.danger{color:var(--danger-ink)}
.remote-confirm .remote-actions{margin-top:8px}
.small-button.danger{color:var(--danger-ink);border-color:var(--danger-line)}
.remote-intro{padding-top:10px;border-top:1px solid var(--border)}
.remote-intro ul{list-style:none;margin:0 0 14px;padding:0;display:grid;gap:8px}
.remote-intro li{display:flex;gap:9px;align-items:flex-start;font-size:var(--text-sm);line-height:1.55;color:var(--ink-soft)}
.remote-intro li svg{margin-top:3px;color:var(--teal)}
.remote-steps{list-style:none;margin:0;padding:10px 0 4px;border-top:1px solid var(--border);display:grid;gap:2px}
.remote-step{display:flex;align-items:center;gap:10px;min-height:28px;font-size:var(--text-sm);color:var(--muted)}
.remote-step-mark{display:grid;place-items:center;width:20px;height:20px;border-radius:50%;border:1.5px solid var(--line-strong);font-size:var(--text-xs);font-weight:700;flex-shrink:0}
.remote-step.done{color:var(--ink-soft)}
.remote-step.done .remote-step-mark{border-color:var(--teal);background:var(--teal);color:var(--surface)}
.remote-step.active{color:var(--ink);font-weight:550}
.remote-step.active .remote-step-mark{border-color:var(--accent)}
.remote-step.failed{color:var(--danger-ink);font-weight:550}
.remote-step.failed .remote-step-mark{border-color:var(--danger-ink);background:var(--danger-ink);color:var(--surface)}
.remote-step.warning{color:var(--ink);font-weight:550}
.remote-step.warning .remote-step-mark{border-color:var(--warn);background:var(--warn);color:var(--surface)}
.remote-spinner{width:10px;height:10px;border-radius:50%;border:2px solid var(--accent);border-right-color:transparent;animation:remote-spin .8s linear infinite}
@keyframes remote-spin{to{transform:rotate(360deg)}}
@media (prefers-reduced-motion: reduce){.remote-spinner{animation:none}}
.remote-problem{margin-top:10px;padding:12px 13px;border-radius:var(--radius-md);background:color-mix(in srgb,var(--danger-ink) 7%,var(--surface));border:1px solid color-mix(in srgb,var(--danger-ink) 22%,transparent)}
.remote-problem.warning{background:var(--fill);border-color:var(--border)}
.remote-problem strong{display:block;font-size:var(--text-sm);font-weight:600;color:var(--ink)}
.remote-problem p{margin:4px 0 10px;font-size:var(--text-sm);line-height:1.6;color:var(--ink-soft)}
.remote-problem .remote-detail{font-family:var(--mono);font-size:var(--text-xs);color:var(--muted);white-space:pre-wrap;overflow-wrap:anywhere}
.update-command{display:flex;align-items:center;gap:8px;min-width:0;margin-bottom:10px}
.update-command code{flex:1;min-width:0;overflow-x:auto;white-space:nowrap;padding:7px 9px;border-radius:var(--radius-sm);background:var(--fill);font-size:var(--text-sm);color:var(--ink)}
.remote-actions{display:flex;gap:8px;flex-wrap:wrap;align-items:center}
.remote-live{margin-top:12px;padding-top:12px;border-top:1px solid var(--border)}
.remote-url{display:flex;align-items:center;gap:8px;min-width:0;padding:9px 11px;border-radius:var(--radius-md);background:var(--fill)}
.remote-url a{flex:1;min-width:0;overflow-wrap:anywhere;font-family:var(--mono);font-size:var(--text-md);font-weight:550;color:var(--accent);text-decoration:none}
.remote-url a:hover{text-decoration:underline}
.remote-phone{display:grid;grid-template-columns:minmax(0,1fr) 240px;gap:18px;margin-top:14px;align-items:start}
.remote-phone-steps strong{font-size:var(--text-sm);font-weight:600;color:var(--ink)}
.remote-phone-steps ol{margin:8px 0 0;padding-left:20px;display:grid;gap:7px;font-size:var(--text-sm);line-height:1.6;color:var(--ink-soft)}
.remote-qr-box{display:flex;flex-direction:column;align-items:center;gap:8px}
/* The code draws its own white quiet zone, so it scans in the dark theme too. */
.remote-qr{width:236px;height:236px;border-radius:var(--radius-md);overflow:hidden}
.remote-qr :deep(svg){display:block;width:100%;height:100%}
.remote-qr-note{margin:0;font-size:var(--text-xs);line-height:1.5;color:var(--danger-ink);text-align:center}
.remote-qr-reveal{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:8px;width:236px;height:236px;border:1.5px dashed var(--line-strong);border-radius:var(--radius-md);background:var(--fill);color:var(--ink-soft);font:inherit;font-size:var(--text-sm);font-weight:550;cursor:pointer}
.remote-qr-reveal:hover{border-color:var(--accent);color:var(--ink)}
.remote-qr-reveal:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
.remote-details{margin-top:12px;padding-top:8px;border-top:1px solid var(--border)}
.remote-details-toggle{display:inline-flex;align-items:center;gap:6px;padding:4px 0;border:0;background:none;font:inherit;font-size:var(--text-xs);font-weight:550;color:var(--muted);cursor:pointer}
.remote-details-toggle svg{transition:transform .15s}
.remote-details-toggle svg.open{transform:rotate(90deg)}
.remote-details dl{display:grid;grid-template-columns:max-content minmax(0,1fr);gap:5px 14px;margin:8px 0 0;font-size:var(--text-xs)}
.remote-details dt{color:var(--muted)}
.remote-details dd{margin:0;font-family:var(--mono);color:var(--ink-soft);overflow-wrap:anywhere}
.remote-details dd small{font-family:var(--font, inherit);color:var(--muted)}
.remote-hint{margin:10px 0 0;font-size:var(--text-xs);line-height:1.6;color:var(--muted)}
.update-hint{margin:10px 0 0;font-size:var(--text-xs);line-height:1.6;color:var(--muted)}
@media(max-width:620px){
  .remote-phone{grid-template-columns:minmax(0,1fr)}
  .remote-qr-box{order:-1}
}
</style>
