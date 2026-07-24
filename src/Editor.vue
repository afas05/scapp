<script setup lang="ts">
import { ref, reactive, computed, onMounted, onBeforeUnmount } from 'vue';
import { useRouter } from 'vue-router';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { open as openUrl } from '@tauri-apps/plugin-shell';
import { join as pathJoin } from '@tauri-apps/api/path';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import Icon from './components/shared/Icon.vue';
import { useSettingsStore } from './stores/settingsStore';
import { useUserStore } from './stores/userStore';
import {
  useAutopostStore,
  type Privacy,
  type Service,
  type AutopostStatus,
} from './stores/autopostStore';
import { useHttp } from './composables/useHttp';

interface RecordingInfo {
  path: string;
  name: string;
  sizeBytes: number;
  modifiedMs: number;
  durationSecs: number;
}

interface Clip {
  id: string;
  start: number;
  end: number;
  cropOffset: number; // 0..1 horizontal window position
}

type ClipPhase = 'idle' | 'exporting' | 'uploading' | 'processing' | 'done' | 'failed';
interface ClipState {
  phase: ClipPhase;
  percent: number; // 0..1 export progress
  message: string;
  url?: string;
}

const router = useRouter();
const settingsStore = useSettingsStore();
const userStore = useUserStore();
const autopostStore = useAutopostStore();

const SERVICES: Service[] = ['youtube', 'tiktok', 'instagram'];
const PRIVACIES: Privacy[] = ['private', 'unlisted', 'public'];

// --- source / player state ---------------------------------------------------
const recordings = ref<RecordingInfo[]>([]);
const loadingList = ref(false);
const currentPath = ref('');
const videoSrc = ref('');
const videoEl = ref<HTMLVideoElement | null>(null);
const stageEl = ref<HTMLElement | null>(null);
const duration = ref(0);
const currentTime = ref(0);
const playing = ref(false);
const videoAspect = ref(16 / 9);
const error = ref('');

const currentName = computed(() => {
  const p = currentPath.value;
  if (!p) return '';
  const parts = p.split(/[/\\]/);
  return parts[parts.length - 1];
});

// --- clips -------------------------------------------------------------------
const clips = ref<Clip[]>([]);
const selectedClipId = ref<string | null>(null);
const inPoint = ref<number | null>(null);
const pendingCropOffset = ref(0.5);

const selectedClip = computed(() =>
  clips.value.find(c => c.id === selectedClipId.value) || null
);
const activeCropOffset = computed(() =>
  selectedClip.value ? selectedClip.value.cropOffset : pendingCropOffset.value
);

// Fraction of the (landscape) video width kept by the 9:16 crop band.
const bandWidthFraction = computed(() =>
  Math.min(1, (9 / 16) / videoAspect.value)
);
const cropBandStyle = computed(() => {
  const w = bandWidthFraction.value;
  const left = activeCropOffset.value * (1 - w);
  return { left: `${left * 100}%`, width: `${w * 100}%` };
});

function fmtTime(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) secs = 0;
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  const cs = Math.floor((secs - Math.floor(secs)) * 100);
  return `${m}:${s.toString().padStart(2, '0')}.${cs.toString().padStart(2, '0')}`;
}
function fmtDuration(secs: number): string {
  if (!secs) return '—';
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}
function fmtSize(bytes: number): string {
  if (bytes > 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

// --- load recordings + files -------------------------------------------------
async function loadRecordings() {
  loadingList.value = true;
  try {
    const dir = settingsStore.recordingPath;
    if (!dir) { recordings.value = []; return; }
    recordings.value = await invoke<RecordingInfo[]>('list_recordings', { dir });
  } catch (e: any) {
    console.warn('[Editor] list_recordings failed:', e);
    recordings.value = [];
  } finally {
    loadingList.value = false;
  }
}

async function pickFile() {
  try {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: 'Video', extensions: ['mp4', 'mov', 'mkv', 'webm', 'm4v'] }],
    });
    if (typeof picked === 'string') loadVideo(picked);
  } catch (e: any) {
    console.warn('[Editor] file pick failed:', e);
  }
}

