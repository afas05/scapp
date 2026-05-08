<script setup lang="ts">
import { ref, onBeforeUnmount } from "vue";
import { open } from '@tauri-apps/plugin-shell';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { useUserStore } from "./stores/userStore";
// @ts-ignore — JS composable, no .d.ts
import { useMediaSoup } from "./composables/useMediaSoup.js";
// @ts-ignore — JS module, no .d.ts
import { loadConfig } from "./config.js";
import WindowPicker from "./components/WindowPicker.vue";

const showPicker = ref(false);
const connectionState = ref(false);
const connectProcess = ref(false);
const producerId = ref('');
const userSession = ref('');
const viewerCount = ref(0);
const errorMessage = ref('');
const updateStatus = ref('');
const updateBusy = ref(false);

const userStore = useUserStore();
const mediaSoup = useMediaSoup();

mediaSoup.on('producerCreated', (id: string, session: string) => {
  console.log('[MediaSoup] Producer created:', id, 'session:', session);
  producerId.value = id;
  userSession.value = session;
  connectionState.value = true;
  connectProcess.value = false;
});

mediaSoup.on('producerClosed', () => {
  console.log('[MediaSoup] Producer closed');
  connectionState.value = false;
  producerId.value = '';
  userSession.value = '';
});

mediaSoup.on('viewerCount', (count: number) => {
  viewerCount.value = count;
});

mediaSoup.on('streamEnded', () => {
  console.log('[MediaSoup] Stream ended by server');
  connectionState.value = false;
});

function startStream() {
  showPicker.value = true;
}

async function doStartStream({ windowHandle, processPid }: { windowHandle: number; processPid: number }) {
  errorMessage.value = '';
  connectProcess.value = true;
  try {
    const config = await loadConfig();
    console.log('[MediaSoup] Connecting to SFU:', config.sfuWsUrl);
    await mediaSoup.init(userStore.sessionHash, config);
    await mediaSoup.startSharing(windowHandle, processPid);
  } catch (err: any) {
    console.error('[MediaSoup] Failed to start stream:', err);
    errorMessage.value = `Failed to start stream: ${err?.message ?? err}`;
    connectProcess.value = false;
    try { await mediaSoup.destroy(); } catch {}
  }
}

async function stopStream() {
  try {
    await mediaSoup.destroy();
  } catch (err) {
    console.error('[MediaSoup] Error during teardown:', err);
  }
  connectionState.value = false;
  connectProcess.value = false;
  producerId.value = '';
  userSession.value = '';
}

function openStream() {
  open('https://streamsnipe.live?streamId=' + producerId.value);
}

async function checkForUpdate() {
  if (updateBusy.value) return;
  updateBusy.value = true;
  updateStatus.value = 'Checking for updates...';
  try {
    const update = await check();
    if (!update) {
      updateStatus.value = 'You are on the latest version.';
      return;
    }
    updateStatus.value = `Downloading ${update.version}...`;
    await update.downloadAndInstall();
    updateStatus.value = 'Installed. Restarting...';
    await relaunch();
  } catch (err: any) {
    console.error('[Updater] Failed:', err);
    updateStatus.value = `Update failed: ${err?.message ?? err}`;
  } finally {
    updateBusy.value = false;
  }
}

onBeforeUnmount(() => {
  if (connectionState.value || connectProcess.value) {
    mediaSoup.destroy().catch(() => {});
  }
});
</script>

<template>
  <div>
    <div v-if="errorMessage" class="error-message">
      {{ errorMessage }}
    </div>
    <div v-if="!connectionState && !connectProcess">
      <button class="stream-button" @click="startStream">Stream</button>
    </div>
    <div v-if="connectionState">
      <button id="stop-stream" class="stream-button" @click="stopStream">Stop stream</button>
    </div>
    <div v-if="connectProcess && !connectionState" class="text-white">
      Connecting to SFU...
    </div>
    <div v-if="connectionState" class="text-white">
      Streaming &mdash; viewers: {{ viewerCount }}
      <br />
      <a @click.prevent="openStream" :href="'https://streamsnipe.live?streamId=' + producerId">Open stream</a>
    </div>
    <div class="update-row">
      <button class="update-button" @click="checkForUpdate" :disabled="updateBusy">
        Check for updates
      </button>
      <span v-if="updateStatus" class="update-status">{{ updateStatus }}</span>
    </div>

    <WindowPicker v-model="showPicker" @selected="doStartStream" />
  </div>
</template>

<style scoped>
.text-white {
  color: white;
}
.error-message {
  color: #ff6b6b;
  background-color: rgba(255, 107, 107, 0.1);
  border: 1px solid #ff6b6b;
  border-radius: 8px;
  padding: 1rem;
  margin-bottom: 1rem;
  font-size: 0.9rem;
}
.stream-button {
  border-radius: 9999px;
  background-color: #4c036c;
  font-size: 1.5rem;
  padding-left: 3rem;
  padding-right: 3rem;
  color: white;
}
.stream-button:hover {
  background-color: #5c037c;
  border: 1px solid #5c037c;
  font-size: 1.6rem;
}
#stop-stream {
  background-color: #6a0e01;
}
#stop-stream:hover {
  background-color: #7a0e01;
  border: 1px solid #7a0e01;
}
.update-row {
  margin-top: 1.5rem;
  display: flex;
  align-items: center;
  gap: 0.75rem;
}
.update-button {
  border-radius: 9999px;
  background-color: transparent;
  border: 1px solid #888;
  color: #ccc;
  font-size: 0.85rem;
  padding: 0.4rem 1rem;
  cursor: pointer;
}
.update-button:hover:not(:disabled) {
  border-color: #ccc;
  color: white;
}
.update-button:disabled {
  opacity: 0.5;
  cursor: default;
}
.update-status {
  color: #ccc;
  font-size: 0.85rem;
}
</style>
