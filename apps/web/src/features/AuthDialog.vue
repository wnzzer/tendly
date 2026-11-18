<script setup lang="ts">
import { ref } from "vue";
import { errorMessage, json, request } from "./api";
import ModalDialog from "./ModalDialog.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
/** Reached from a phone, the token is on the computer; say where to find it. */
defineProps<{ viaTailnet?: boolean; linkFailed?: boolean }>();
const emit = defineEmits<{ authenticated: [] }>();
const token = ref(""),
  error = ref(""),
  busy = ref(false);
async function signIn() {
  if (!token.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    await request("/auth", json("POST", { token: token.value }));
    token.value = "";
    emit("authenticated");
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <ModalDialog :title="t('Connect to Tendly')" :closable="false"
    ><form class="form-stack" @submit.prevent="signIn">
      <p class="form-description">
        {{
          t(
            "This host requires its Tendly access token. The browser receives a protected session cookie; the token is not stored in browser storage.",
          )
        }}
        {{ t("This browser stays signed in for 30 days, including across restarts and upgrades.") }}
      </p>
      <p v-if="linkFailed" class="inline-error" role="alert">
        {{ t("That sign-in code is no longer valid. Show a new one on the computer.") }}
      </p>
      <p v-if="viaTailnet" class="form-description auth-remote-hint">
        {{
          t(
            "On the computer running Tendly, open Settings → Remote access and scan the sign-in code with this device, or run tendly status there to see the token.",
          )
        }}
      </p>
      <input
        class="sr-only"
        type="text"
        name="username"
        autocomplete="username"
        value="tendly"
        readonly
        tabindex="-1"
        aria-hidden="true"
      /><label
        >{{ t("Host access token")
        }}<input v-model="token" name="password" type="password" required autocomplete="current-password" autofocus
      /></label>
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <button class="primary-button" :disabled="busy || !token">
        {{ t(busy ? "Connecting…" : "Connect securely") }}
      </button>
    </form></ModalDialog
  >
</template>