function loadVideo(path: string) {
  currentPath.value = path;
  videoSrc.value = convertFileSrc(path);
  clips.value = [];
  selectedClipId.value = null;
  inPoint.value = null;
  currentTime.value = 0;
  duration.value = 0;
  error.value = '';
}

function onLoadedMetadata() {
  const v = videoEl.value;
  if (!v) return;
  duration.value = v.duration || 0;
  if (v.videoWidth && v.videoHeight) videoAspect.value = v.videoWidth / v.videoHeight;
}
function onTimeUpdate() {
  const v = videoEl.value;
  if (v) currentTime.value = v.currentTime;
}
function togglePlay() {
  const v = videoEl.value;
  if (!v) return;
  if (v.paused) { v.play(); playing.value = true; }
  else { v.pause(); playing.value = false; }
}
function seekTo(t: number) {
  const v = videoEl.value;
  if (!v) return;
  v.currentTime = Math.max(0, Math.min(duration.value, t));
}

// --- timeline scrub ----------------------------------------------------------
function onTimelineClick(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  const rect = el.getBoundingClientRect();
  const frac = (e.clientX - rect.left) / rect.width;
  seekTo(frac * duration.value);
}

// --- clip in/out -------------------------------------------------------------
function markIn() {
  inPoint.value = currentTime.value;
}
function markOut() {
  if (inPoint.value == null) { error.value = 'Set a start point first (Mark In).'; return; }
  const start = Math.min(inPoint.value, currentTime.value);
  const end = Math.max(inPoint.value, currentTime.value);
  if (end - start < 0.2) { error.value = 'Clip is too short.'; return; }
  const clip: Clip = { id: `clip_${Date.now()}`, start, end, cropOffset: pendingCropOffset.value };
  clips.value.push(clip);
  selectedClipId.value = clip.id;
  inPoint.value = null;
  error.value = '';
}
function removeClip(id: string) {
  clips.value = clips.value.filter(c => c.id !== id);
  if (selectedClipId.value === id) selectedClipId.value = null;
  delete clipStates[id];
}
function selectClip(id: string) {
  selectedClipId.value = id;
  const c = clips.value.find(x => x.id === id);
  if (c) seekTo(c.start);
}
function setCrop(v: number) {
  const clamped = Math.max(0, Math.min(1, v));
  if (selectedClip.value) selectedClip.value.cropOffset = clamped;
  else pendingCropOffset.value = clamped;
}

// --- crop overlay drag -------------------------------------------------------
let dragging = false;
function onCropPointerDown(e: PointerEvent) {
  dragging = true;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
  onCropPointerMove(e);
}
function onCropPointerMove(e: PointerEvent) {
  if (!dragging) return;
  const stage = stageEl.value;
  if (!stage) return;
  const rect = stage.getBoundingClientRect();
  const w = bandWidthFraction.value;
  const travel = rect.width * (1 - w);
  if (travel <= 0) return;
  // Center the band on the pointer, then convert to a 0..1 offset.
  const centerX = e.clientX - rect.left;
  const leftPx = centerX - (rect.width * w) / 2;
  setCrop(leftPx / travel);
}
function onCropPointerUp() { dragging = false; }

// --- autopost modal ----------------------------------------------------------
const showPost = ref(false);
const status = ref<AutopostStatus | null>(null);
const statusLoading = ref(false);
const form = reactive({
  title: '',
  description: '',
  tags: '',
  privacy: 'private' as Privacy,
  services: ['youtube'] as Service[],
});
const posting = ref(false);
const clipStates = reactive<Record<string, ClipState>>({});

