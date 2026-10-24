<script setup lang="ts">
import { computed } from 'vue';
import { open } from '@tauri-apps/plugin-shell';
import {
  DOWNLOAD_URL,
  updateRequired,
  updateStatus,
  updateBusy,
  updateProgress,
  installUpdate,
} from './lib/updater';

const percent = computed(() =>
  updateProgress.value === null ? null : Math.round(updateProgress.value * 100));
const failed = computed(() => updateStatus.value.startsWith('Update failed'));

function openDownloadPage() {
  open(DOWNLOAD_URL).catch(err => console.error('[Updater] Failed to open download page:', err));
}
</script>

<template>
  <div class="update-root">
    <img class="logo" alt="logo" src="./assets/logo.jpg" />
    <h1>Update required</h1>
    <p class="message">{{ updateRequired }}</p>

    <button class="primary" :disabled="updateBusy" @click="installUpdate">
      {{ updateBusy ? 'Updating…' : 'Update now' }}
    </button>

    <div v-if="updateBusy && percent !== null" class="bar">
      <div class="fill" :style="{ width: percent + '%' }" />
    </div>
    <p v-if="updateStatus" class="status" :class="{ error: failed }">{{ updateStatus }}</p>

    <p class="fallback">
      Not working?
      <span class="link" @click="openDownloadPage">Download the latest version</span>
    </p>
  </div>
</template>

<style scoped>
.update-root {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 8vh 24px 0;
  text-align: center;
}
.logo {
  height: 80px;
  margin-bottom: 1rem;
}
h1 {
  font-size: 18px;
  margin: 0 0 8px;
}
.message {
  color: var(--text-dim);
  max-width: 420px;
  margin: 0 0 16px;
}
.primary {
  padding: 9px 20px;
  border-radius: 6px;
  background: linear-gradient(180deg, var(--accent) 0%, var(--accent-soft) 100%);
  color: #fff;
  border: 1px solid var(--accent);
  font-size: 13px;
  font-weight: 600;
  box-shadow: 0 4px 20px var(--accent-glow);
  cursor: pointer;
}
.primary:disabled {
  opacity: 0.6;
  cursor: default;
}
.bar {
  width: 260px;
  height: 4px;
  margin-top: 14px;
  border-radius: 2px;
  background: var(--surface-2);
  overflow: hidden;
}
.fill {
  height: 100%;
  background: var(--accent);
  transition: width 0.2s;
}
.status {
  font-size: 12px;
  color: var(--text-dim);
}
.status.error {
  color: #ff6b6b;
}
.fallback {
  margin-top: 24px;
  font-size: 12px;
  color: var(--text-mute);
}
.link {
  color: var(--accent);
  cursor: pointer;
  text-decoration: underline;
}
</style>
