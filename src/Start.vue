<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, computed, watch } from 'vue';
import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { useRouter } from 'vue-router';
import { useUserStore } from './stores/userStore';
// @ts-ignore — JS composable, no .d.ts
import { useMediaSoup } from './composables/useMediaSoup.js';
// @ts-ignore — JS module, no .d.ts
import { loadConfig } from './config.js';

import { openPath } from '@tauri-apps/plugin-opener';
import { join as pathJoin } from '@tauri-apps/api/path';
import IdleHome, { type IdleSource } from './components/idle/IdleHome.vue';
import LiveDashboard from './components/live/LiveDashboard.vue';
import Connecting from './components/shared/Connecting.vue';
import WindowPicker from './components/WindowPicker.vue';
import SettingsModal from './components/shared/SettingsModal.vue';
import { useSettingsStore } from './stores/settingsStore';

interface WindowInfo {
  id: number;
  pid: number;
  title: string;
  process_name: string;
  thumbnail: string;
}

interface MonitorInfo {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
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
const switchingSourceId = ref<number | null>(null);
const cameraOn = ref<boolean>(false);
const micOn = ref<boolean>(true);
const availableMics = ref<MicDevice[]>([]);
const micDevice = ref<string>('');
const camDevice = ref<string>('Logitech C920');

const producerId = ref<string>('');
const userSession = ref<string>('');
const viewerCount = ref<number>(0);
const errorMessage = ref<string>('');
// Non-fatal: the stream is live, but with less audio than was asked for.
const noticeMessage = ref<string>('');
const showPicker = ref<boolean>(false);
const showSettings = ref<boolean>(false);
const settingsStore = useSettingsStore();
const visibility = computed(() => settingsStore.visibility);
const lastSession = ref<{ duration: string; peakViewers: number } | null>(null);
const streamStartedAt = ref<number>(0);
const peakViewers = ref<number>(0);
const updateStatus = ref<string>('');
const updateBusy = ref<boolean>(false);
const appVersion = ref<string>('');

const previewFrame = ref<string>('');
let previewTimer: number | null = null;
let previewBusy = false;

async function refreshPreview() {
  if (previewBusy) return;
  previewBusy = true;
  try {
    const frame = await invoke<string>('get_preview_frame');
    if (frame) previewFrame.value = frame;
  } catch {}
  previewBusy = false;
}

function startPreviewTimer() {
  stopPreviewTimer();
  previewTimer = window.setInterval(refreshPreview, 66);
}

function stopPreviewTimer() {
  if (previewTimer != null) {
    clearInterval(previewTimer);
    previewTimer = null;
  }
  previewFrame.value = '';
}

watch(phase, (p) => {
  if (p === 'live') startPreviewTimer();
  else stopPreviewTimer();
});

const FALLBACK_SCREEN: IdleSource = {
  id: 0,
  pid: 0,
  title: 'Entire Screen',
  processName: 'Primary monitor',
  kind: 'desktop',
  monitorIndex: 0,
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

async function loadMonitorSources(): Promise<IdleSource[]> {
  try {
    const monitors = await invoke<MonitorInfo[]>('list_monitors');
    if (monitors.length === 0) return [FALLBACK_SCREEN];
    return monitors.map((m, i) => ({
      id: -(i + 1),
      pid: 0,
      title: monitors.length === 1 ? 'Entire Screen' : `${m.name}`,
      processName: `${m.width}×${m.height}`,
      thumbnail: m.thumbnail || undefined,
      kind: 'desktop' as const,
      monitorIndex: i,
    }));
  } catch (e: any) {
    console.warn('[Monitors] list_monitors failed:', e);
    return [FALLBACK_SCREEN];
  }
}

async function loadSources() {
  const monitorSources = await loadMonitorSources();
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
    sources.value = [...monitorSources, ...mapped];
    if (!sources.value.some(s => s.id === selectedId.value)) {
      selectedId.value = sources.value[0].id;
    }
  } catch (e: any) {
    console.warn('[Sources] list_windows failed:', e);
    sources.value = monitorSources;
  }
}

const selectedSource = computed(() =>
  sources.value.find(s => s.id === selectedId.value) || sources.value[0]
);

const shareUrl = computed(() => {
  if (producerId.value) return `streamsnipe.live?streamId=${producerId.value}`;
  return 'streamsnipe.live';
});

// Fire-and-forget Tauri invoke — never let presence/etc. failures break streaming.
async function safeInvoke(cmd: string, args?: Record<string, unknown>) {
  try {
    await invoke(cmd, args);
  } catch (err) {
    console.warn(`[invoke] ${cmd} failed:`, err);
  }
}

mediaSoup.on('producerCreated', (id: string, session: string) => {
  console.log('[MediaSoup] Producer created:', id, 'session:', session);
  producerId.value = id;
  userSession.value = session;
  phase.value = 'live';
  streamStartedAt.value = Date.now();
  peakViewers.value = 0;
  safeInvoke('discord_set_streaming', { producerId: id });
});

mediaSoup.on('producerClosed', () => {
  console.log('[MediaSoup] Producer closed');
  finalizeSession();
  phase.value = 'idle';
  noticeMessage.value = '';
  producerId.value = '';
  userSession.value = '';
  safeInvoke('discord_clear');
});

mediaSoup.on('viewerCount', (count: number) => {
  viewerCount.value = count;
  if (count > peakViewers.value) peakViewers.value = count;
});

mediaSoup.on('streamEnded', () => {
  console.log('[MediaSoup] Stream ended by server');
  finalizeSession();
  phase.value = 'idle';
  noticeMessage.value = '';
  safeInvoke('discord_clear');
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
  noticeMessage.value = '';
  phase.value = 'connecting';
  try {
    const config = await loadConfig();
    console.log('[MediaSoup] Connecting to SFU:', config.sfuWsUrl);
    await mediaSoup.init(userStore.sessionHash, config);
    const src = selectedSource.value;
    const status = await mediaSoup.startSharing(
      src.monitorIndex !== undefined ? 0 : src.id,
      src.pid,
      micOn.value,
      micDevice.value || null,
      src.monitorIndex ?? null,
      visibility.value,
    );
    // The pipeline degrades instead of failing when an audio device won't
    // open; say so rather than letting the user find out from their viewers.
    if (status?.notice) {
      console.warn('[MediaSoup] Started with reduced audio:', status);
      noticeMessage.value = status.notice;
    }
    if (status?.mic === 'failed') micOn.value = false;
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

// Swap the live capture to another source. Re-targets GStreamer's head source
// element only — the stream/producer keeps running — so viewers see the new
// source without a reconnect. Mirrors the window/monitor mapping in startStream().
async function onLiveSelectSource(id: number) {
  if (id === selectedId.value || switchingSourceId.value !== null) return;
  const src = sources.value.find(s => s.id === id);
  if (!src) return;
  switchingSourceId.value = id;
  try {
    await mediaSoup.switchSource(
      src.monitorIndex !== undefined ? 0 : src.id,
      src.monitorIndex ?? null,
    );
    selectedId.value = id;
  } catch (err: any) {
    console.error('[Source] switch failed:', err);
    errorMessage.value = `Failed to switch source: ${err?.message ?? err}`;
  } finally {
    switchingSourceId.value = null;
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
  safeInvoke('discord_clear');
}

function openStream() {
  if (producerId.value) {
    open('https://streamsnipe.live?streamId=' + producerId.value);
  }
}

async function openRecordingsFolder() {
  const path = settingsStore.recordingPath;
  if (path) {
    try {
      await openPath(path);
    } catch (err) {
      console.warn('[Recordings] Failed to open folder:', err);
    }
  }
}

async function onStartRecording() {
  const dir = settingsStore.recordingPath;
  if (!dir) {
    console.warn('[Recording] No recording path configured');
    return;
  }
  const timestamp = new Date().toISOString().replace(/[:.]/g, '-').replace('T', '_').slice(0, 19);
  const filename = `recording_${timestamp}.mp4`;
  const filePath = await pathJoin(dir, filename);
  // Free tier (plan 1) always gets the watermark; paid tier gets it unless the
  // user opted out in Settings.
  const watermark = userStore.plan === 1 ? true : !settingsStore.hideWatermark;
  try {
    await invoke('start_recording', { path: filePath, watermark });
    console.log('[Recording] Started:', filePath, 'watermark=', watermark);
  } catch (err: any) {
    console.error('[Recording] Failed to start:', err);
    errorMessage.value = `Recording failed: ${err?.message ?? err}`;
  }
}

async function onStopRecording() {
  try {
    await invoke('stop_recording');
    console.log('[Recording] Stopped');
  } catch (err: any) {
    console.error('[Recording] Failed to stop:', err);
  }
}

async function onPickerSelected(payload: { windowHandle: number; processPid: number; monitorIndex?: number }) {
  if (payload.monitorIndex !== undefined) {
    const monitorId = -(payload.monitorIndex + 1);
    if (!sources.value.some(s => s.id === monitorId)) {
      await loadSources();
    }
    selectedId.value = monitorId;
    return;
  }
  if (!sources.value.some(s => s.id === payload.windowHandle)) {
    await loadSources();
  }
  selectedId.value = payload.windowHandle;
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
  getVersion().then(v => { appVersion.value = `v${v}`; }).catch(() => {});
});

onBeforeUnmount(() => {
  stopPreviewTimer();
  if (phase.value === 'live' || phase.value === 'connecting') {
    mediaSoup.destroy().catch(() => {});
  }
});
</script>

<template>
  <div class="start-shell">
    <div v-if="errorMessage" class="error-banner">{{ errorMessage }}</div>
    <div v-else-if="noticeMessage" class="notice-banner">
      <span>{{ noticeMessage }}</span>
      <button class="notice-dismiss" @click="noticeMessage = ''" aria-label="Dismiss">×</button>
    </div>

    <IdleHome
      v-if="phase === 'idle'"
      :app-version="appVersion"
      :sources="sources"
      :selected-id="selectedId"
      :camera-on="cameraOn"
      :mic-on="micOn"
      :mic-device="micDevice"
      :available-mics="availableMics"
      :cam-device="camDevice"
      :visibility="visibility"
      :last-session="lastSession"
      :update-status="updateStatus"
      @select="id => (selectedId = id)"
      @open-picker="showPicker = true"
      @toggle-mic="micOn = !micOn"
      @pick-mic="v => (micDevice = v)"
      @pick-visibility="v => settingsStore.setVisibility(v)"
      @start="startStream"
      @logout="onLogout"
      @check-update="checkForUpdate"
      @open-settings="showSettings = true"
      @open-recordings="openRecordingsFolder"
      @open-editor="router.push('/editor')"
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
      :preview-frame="previewFrame"
      :switching-id="switchingSourceId"
      @select="onLiveSelectSource"
      @toggle-mic="onLiveToggleMic"
      @stop="stopStream"
      @open-settings="showSettings = true"
      @start-recording="onStartRecording"
      @stop-recording="onStopRecording"
    />

    <Connecting v-if="phase === 'connecting'" />

    <WindowPicker v-model="showPicker" @selected="onPickerSelected" />
    <SettingsModal v-model="showSettings" />

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
/* Same slot as .error-banner, but amber: the stream is live, something is just
   missing from it. Dismissible, since it isn't blocking anything. */
.notice-banner {
  position: absolute;
  top: 8px;
  left: 14px;
  right: 14px;
  z-index: 40;
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  color: #f0b429;
  background-color: rgba(240, 180, 41, 0.1);
  border: 1px solid #f0b429;
  border-radius: 8px;
  padding: 0.6rem 0.8rem;
  font-size: 11px;
}
.notice-dismiss {
  margin-left: auto;
  flex: none;
  background: none;
  border: none;
  color: inherit;
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  padding: 0;
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
