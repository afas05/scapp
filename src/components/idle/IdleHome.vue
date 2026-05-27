<script setup lang="ts">
import { computed } from 'vue';
import Icon from '../shared/Icon.vue';
import StatusDot from '../shared/StatusDot.vue';
import MeterBars from '../shared/MeterBars.vue';
import ScreenPlaceholder from '../shared/ScreenPlaceholder.vue';
import CamPlaceholder from '../shared/CamPlaceholder.vue';
import SetupRow from './SetupRow.vue';
import SourceCard from './SourceCard.vue';
import DeviceSelect from './DeviceSelect.vue';
import { useMicLevel } from '../../composables/useMicLevel';

export interface IdleSource {
  id: number;
  pid: number;
  title: string;
  processName: string;
  thumbnail?: string;
  kind?: 'game' | 'browser' | 'app' | 'desktop';
  monitorIndex?: number;
}

export interface MicDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

const props = withDefaults(defineProps<{
  appVersion?: string;
  sfuLatencyMs?: number;
  sources: IdleSource[];
  selectedId: number;
  cameraOn: boolean;
  micOn: boolean;
  micDevice?: string;
  availableMics?: MicDevice[];
  camDevice?: string;
  lastSession?: { duration: string; peakViewers: number } | null;
  updateStatus?: string;
}>(), {
  appVersion: '',
  sfuLatencyMs: 28,
  micDevice: '',
  availableMics: () => [],
  camDevice: 'Logitech C920',
  lastSession: null,
  updateStatus: '',
});

const emit = defineEmits<{
  start: [];
  select: [id: number];
  openPicker: [];
  toggleCamera: [];
  toggleMic: [];
  pickMic: [device: string];
  pickCam: [device: string];
  logout: [];
  checkUpdate: [];
  openSettings: [];
  openRecordings: [];
}>();

const selectedMic = computed(() =>
  props.availableMics.find(m => m.id === props.micDevice)
);

const { level: micLevel, dbfs } = useMicLevel(
  () => props.micOn,
  () => selectedMic.value?.name,
);

const selectedSource = computed(() =>
  props.sources.find(s => s.id === props.selectedId) || props.sources[0]
);

const dbLabel = computed(() => {
  if (!props.micOn) return '—';
  if (!Number.isFinite(dbfs.value)) return '-∞dB';
  const v = Math.max(-60, Math.min(0, dbfs.value));
  return `${Math.round(v)}dB`;
});

const micOptions = computed(() => props.availableMics.map(m => m.name));
const micDisplayName = computed(() =>
  selectedMic.value?.name ?? (props.availableMics[0]?.name ?? '')
);
function onPickMicName(name: string) {
  const found = props.availableMics.find(m => m.name === name);
  if (found) emit('pickMic', found.id);
}

// up to 4 cards, plus a "more" tile
const quickSources = computed(() => props.sources.slice(0, 4));
</script>

<template>
  <div class="idle-root">
    <header class="header">
      <div class="brand">
        <span class="wordmark">StreamSnipe</span>
        <span class="ws-mono version">{{ appVersion }}</span>
      </div>
      <div class="head-right">
        <div class="sfu">
          <StatusDot color="var(--live)" :pulse="false" :size="6" />
          <span>Server ready</span>
          <span class="ws-mono ms">· {{ sfuLatencyMs }}ms</span>
        </div>
        <button type="button" class="ghost" @click="emit('logout')">
          <Icon name="logout" :size="11" />
          Logout
        </button>
      </div>
    </header>

    <div class="body">
      <div class="grid">
        <div class="preview">
          <ScreenPlaceholder
            v-if="selectedSource"
            :kind="selectedSource.kind"
            :label="selectedSource ? `${selectedSource.processName}` : ''"
          />
          <ScreenPlaceholder v-else kind="desktop" />

          <div class="badge tl">
            <Icon name="monitor" :size="10" />
            Capturing
          </div>
          <button type="button" class="badge tr clickable" @click="emit('openPicker')">
            <Icon name="swap" :size="10" />
            Change source
          </button>

          <div v-if="selectedSource" class="src-title">{{ selectedSource.title }}</div>
        </div>

        <div class="setup-col">
          <SetupRow
            icon="mic"
            label="Microphone"
            :on="micOn"
            toggleable
            @toggle="emit('toggleMic')"
          >
            <template #value>
              <DeviceSelect
                :value="micDisplayName"
                :options="micOptions"
                :disabled="!micOn || micOptions.length === 0"
                @change="onPickMicName"
              />
            </template>
            <div class="meter-row">
              <MeterBars :value="micOn ? micLevel : 0" :bars="14" :height="6" />
              <span class="ws-mono db">{{ dbLabel }}</span>
            </div>
          </SetupRow>

          <div class="disabled-feature">
            <SetupRow
              icon="cam"
              label="Webcam"
              :on="false"
            >
              <template #value>
                <span class="coming-soon">Coming soon</span>
              </template>
              <div class="cam-tile">
                <CamPlaceholder :on="false" />
              </div>
            </SetupRow>
          </div>

          <SetupRow
            icon="audio"
            label="System audio"
            subtext="Capture game"
            :on="true"
          />
        </div>

        <div class="quick-strip">
          <div class="section-label">Quick sources</div>
          <div class="cards-list">
            <SourceCard
              v-for="s in quickSources"
              :key="s.id"
              :source="{ id: s.id, kind: s.kind, title: s.title, process: s.processName, thumbnail: s.thumbnail }"
              :selected="s.id === selectedId"
              fill
              @click="emit('select', s.id)"
            />
            <button type="button" class="more-tile" @click="emit('openPicker')">
              <Icon name="plus" :size="14" />
              <span>Other window…</span>
            </button>
          </div>
        </div>

        <div class="start-col">
          <button type="button" class="primary" @click="emit('start')">
            <Icon name="play" :size="14" />
            Start streaming
          </button>
        </div>
      </div>
    </div>

    <footer class="footer">
      <div class="foot-left">
        <span v-if="lastSession" class="last">
          <Icon name="history" :size="9" style="vertical-align: -1px;" />
          Last: {{ lastSession.duration }} · {{ lastSession.peakViewers }} peak
        </span>
        <span v-if="updateStatus" class="last">{{ updateStatus }}</span>
      </div>
      <div class="foot-right">
        <span class="foot-link" @click="emit('openSettings')"><Icon name="cog" :size="10" /> Settings</span>
        <span class="foot-link" @click="emit('openRecordings')"><Icon name="folder" :size="10" /> Recordings</span>
        <span class="foot-link" @click="emit('checkUpdate')">
          <Icon name="refresh" :size="10" />
          Check for updates
        </span>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.idle-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.header {
  display: flex;
  align-items: center;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border-soft);
  flex-shrink: 0;
}
.brand {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex: 1;
}
.wordmark {
  font-size: 15px;
  font-weight: 600;
  letter-spacing: -0.3px;
}
.version {
  font-size: 9px;
  color: var(--text-mute);
  letter-spacing: 0.6px;
}
.head-right {
  display: flex;
  align-items: center;
  gap: 8px;
}
.sfu {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--text-dim);
}
.ms { color: var(--text-mute); font-size: 10px; }