const selectedServices = computed(() => form.services);
function toggleService(s: Service) {
  const i = form.services.indexOf(s);
  if (i >= 0) form.services.splice(i, 1);
  else form.services.push(s);
}
function providerConnected(s: Service): boolean {
  return !!status.value?.providers?.[s]?.connected;
}
function connectUrl(s: Service): string | null {
  return status.value?.providers?.[s]?.connect_url ?? null;
}

async function openPostModal() {
  if (!clips.value.length) { error.value = 'Add at least one clip first.'; return; }
  await autopostStore.hydrate();
  form.privacy = autopostStore.lastPrivacy;
  form.services = [...(autopostStore.lastServices.length ? autopostStore.lastServices : autopostStore.defaultServices)];
  form.description = autopostStore.lastDescription;
  if (!form.title) form.title = currentName.value.replace(/\.[^.]+$/, '');
  showPost.value = true;
  refreshStatus();
  autopostStore.fetchServerSettings();
}

async function refreshStatus() {
  statusLoading.value = true;
  try {
    const res = await useHttp('GET', 'autopost/status', undefined, userStore.sessionHash);
    if (res.ok) status.value = await res.json();
  } catch (e: any) {
    console.warn('[Editor] status failed:', e);
  } finally {
    statusLoading.value = false;
  }
}

function connect(s: Service) {
  const url = connectUrl(s);
  if (url) openUrl(url);
}

const canSubmit = computed(() => {
  if (posting.value) return false;
  if (!form.title.trim() || form.title.length > 100) return false;
  if (!form.services.length) return false;
  // Every chosen service must be connected.
  return form.services.every(s => providerConnected(s));
});

function sleep(ms: number) { return new Promise(r => setTimeout(r, ms)); }

async function submitAll() {
  if (!canSubmit.value) return;
  posting.value = true;
  error.value = '';
  await autopostStore.persistLast(form.privacy, [...form.services], form.description);

  const tags = form.tags.split(',').map(t => t.trim()).filter(Boolean).slice(0, 30);
  const watermark = userStore.plan === 1 ? true : !settingsStore.hideWatermark;
  const clipsDir = await pathJoin(settingsStore.recordingPath, 'clips');
  const base = currentName.value.replace(/\.[^.]+$/, '') || 'clip';

  for (let i = 0; i < clips.value.length; i++) {
    const clip = clips.value[i];
    const st = (clipStates[clip.id] = reactive<ClipState>({ phase: 'exporting', percent: 0, message: 'Exporting…' }));
    const output = await pathJoin(clipsDir, `${base}_clip${i + 1}_${Date.now()}.mp4`);
    try {
      await invoke('export_clip', {
        clipId: clip.id,
        input: currentPath.value,
        output,
        startSecs: clip.start,
        endSecs: clip.end,
        cropOffset: clip.cropOffset,
        watermark,
      });
      st.phase = 'uploading';
      st.percent = 1;
      st.message = 'Uploading…';

      const title = clips.value.length > 1 ? `${form.title} (${i + 1})` : form.title;
      const resp = await invoke<{ status: number; body: any }>('autopost_submit', {
        token: userStore.sessionHash,
        filePath: output,
        title,
        description: form.description || null,
        tags,
        privacy: form.privacy,
        services: [...form.services],
        publishAt: null,
      });

      if (resp.status === 202 && resp.body?.post_id) {
        st.phase = 'processing';
        st.message = 'Processing on server…';
        await pollPost(resp.body.post_id, st);
      } else if (resp.status === 409) {
        const missing = (resp.body?.missing ?? []).map((m: any) => m.provider).join(', ');
        st.phase = 'failed';
        st.message = `Not connected: ${missing || 'service'}`;
      } else {
        st.phase = 'failed';
        st.message = resp.body?.message || `Upload rejected (${resp.status})`;
      }
    } catch (e: any) {
      st.phase = 'failed';
      st.message = `${e?.message ?? e}`;
    }
  }
  posting.value = false;
}

