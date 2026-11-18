<script setup lang="ts">
import { dismissToast, pauseToast, resumeToast, toasts, type Toast } from "./toasts";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";

const { t } = useI18n();
function act(toast: Toast) {
  dismissToast(toast.id);
  toast.action?.run();
}
</script>

<template>
  <Teleport to="body">
    <div class="toast-host" role="region" :aria-label="t('Notifications')">
      <TransitionGroup name="toast">
        <div
          v-for="toast in toasts"
          :key="toast.id"
          :class="['toast', toast.tone]"
          :role="toast.tone === 'error' ? 'alert' : 'status'"
          @mouseenter="pauseToast(toast.id)"
          @mouseleave="resumeToast(toast.id)"
          @focusin="pauseToast(toast.id)"
          @focusout="resumeToast(toast.id)"
        >
          <Icon :name="toast.tone === 'success' ? 'check' : 'info'" :size="15" class="toast-icon" />
          <span class="toast-message">{{ toast.message }}</span>
          <button v-if="toast.action" type="button" class="toast-action" @click="act(toast)">
            {{ toast.action.label }}
          </button>
          <button type="button" class="toast-close" :aria-label="t('Dismiss')" @click="dismissToast(toast.id)">
            <Icon name="close" :size="13" />
          </button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.toast-host {
  position: fixed;
  left: 50%;
  bottom: calc(36px + env(safe-area-inset-bottom));
  z-index: 200;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  transform: translateX(-50%);
  pointer-events: none;
  width: min(520px, calc(100vw - 24px));
}
.toast {
  --tone: var(--ok);
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  max-width: 100%;
  padding: 10px 10px 10px 14px;
  border-radius: var(--radius-lg);
  background: var(--ink);
  color: var(--surface);
  box-shadow: var(--shadow-lg);
  font-size: var(--text-md);
}
.toast.info {
  --tone: var(--info);
}
.toast.warn {
  --tone: var(--warn);
}
.toast.error {
  --tone: var(--danger);
}
.toast-icon {
  color: var(--tone);
  flex: none;
}
.toast-message {
  flex: 1;
  min-width: 0;
  line-height: 1.45;
}
.toast-action {
  flex: none;
  min-height: 30px;
  padding: 0 12px;
  border: 0;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--surface) 14%, transparent);
  color: var(--surface);
  font-weight: 600;
  font-size: var(--text-sm);
  cursor: pointer;
}
.toast-action:hover {
  background: color-mix(in srgb, var(--surface) 24%, transparent);
}
.toast-close {
  flex: none;
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border: 0;
  border-radius: var(--radius-sm);
  background: none;
  color: color-mix(in srgb, var(--surface) 70%, transparent);
  cursor: pointer;
}
.toast-close:hover {
  color: var(--surface);
}
.toast-enter-active,
.toast-leave-active {
  transition:
    opacity var(--duration-normal),
    transform var(--duration-normal) var(--ease-out);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
@media (max-width: 760px) {
  .toast-host {
    bottom: calc(80px + env(safe-area-inset-bottom));
  }
}
@media (prefers-reduced-motion: reduce) {
  .toast-enter-active,
  .toast-leave-active {
    transition: none;
  }
}
</style>