.ghost {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 10px;
  border-radius: 6px;
  background: transparent;
  color: var(--text-dim);
  border: 1px solid var(--border);
  font-size: 11px;
  font-family: inherit;
}
.body {
  flex: 1;
  min-height: 0;
  padding: 14px;
  overflow: hidden;
}
.grid {
  display: grid;
  grid-template-columns: 1fr 200px;
  grid-template-rows: auto auto;
  gap: 10px;
  height: 100%;
}

.preview {
  grid-column: 1 / 2;
  grid-row: 1 / 2;
  border-radius: 8px;
  overflow: hidden;
  background: var(--surface-1);
  border: 1px solid var(--border-soft);
  position: relative;
  aspect-ratio: 16 / 9;
  min-height: 0;
}
.badge {
  position: absolute;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: rgba(0,0,0,0.45);
  backdrop-filter: blur(6px);
  padding: 4px 8px;
  border-radius: 4px;
  font-size: 10px;
  color: rgba(255,255,255,0.85);
  font-family: 'Geist Mono', monospace;
  letter-spacing: 0.4px;
  text-transform: uppercase;
  border: 1px solid transparent;
}
.badge.tl { left: 10px; top: 10px; }
.badge.tr {
  right: 10px;
  top: 10px;
  background: rgba(0,0,0,0.55);
  border-color: rgba(255,255,255,0.12);
  color: #fff;
  font-family: inherit;
  letter-spacing: 0;
  text-transform: none;
  cursor: pointer;
}
.src-title {
  position: absolute;
  left: 10px;
  bottom: 10px;
  font-size: 11px;
  color: rgba(255,255,255,0.85);
  font-weight: 500;
  text-shadow: 0 1px 4px rgba(0,0,0,0.6);
}

.setup-col {
  grid-column: 2 / 3;
  grid-row: 1 / 2;
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
}
.meter-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}
.db {
  font-size: 9px;
  color: var(--text-mute);
  width: 32px;
  text-align: right;
}
.cam-tile {
  margin-top: 6px;
  width: 100%;
  aspect-ratio: 16/9;
  border-radius: 4px;
  overflow: hidden;
  background: var(--bg-deep);
  border: 1px solid var(--border-soft);
}
.disabled-feature {
  position: relative;
  opacity: 0.4;
  filter: blur(0.5px);
  pointer-events: none;
  user-select: none;
}
.coming-soon {
  font-size: 9px;
  color: var(--text-mute);
  letter-spacing: 0.4px;
}

.quick-strip {
  grid-column: 1 / 2;
  grid-row: 2 / 3;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  min-height: 0;
}
.section-label {
  font-size: 9px;
  color: var(--text-mute);
  letter-spacing: 0.6px;
  text-transform: uppercase;
  font-weight: 500;
  margin-bottom: 6px;
}
.cards-list {
  display: flex;
  gap: 8px;
  min-width: 0;
}
.start-col {
  grid-column: 2 / 3;
  grid-row: 2 / 3;
  display: flex;
  align-items: flex-end;
  min-height: 0;
}
.start-col .primary {
  width: 100%;
  justify-content: center;
}
.more-tile {
  flex: 1;
  min-width: 0;
  aspect-ratio: 16/10;
  border-radius: 6px;
  border: 1px dashed var(--border);
  background: transparent;
  color: var(--text-dim);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  font-size: 10px;
  font-family: inherit;
  cursor: pointer;
}

.primary {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 11px 22px;
  border-radius: 8px;
  background: linear-gradient(180deg, var(--accent) 0%, var(--accent-soft) 100%);
  color: #fff;
  border: 1px solid var(--accent);
  font-size: 13px;
  font-weight: 600;
  font-family: inherit;
  letter-spacing: 0.2px;
  box-shadow: 0 4px 20px var(--accent-glow), inset 0 1px 0 rgba(255,255,255,0.15);
  cursor: pointer;
}

.footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 14px;
  border-top: 1px solid var(--border-soft);
  background: var(--bg-deep);
  flex-shrink: 0;
}
.foot-left, .foot-right {
  display: flex;
  gap: 12px;
  color: var(--text-mute);
  font-size: 10px;
}
.last { display: inline-flex; align-items: center; gap: 4px; }
.foot-link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
}
.foot-link:hover { color: var(--text-dim); }
</style>