async function pollPost(postId: number, st: ClipState) {
  for (let attempt = 0; attempt < 120; attempt++) {
    await sleep(2500);
    try {
      const res = await useHttp('GET', `autopost/posts/${postId}`, undefined, userStore.sessionHash);
      if (!res.ok) continue;
      const data = await res.json();
      if (data.state === 'done') {
        st.phase = 'done';
        const done = (data.targets ?? []).find((t: any) => t.remote_url);
        st.url = done?.remote_url;
        st.message = 'Published';
        return;
      }
      if (data.state === 'failed') {
        st.phase = 'failed';
        const failed = (data.targets ?? []).find((t: any) => t.error);
        st.message = failed?.error || 'Server processing failed';
        return;
      }
    } catch (e) {
      // transient — keep polling
    }
  }
  st.phase = 'failed';
  st.message = 'Timed out waiting for server';
}

// --- export progress events --------------------------------------------------
let unlisten: UnlistenFn | null = null;

// --- window sizing -----------------------------------------------------------
async function back() {
  router.push('/start');
}

onMounted(async () => {
  try {
    const win = getCurrentWindow();
    await win.setSize(new LogicalSize(1120, 720));
    await win.center();
  } catch (e) { console.warn('[Editor] resize failed:', e); }

  unlisten = await listen<{ clipId: string; percent: number }>('clip-export-progress', (ev) => {
    const st = clipStates[ev.payload.clipId];
    if (st && st.phase === 'exporting') st.percent = ev.payload.percent;
  });

  loadRecordings();
});

onBeforeUnmount(async () => {
  if (unlisten) { unlisten(); unlisten = null; }
  try {
    const win = getCurrentWindow();
    await win.setSize(new LogicalSize(800, 600));
    await win.center();
  } catch (e) { /* ignore */ }
});
</script>

