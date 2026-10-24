<script setup lang="ts">
import { onMounted, onBeforeUnmount, computed } from 'vue';
import { useRouter, useRoute } from 'vue-router';
import { useUserStore } from './stores/userStore';
import { useSettingsStore } from './stores/settingsStore';
import {
  availableUpdate,
  dismissedVersion,
  streaming,
  updateRequired,
  updateBusy,
  startUpdateChecks,
  installUpdate,
} from './lib/updater';

const userStore = useUserStore();
const settingsStore = useSettingsStore();
const router = useRouter();
const route = useRoute();

// Optional updates only; a required one has its own screen.
const showUpdateBanner = computed(() =>
  !!availableUpdate.value
  && !updateRequired.value
  && !streaming.value
  && dismissedVersion.value !== availableUpdate.value.version);

function dismissUpdate() {
  dismissedVersion.value = availableUpdate.value?.version ?? null;
}

let stopUpdateChecks: (() => void) | null = null;
onBeforeUnmount(() => stopUpdateChecks?.());

onMounted(async () => {
  stopUpdateChecks = startUpdateChecks();
  await Promise.all([userStore.hydrate(), settingsStore.hydrate()]);
  if (userStore.isLoggedIn && route.path !== '/start') {
    await router.replace('/start');
  } else if (!userStore.isLoggedIn && route.meta.requiresAuth) {
    await router.replace('/login');
  }
});
</script>

<template>
  <main class="app-shell">
    <div v-if="showUpdateBanner" class="update-banner">
      <span>StreamSnipe {{ availableUpdate?.version }} is available.</span>
      <button :disabled="updateBusy" @click="installUpdate">
        {{ updateBusy ? 'Updating…' : 'Update & restart' }}
      </button>
      <span class="dismiss" @click="dismissUpdate">✕</span>
    </div>
    <div v-if="userStore.hydrating" class="loading">Loading…</div>
    <router-view v-else />
  </main>
</template>

<style scoped>
.app-shell {
  position: relative;
  width: 100%;
  height: 100%;
  background: var(--bg);
  color: var(--text);
  overflow: hidden;
}
.update-banner {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 100;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px 6px 14px;
  border-radius: 8px;
  background: var(--surface-2);
  border: 1px solid var(--accent-soft);
  box-shadow: 0 4px 20px var(--accent-glow);
  font-size: 12px;
  white-space: nowrap;
}
.update-banner button {
  padding: 4px 10px;
  border-radius: 5px;
  background: var(--accent);
  color: #fff;
  border: none;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}
.update-banner button:disabled {
  opacity: 0.6;
  cursor: default;
}
.update-banner .dismiss {
  color: var(--text-mute);
  cursor: pointer;
  padding: 0 4px;
}
.loading {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-dim);
  font-size: 12px;
}
</style>

<style>
input,
button {
  font-family: inherit;
}
.app-input {
  background: var(--surface-2);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 13px;
  margin: 4px 0;
}
.app-input:focus {
  outline: none;
  border-color: var(--accent);
}
</style>
