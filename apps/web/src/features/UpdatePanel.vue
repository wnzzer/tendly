<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import Icon from "./Icon.vue";
import CopyButton from "./CopyButton.vue";
import { errorMessage, request } from "./api";
import type { BackendHealth } from "./backend-capabilities";
import { installUpdate, readUpdate, restartForUpdate, updateStage, waitForVersion, type UpdateStatus } from "./update";
import { useI18n } from "../i18n";

/**
 * The installed version and the way to a newer one.
 *
 * Installing and restarting are two clicks because only the second costs
 * anything: a restart stops every running session, so the panel says how many
 * and asks again before doing it.
 */
const { t } = useI18n();
const status = ref<UpdateStatus>();
const busy = ref<"" | "checking" | "installing" | "restarting">("");
const error = ref(""),
  confirming = ref(false),
  restartFailed = ref(false);
const stage = computed(() => status.value && updateStage(status.value));

async function load(refresh = false) {
  busy.value = "checking";
  error.value = "";
  try {
    status.value = await readUpdate(refresh);
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    busy.value = "";
  }
}
onMounted(() => load());

async function install() {
  busy.value = "installing";
  error.value = "";
  try {
    status.value = await installUpdate();
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    busy.value = "";
  }
}

async function restart() {
  const target = status.value?.installed;
  if (!target) return;
  if (status.value!.running_sessions > 0 && !confirming.value) {
    confirming.value = true;
    return;
  }
  confirming.value = false;
  busy.value = "restarting";
  error.value = "";
  restartFailed.value = false;
  try {
    await restartForUpdate();
    if (await waitForVersion(target, () => request<BackendHealth>("/health"))) {
      location.reload();
      return;
    }
    restartFailed.value = true;
  } catch (cause) {
    error.value = errorMessage(cause);
  }
  busy.value = "";
}
</script>

<template>
  <section class="update-page">
    <div class="settings-lead">
      <p>{{ t("Which Tendly this server runs, and how to move it to the newest release.") }}</p>
    </div>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>
    <article class="preference-card update-card">
      <div class="update-row">
        <span
          ><strong>{{ t("Installed version") }}</strong
          ><small>{{ status?.current ?? "…" }}</small></span
        >
        <span
          ><strong>{{ t("Latest release") }}</strong
          ><small>{{
            status?.latest ?? (stage === "manual" ? t("Not checked for this kind of install") : "—")
          }}</small></span
        >
        <button type="button" class="small-button" :disabled="!!busy" @click="load(true)">
          <Icon name="refresh" :size="14" />{{ t(busy === "checking" ? "Checking…" : "Check for updates") }}
        </button>
      </div>

      <div v-if="stage === 'current'" class="update-state" role="status">
        <Icon name="check" :size="15" />{{ t("Up to date.") }}
      </div>

      <div v-else-if="stage === 'check-failed'" class="update-state warn" role="status">
        {{ t("Could not check for a newer version: {error}", { error: status!.check_error! }) }}
      </div>

      <div v-else-if="stage === 'available'" class="update-state">
        <p>
          {{
            t(
              "Tendly {version} is available. Installing does not interrupt anything; it takes effect when you restart.",
              { version: status!.latest! },
            )
          }}
        </p>
        <button type="button" class="primary-button" :disabled="!!busy" @click="install">
          <Icon name="download" :size="14" />{{
            t(busy === "installing" ? "Installing… (this can take a minute)" : "Install {version}", {
              version: status!.latest!,
            })
          }}
        </button>
      </div>

      <div v-else-if="stage === 'needs-permission'" class="update-state">
        <p>
          {{
            t(
              "Tendly {version} is available, but this install belongs to another user (usually installed with sudo). Run this on the server:",
              { version: status!.latest! },
            )
          }}
        </p>
        <div class="update-command">
          <code>{{ status!.command }}</code><CopyButton :text="status!.command" :label="t('Copy command')" />
        </div>
        <p class="update-hint">
          {{ t("Installing it under your own npm prefix instead lets this button update it next time.") }}
        </p>
      </div>

      <div v-else-if="stage === 'manual'" class="update-state">
        <p>{{ t("This install was not made by npm, so it cannot update itself. Get the latest release from:") }}</p>
        <div class="update-command">
          <code>{{ status!.command }}</code><CopyButton :text="status!.command" :label="t('Copy link')" />
        </div>
      </div>

      <div v-else-if="stage === 'restart'" class="update-state">
        <p>{{ t("Tendly {version} is installed. Restart to use it.", { version: status!.installed! }) }}</p>
        <template v-if="status!.restart === 'manual'">
          <p class="update-hint">
            {{
              t(
                "Nothing would start this server again on its own. Restart it where it was started, for example with tendly restart.",
              )
            }}
          </p>
        </template>
        <template v-else>
          <p v-if="confirming" class="preference-note danger" role="alert">
            {{
              t("Restarting stops {count} running sessions. Their history stays; start them again afterwards.", {
                count: status!.running_sessions,
              })
            }}
          </p>
          <p v-if="restartFailed" class="preference-note danger" role="alert">
            {{
              t(
                "Tendly did not come back with the new version within 90 seconds. Check the server, for example with tendly status.",
              )
            }}
          </p>
          <div class="update-actions">
            <button
              type="button"
              :class="confirming ? 'primary-button danger' : 'primary-button'"
              :disabled="!!busy"
              @click="restart"
            >
              <Icon name="refresh" :size="14" />{{
                t(busy === "restarting" ? "Restarting…" : confirming ? "Stop sessions and restart" : "Restart now")
              }}
            </button>
            <button v-if="confirming" type="button" class="secondary-button" @click="confirming = false">
              {{ t("Cancel") }}
            </button>
          </div>
        </template>
      </div>
    </article>
    <p class="update-hint">
      {{ t("From a terminal: tendly update installs the same way, and tendly update --check only looks.") }}
    </p>
  </section>
</template>

<style scoped>
.update-page {
  min-width: 0;
}
.settings-lead {
  margin: 2px 0 16px;
}
.settings-lead p {
  margin: 0;
  font-size: var(--text-sm);
  line-height: 1.7;
  color: var(--ink-soft);
}
.update-card {
  padding-bottom: 13px;
}
.update-row {
  display: flex;
  align-items: center;
  gap: 18px;
  flex-wrap: wrap;
}
.update-row > span {
  min-width: 110px;
}
.update-row > span:nth-child(2) {
  flex: 1;
}
.update-row strong {
  display: block;
  font-size: var(--text-sm);
  font-weight: 550;
  color: var(--ink);
}
.update-row small {
  display: block;
  margin-top: 2px;
  font-size: var(--text-sm);
  font-family: var(--mono, ui-monospace, monospace);
  color: var(--ink-soft);
}
.update-state {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--border);
  font-size: var(--text-sm);
  line-height: 1.6;
  color: var(--ink-soft);
}
.update-state[role="status"] {
  display: flex;
  align-items: center;
  gap: 7px;
}
.update-state.warn {
  color: var(--danger-ink);
}
.update-state p {
  margin: 0 0 10px;
}
.update-command {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.update-command code {
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  white-space: nowrap;
  padding: 7px 9px;
  border-radius: var(--radius-sm);
  background: var(--fill);
  font-size: var(--text-sm);
  color: var(--ink);
}
.update-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.primary-button.danger {
  background: var(--danger-ink);
  border-color: var(--danger-ink);
}
.update-hint {
  margin: 10px 0 0;
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--muted);
}
</style>