<template>
  <div class="editor-root">
    <header class="ed-header">
      <button type="button" class="ghost" @click="back">
        <Icon name="chev" :size="12" style="transform: rotate(180deg)" /> Back
      </button>
      <div class="ed-title">
        <Icon name="scissors" :size="13" />
        <span>Clip Studio</span>
        <span v-if="currentName" class="ed-file ws-mono">· {{ currentName }}</span>
      </div>
      <div class="ed-spacer" />
    </header>

    <div v-if="error" class="ed-error">{{ error }}</div>

    <div class="ed-body">
      <!-- Sources -->
      <aside class="ed-sources ws-scroll">
        <div class="section-label">Recordings</div>
        <button type="button" class="pick-btn" @click="pickFile">
          <Icon name="folder" :size="12" /> Open a video file…
        </button>
        <div v-if="loadingList" class="hint">Loading…</div>
        <div v-else-if="!recordings.length" class="hint">No recordings in your save folder yet.</div>
        <button
          v-for="r in recordings"
          :key="r.path"
          type="button"
          class="rec-item"
          :class="{ active: r.path === currentPath }"
          @click="loadVideo(r.path)"
        >
          <Icon name="film" :size="14" />
          <div class="rec-meta">
            <div class="rec-name">{{ r.name }}</div>
            <div class="rec-sub ws-mono">{{ fmtDuration(r.durationSecs) }} · {{ fmtSize(r.sizeBytes) }}</div>
          </div>
        </button>
      </aside>

      <!-- Editor -->
      <main class="ed-main">
        <div v-if="!currentPath" class="empty-stage">
          <Icon name="film" :size="32" />
          <p>Select a recording or open a file to start cutting shorts.</p>
        </div>

        <template v-else>
          <div class="stage-wrap">
            <div ref="stageEl" class="stage" :style="{ aspectRatio: videoAspect }">
              <video
                ref="videoEl"
                :src="videoSrc"
                class="video"
                @loadedmetadata="onLoadedMetadata"
                @timeupdate="onTimeUpdate"
                @play="playing = true"
                @pause="playing = false"
                @click="togglePlay"
              />
              <!-- 9:16 crop band -->
              <div class="crop-mask left" :style="{ width: cropBandStyle.left }" />
              <div
                class="crop-band"
                :style="cropBandStyle"
                @pointerdown="onCropPointerDown"
                @pointermove="onCropPointerMove"
                @pointerup="onCropPointerUp"
              >
                <div class="crop-grip" />
                <span class="crop-tag ws-mono">9:16</span>
              </div>
              <div class="badge tl ws-mono">{{ fmtTime(currentTime) }} / {{ fmtTime(duration) }}</div>
            </div>
          </div>

          <!-- transport + timeline -->
          <div class="transport">
            <button type="button" class="round" @click="togglePlay">
              <Icon :name="playing ? 'stop' : 'play'" :size="14" />
            </button>
            <div class="timeline" @click="onTimelineClick">
              <div
                v-for="c in clips"
                :key="c.id"
                class="tl-clip"
                :class="{ sel: c.id === selectedClipId }"
                :style="{ left: `${(c.start / (duration || 1)) * 100}%`, width: `${((c.end - c.start) / (duration || 1)) * 100}%` }"
              />
              <div
                v-if="inPoint != null"
                class="tl-in"
                :style="{ left: `${(inPoint / (duration || 1)) * 100}%` }"
              />
              <div class="tl-playhead" :style="{ left: `${(currentTime / (duration || 1)) * 100}%` }" />
            </div>
          </div>

          <div class="tools">
            <button type="button" class="ghost" @click="markIn">
              <Icon name="chev" :size="11" /> Mark In
            </button>
            <button type="button" class="ghost" @click="markOut">
              Mark Out <Icon name="chev" :size="11" style="transform: rotate(180deg)" />
            </button>
            <div class="crop-ctl">
              <span class="lbl">Crop</span>
              <input
                type="range" min="0" max="1" step="0.01"
                :value="activeCropOffset"
                @input="setCrop(parseFloat(($event.target as HTMLInputElement).value))"
              />
            </div>
            <div class="tool-spacer" />
            <button
              type="button" class="primary-sm"
              :disabled="!clips.length"
              @click="openPostModal"
            >
              <Icon name="upload" :size="12" /> Export &amp; Post ({{ clips.length }})
            </button>
          </div>

          <!-- clips list -->
          <div class="clips ws-scroll">
            <div v-if="!clips.length" class="hint center">
              Use <b>Mark In</b> / <b>Mark Out</b> to cut a clip, then drag the 9:16 band to frame it.
            </div>
            <div
              v-for="(c, i) in clips"
              :key="c.id"
              class="clip-row"
              :class="{ sel: c.id === selectedClipId }"
              @click="selectClip(c.id)"
            >
              <span class="clip-idx ws-mono">#{{ i + 1 }}</span>
              <span class="clip-time ws-mono">{{ fmtTime(c.start) }} → {{ fmtTime(c.end) }}</span>
              <span class="clip-dur ws-mono">{{ (c.end - c.start).toFixed(1) }}s</span>
              <span v-if="clipStates[c.id]" class="clip-state" :class="clipStates[c.id].phase">
                <template v-if="clipStates[c.id].phase === 'exporting'">
                  Exporting {{ Math.round(clipStates[c.id].percent * 100) }}%
                </template>
                <template v-else>{{ clipStates[c.id].message }}</template>
                <a v-if="clipStates[c.id].url" class="ws-link" @click.stop="openUrl(clipStates[c.id].url!)">↗</a>
              </span>
              <span class="clip-spacer" />
              <button type="button" class="icon-btn" @click.stop="removeClip(c.id)">
                <Icon name="trash" :size="12" />
              </button>
            </div>
          </div>
        </template>
      </main>
    </div>

    <!-- Autopost modal -->
    <Teleport to="body">
      <div v-if="showPost" class="modal-overlay" @click.self="showPost = false">
        <div class="modal">
          <div class="modal-header">
            <h2>Post {{ clips.length }} short{{ clips.length > 1 ? 's' : '' }}</h2>
            <button type="button" class="icon-btn" @click="showPost = false"><Icon name="x" :size="14" /></button>
          </div>

          <div class="modal-body ws-scroll">
            <label class="fld">
              <span class="fld-label">Title <span class="req">*</span></span>
              <input class="app-input" v-model="form.title" maxlength="100" placeholder="My epic clip" />
            </label>
            <label class="fld">
              <span class="fld-label">Description</span>
              <textarea class="app-input area" v-model="form.description" maxlength="5000" rows="3" />
            </label>
            <label class="fld">
              <span class="fld-label">Tags <span class="hint-inline">comma separated</span></span>
              <input class="app-input" v-model="form.tags" placeholder="gaming, funny, clip" />
            </label>

            <div class="fld">
              <span class="fld-label">Privacy</span>
              <div class="seg">
                <button
                  v-for="p in PRIVACIES" :key="p"
                  type="button" class="seg-btn"
                  :class="{ on: form.privacy === p }"
                  @click="form.privacy = p"
                >{{ p }}</button>
              </div>
            </div>

            <div class="fld">
              <span class="fld-label">Post to</span>
              <div class="svc-list">
                <div
                  v-for="s in SERVICES" :key="s"
                  class="svc"
                  :class="{ on: selectedServices.includes(s), off: !providerConnected(s) }"
                >
                  <button type="button" class="svc-toggle" @click="toggleService(s)">
                    <span class="chk" :class="{ on: selectedServices.includes(s) }">
                      <Icon v-if="selectedServices.includes(s)" name="check" :size="10" />
                    </span>
                    <span class="svc-name">{{ s }}</span>
                  </button>
                  <span v-if="statusLoading" class="svc-status">…</span>
                  <span v-else-if="providerConnected(s)" class="svc-status ok">
                    <Icon name="check" :size="10" /> connected
                  </span>
                  <a v-else class="svc-status link" @click="connect(s)">
                    <Icon name="link" :size="10" /> connect on website ↗
                  </a>
                </div>
              </div>
            </div>
          </div>

          <div class="modal-footer">
            <button type="button" class="ghost" @click="showPost = false">Close</button>
            <button type="button" class="primary-sm" :disabled="!canSubmit" @click="submitAll">
              <Icon name="upload" :size="12" />
              {{ posting ? 'Posting…' : `Post ${clips.length} clip${clips.length > 1 ? 's' : ''}` }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.editor-root {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
}
.ed-header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--border-soft);
  flex-shrink: 0;
}
.ed-title { display: flex; align-items: center; gap: 7px; font-size: 14px; font-weight: 600; }
.ed-file { font-size: 10px; color: var(--text-mute); font-weight: 400; }
.ed-spacer, .ed-title { flex: 1; }
.ed-spacer { flex: 0 0 60px; }

