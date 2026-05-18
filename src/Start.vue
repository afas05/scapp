<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, computed } from 'vue';
import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { useRouter } from 'vue-router';
import { useUserStore } from './stores/userStore';
// @ts-ignore — JS composable, no .d.ts
import { useMediaSoup } from './composables/useMediaSoup.js';
// @ts-ignore — JS module, no .d.ts
import { loadConfig } from './config.js';

import IdleHome, { type IdleSource } from './components/idle/IdleHome.vue';
import LiveDashboard from './components/live/LiveDashboard.vue';
import Connecting from './components/shared/Connecting.vue';
import WindowPicker from './components/WindowPicker.vue';

interface WindowInfo {
  id: number;
  pid: number;
  title: string;
  process_name: string;
  thumbnail: string;
}

interface MicDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

const router = useRouter();
const userStore = useUserStore();
const mediaSoup = useMediaSoup();

type Phase = 'idle' | 'connecting' | 'live';
const phase = ref<Phase>('idle');

const sources = ref<IdleSource[]>([]);
const selectedId = ref<number>(0);
const cameraOn = ref<boolean>(true);
const micOn = ref<boolean>(true);
const availableMics = ref<MicDevice[]>([]);
const micDevice = ref<string>('');
const camDevice = ref<string>('Logitech C920');

const producerId = ref<string>('');
const userSession = ref<string>('');
const viewerCount = ref<number>(0);
const errorMessage = ref<string>('');
const showPicker = ref<boolean>(false);
const lastSession = ref<{ duration: string; peakViewers: number } | null>(null);
const streamStartedAt = ref<number>(0);
const peakViewers = ref<number>(0);
const updateStatus = ref<string>('');
const updateBusy = ref<boolean>(false);

const ENTIRE_SCREEN: IdleSource = {
  id: 0,
  pid: 0,
  title: 'Entire Screen',
  processName: 'Primary monitor',
  kind: 'desktop',
};

function kindFromProcess(name: string): IdleSource['kind'] {
  const n = name.toLowerCase();
  if (n.includes('chrome') || n.includes('firefox') || n.includes('edge') || n.includes('discord')) return 'browser';
  if (n.includes('code') || n.includes('rider') || n.includes('idea') || n.includes('explorer')) return 'app';
  return 'game';
}

async function loadMicDevices() {
  try {
    const list = await invoke<MicDevice[]>('list_audio_inputs');
    availableMics.value = list;
    if (!micDevice.value || !list.some(m => m.id === micDevice.value)) {
      const def = list.find(m => m.isDefault) ?? list[0];
      micDevice.value = def?.id ?? '';
    }
  } catch (e: any) {
    console.warn('[Mics] list_audio_inputs failed:', e);
    availableMics.value = [];
    micDevice.value = '';
  }
}

async function loadSources() {
  try {
    const list = await invoke<WindowInfo[]>('list_windows');
    const mapped: IdleSource[] = list.map(w => ({
      id: w.id,
      pid: w.pid,
      title: w.title,
      processName: w.process_name,
      thumbnail: w.thumbnail || undefined,
      kind: kindFromProcess(w.process_name),
    }));
    sources.value = [ENTIRE_SCREEN, ...mapped];
    if (!sources.value.some(s => s.id === selectedId.value)) {
      selectedId.value = sources.value[0].id;
    }
  } catch (e: any) {
    console.warn('[Sources] list_windows failed:', e);
    sources.value = [ENTIRE_SCREEN];
  }
}

const selectedSource = computed(() =>
  sources.value.find(s => s.id === selectedId.value) || sources.value[0]
);

const shareUrl = computed(() => {
  if (producerId.value) return `streamsnipe.live?streamId=${producerId.value}`;
  return 'streamsnipe.live';
});

mediaSoup.on('producerCreated', (id: string, session: string) => {
  console.log('[MediaSoup] Producer created:', id, 'session:', session);
  producerId.value = id;
  userSession.value = session;
  phase.value = 'live';
  streamStartedAt.value = Date.now();
  peakViewers.value = 0;
});

mediaSoup.on('producerClosed', () => {
  console.log('[MediaSoup] Producer closed');
  finalizeSession();
  phase.value = 'idle';
  producerId.value = '';
  userSession.value = '';
});

mediaSoup.on('viewerCount', (count: number) => {
  viewerCount.value = count;
  if (count > peakViewers.value) peakViewers.value = count;
});

mediaSoup.on('streamEnded', () => {
  console.log('[MediaSoup] Stream ended by server');
  finalizeSession();
  phase.value = 'idle';
});

function finalizeSession() {
  if (streamStartedAt.value === 0) return;
  const secs = Math.floor((Date.now() - streamStartedAt.value) / 1000);
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  const duration = m > 0 ? `${m}m` : `${s}s`;
  lastSession.value = { duration, peakViewers: peakViewers.value };
  streamStartedAt.value = 0;
}

async function startStream() {
  if (!selectedSource.value) return;
  errorMessage.value = '';
  phase.value = 'connecting';
  try {
    const config = await loadConfig();
    console.log('[MediaSoup] Connecting to SFU:', config.sfuWsUrl);
    await mediaSoup.init(userStore.sessionHash, config);
    await mediaSoup.startSharing(
      selectedSource.value.id,
      selectedSource.value.pid,
      micOn.value,
      micDevice.value || null,
    );
  } catch (err: any) {
    console.error('[MediaSoup] Failed to start stream:', err);
    errorMessage.value = `Failed to start stream: ${err?.message ?? err}`;
    phase.value = 'idle';
    try { await mediaSoup.destroy(); } catch {}
  }
}

async function onLiveToggleMic() {
  micOn.value = !micOn.value;
  try {
    await invoke('set_mic_muted', { muted: !micOn.value });
  } catch (err) {
    console.warn('[Mic] set_mic_muted failed:', err);
  }
}

async function stopStream() {
  try {
    await mediaSoup.destroy();
  } catch (err) {
    console.error('[MediaSoup] Error during teardown:', err);
  }
  finalizeSession();
  phase.value = 'idle';
  producerId.value = '';
  userSession.value = '';
}

function openStream() {
  if (producerId.value) {
    open('https://streamsnipe.live?streamId=' + producerId.value);
  }
}

async function onPickerSelected({ windowHandle }: { windowHandle: number; processPid: number }) {
  // Make sure that pick is also represented in our source list. If not present, refresh.
  if (!sources.value.some(s => s.id === windowHandle)) {
    await loadSources();
  }
  selectedId.value = windowHandle;
}

async function onLogout() {
  try {
    await userStore.logout();
  } catch (err) {
    console.error('[Logout] Failed to clear state:', err);
  }
  await router.replace('/login');
}

async function checkForUpdate() {
  if (updateBusy.value) return;
  updateBusy.value = true;
  updateStatus.value = 'Checking for updates…';
  try {
    const update = await check();
    if (!update) {
      updateStatus.value = 'You are on the latest version.';
      return;
    }
    updateStatus.value = `Downloading ${update.version}…`;
    await update.downloadAndInstall();
    updateStatus.value = 'Installed. Restarting…';
    await relaunch();
  } catch (err: any) {
    console.error('[Updater] Failed:', err);
    updateStatus.value = `Update failed: ${err?.message ?? err}`;
  } finally {
    updateBusy.value = false;
  }
}

onMounted(() => {
  loadSources();
  loadMicDevices();
});

onBeforeUnmount(() => {
  if (phase.value === 'live' || phase.value === 'connecting') {
    mediaSoup.destroy().catch(() => {});
  }
});
</script>

<template>
  <div class="start-shell">
    <div v-if="errorMessage" class="error-banner">{{ errorMessage }}</div>

    <IdleHome
      v-if="phase === 'idle'"
      :sources="sources"
      :selected-id="selectedId"
      :camera-on="cameraOn"
      :mic-on="micOn"
      :mic-device="micDevice"
      :available-mics="availableMics"
      :cam-device="camDevice"
      :last-session="lastSession"
      :update-status="updateStatus"
      @select="id => (selectedId = id)"
      @open-picker="showPicker = true"
      @toggle-camera="cameraOn = !cameraOn"
      @toggle-mic="micOn = !micOn"
      @pick-mic="v => (micDevice = v)"
      @pick-cam="v => (camDevice = v)"
      @start="startStream"
      @logout="onLogout"
      @check-update="checkForUpdate"
    />

    <LiveDashboard
      v-else-if="phase === 'live'"
      :sources="sources"
      :selected-id="selectedId"
      :camera-on="cameraOn"
      :mic-on="micOn"
      :mic-device="micDevice"
      :available-mics="availableMics"
      :viewer-count="viewerCount"
      :share-url="shareUrl"
      @select="id => (selectedId = id)"
      @toggle-camera="cameraOn = !cameraOn"
      @toggle-mic="onLiveToggleMic"
      @stop="stopStream"
    />

    <Connecting v-if="phase === 'connecting'" />

    <WindowPicker v-model="showPicker" @selected="onPickerSelected" />

    <a
      v-if="phase === 'live' && producerId"
      class="open-stream"
      @click.prevent="openStream"
      :href="'https://streamsnipe.live?streamId=' + producerId"
    >
      Open viewer ↗
    </a>
  </div>
</template>

<style scoped>
.start-shell {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
}
.error-banner {
  position: absolute;
  top: 8px;
  left: 14px;
  right: 14px;
  z-index: 40;
  color: #ff6b6b;
  background-color: rgba(255, 107, 107, 0.1);
  border: 1px solid #ff6b6b;
  border-radius: 8px;
  padding: 0.6rem 0.8rem;
  font-size: 11px;
}
.open-stream {
  position: absolute;
  right: 18px;
  bottom: 56px;
  font-size: 10px;
  color: var(--accent);
  text-decoration: none;
  cursor: pointer;
}
.open-stream:hover { text-decoration: underline; }
</style>