.ed-error {
  margin: 6px 14px 0;
  color: #ff8080;
  background: rgba(255,107,107,0.1);
  border: 1px solid #ff6b6b;
  border-radius: 6px;
  padding: 6px 10px;
  font-size: 11px;
}

.ed-body { flex: 1; min-height: 0; display: flex; }

/* sources */
.ed-sources {
  width: 240px;
  flex-shrink: 0;
  border-right: 1px solid var(--border-soft);
  padding: 12px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.section-label {
  font-size: 9px; color: var(--text-mute); letter-spacing: 0.6px;
  text-transform: uppercase; font-weight: 500; margin-bottom: 2px;
}
.pick-btn {
  display: flex; align-items: center; gap: 6px;
  padding: 8px 10px; border-radius: 6px;
  border: 1px dashed var(--border); background: transparent;
  color: var(--text-dim); font-size: 11px; font-family: inherit; cursor: pointer;
  margin-bottom: 4px;
}
.pick-btn:hover { border-color: var(--accent); color: var(--text); }
.rec-item {
  display: flex; align-items: center; gap: 9px;
  padding: 8px; border-radius: 6px; background: var(--surface-1);
  border: 1px solid var(--border-soft); color: var(--text-dim);
  font-family: inherit; cursor: pointer; text-align: left; width: 100%;
}
.rec-item:hover { background: var(--surface-2); }
.rec-item.active { border-color: var(--accent); background: var(--surface-2); color: var(--text); }
.rec-meta { min-width: 0; flex: 1; }
.rec-name { font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rec-sub { font-size: 9px; color: var(--text-mute); margin-top: 2px; }
.hint { font-size: 11px; color: var(--text-mute); padding: 6px 2px; }
.hint.center { text-align: center; padding: 18px; }

/* main */
.ed-main { flex: 1; min-width: 0; display: flex; flex-direction: column; padding: 12px; gap: 10px; }
.empty-stage {
  flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 12px; color: var(--text-mute); font-size: 12px;
}
.stage-wrap { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; }
.stage {
  position: relative; max-height: 100%; max-width: 100%;
  background: #000; border-radius: 8px; overflow: hidden;
  border: 1px solid var(--border-soft);
  height: 100%;
}
.video { display: block; width: 100%; height: 100%; object-fit: contain; background: #000; }
.crop-mask {
  position: absolute; top: 0; bottom: 0; left: 0;
  background: rgba(0,0,0,0.55); pointer-events: none;
}
.crop-band {
  position: absolute; top: 0; bottom: 0;
  border: 1.5px solid var(--accent);
  box-shadow: 0 0 0 100vmax rgba(0,0,0,0.0);
  cursor: ew-resize; touch-action: none;
  background: rgba(155, 92, 246, 0.06);
}
.crop-grip {
  position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);
  width: 3px; height: 34px; border-radius: 2px; background: var(--accent);
  opacity: 0.8;
}
.crop-tag {
  position: absolute; bottom: 6px; left: 50%; transform: translateX(-50%);
  font-size: 9px; color: #fff; background: var(--accent-soft);
  padding: 1px 5px; border-radius: 3px;
}
.badge.tl {
  position: absolute; left: 8px; top: 8px;
  background: rgba(0,0,0,0.5); padding: 3px 7px; border-radius: 4px;
  font-size: 10px; color: rgba(255,255,255,0.85);
}

/* transport */
.transport { display: flex; align-items: center; gap: 10px; }
.round {
  width: 30px; height: 30px; border-radius: 50%; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
  background: var(--surface-2); color: var(--text); border: 1px solid var(--border);
  cursor: pointer;
}
.timeline {
  position: relative; flex: 1; height: 26px; border-radius: 6px;
  background: var(--surface-1); border: 1px solid var(--border-soft);
  cursor: pointer; overflow: hidden;
}
.tl-clip {
  position: absolute; top: 0; bottom: 0; background: var(--accent-soft);
  opacity: 0.5; border-left: 1px solid var(--accent); border-right: 1px solid var(--accent);
}
.tl-clip.sel { opacity: 0.8; }
.tl-in { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--warn); }
.tl-playhead { position: absolute; top: 0; bottom: 0; width: 2px; background: #fff; }

/* tools */
.tools { display: flex; align-items: center; gap: 8px; }
.crop-ctl { display: flex; align-items: center; gap: 6px; }
.crop-ctl .lbl { font-size: 10px; color: var(--text-mute); text-transform: uppercase; letter-spacing: 0.5px; }
.crop-ctl input[type=range] { width: 120px; accent-color: var(--accent); }
.tool-spacer { flex: 1; }

/* clips */
.clips {
  max-height: 150px; overflow-y: auto; display: flex; flex-direction: column; gap: 5px;
  border-top: 1px solid var(--border-soft); padding-top: 8px;
}
.clip-row {
  display: flex; align-items: center; gap: 10px;
  padding: 6px 8px; border-radius: 6px; background: var(--surface-1);
  border: 1px solid var(--border-soft); cursor: pointer; font-size: 11px;
}
.clip-row.sel { border-color: var(--accent); background: var(--surface-2); }
.clip-idx { color: var(--accent); }
.clip-time { color: var(--text-dim); }
.clip-dur { color: var(--text-mute); }
.clip-spacer { flex: 1; }
.clip-state { font-size: 10px; color: var(--text-dim); display: inline-flex; align-items: center; gap: 4px; }
.clip-state.done { color: var(--live); }
.clip-state.failed { color: #ff8080; }
.clip-state.processing, .clip-state.uploading, .clip-state.exporting { color: var(--warn); }
.icon-btn {
  background: transparent; border: none; color: var(--text-mute); cursor: pointer;
  display: inline-flex; padding: 3px; border-radius: 4px;
}
.icon-btn:hover { color: var(--text); background: var(--surface-3); }

/* shared buttons */
.ghost {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 6px 11px; border-radius: 6px; background: transparent;
  color: var(--text-dim); border: 1px solid var(--border);
  font-size: 11px; font-family: inherit; cursor: pointer;
}
.ghost:hover { color: var(--text); }
.primary-sm {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 7px 14px; border-radius: 6px;
  background: linear-gradient(180deg, var(--accent) 0%, var(--accent-soft) 100%);
  color: #fff; border: 1px solid var(--accent);
  font-size: 12px; font-weight: 600; font-family: inherit; cursor: pointer;
  box-shadow: 0 4px 20px var(--accent-glow), inset 0 1px 0 rgba(255,255,255,0.15);
}
.primary-sm:disabled { opacity: 0.45; cursor: not-allowed; box-shadow: none; }

/* modal */
.modal-overlay {
  position: fixed; inset: 0; background: rgba(0,0,0,0.75);
  display: flex; align-items: center; justify-content: center; z-index: 1000;
}
.modal {
  width: 460px; max-width: 92vw; max-height: 86vh; display: flex; flex-direction: column;
  background: var(--bg-deep); border: 1px solid var(--border-soft); border-radius: 12px;
}
.modal-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 16px; border-bottom: 1px solid var(--border-soft);
}
.modal-header h2 { font-size: 14px; font-weight: 600; }
.modal-body { padding: 14px 16px; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; }
.modal-footer {
  display: flex; justify-content: flex-end; gap: 8px;
  padding: 12px 16px; border-top: 1px solid var(--border-soft);
}
.fld { display: flex; flex-direction: column; gap: 4px; }
.fld-label { font-size: 11px; color: var(--text-dim); }
.req { color: var(--accent); }
.hint-inline { color: var(--text-mute); font-size: 9px; }
.app-input.area { resize: vertical; font-family: inherit; }
.seg { display: flex; gap: 6px; }
.seg-btn {
  flex: 1; padding: 7px; border-radius: 6px; text-transform: capitalize;
  background: var(--surface-2); color: var(--text-dim);
  border: 1px solid var(--border); font-size: 11px; font-family: inherit; cursor: pointer;
}
.seg-btn.on { border-color: var(--accent); color: var(--text); background: var(--surface-3); }
.svc-list { display: flex; flex-direction: column; gap: 6px; }
.svc {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 10px; border-radius: 6px; background: var(--surface-1);
  border: 1px solid var(--border-soft);
}
.svc.on { border-color: var(--accent); }
.svc-toggle {
  display: flex; align-items: center; gap: 8px;
  background: transparent; border: none; color: var(--text);
  font-family: inherit; font-size: 12px; cursor: pointer; text-transform: capitalize;
}
.chk {
  width: 16px; height: 16px; border-radius: 4px; border: 1.5px solid var(--border);
  display: flex; align-items: center; justify-content: center; color: #fff;
}
.chk.on { background: var(--accent); border-color: var(--accent); }
.svc-status { font-size: 10px; color: var(--text-mute); display: inline-flex; align-items: center; gap: 3px; }
.svc-status.ok { color: var(--live); }
.svc-status.link { color: var(--accent); cursor: pointer; }
</style>
