<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue';
import Icon from '../shared/Icon.vue';
import StatusDot from '../shared/StatusDot.vue';
import LivePill from '../shared/LivePill.vue';
import Sparkline from '../shared/Sparkline.vue';
import ScreenPlaceholder from '../shared/ScreenPlaceholder.vue';
import SourceCard from '../idle/SourceCard.vue';
import Stat from './Stat.vue';
import NetworkBars from './NetworkBars.vue';
import MixerRow from './MixerRow.vue';
import { useMicLevel, useSimulatedLevel, fmtDuration } from '../../composables/useMicLevel';
import type { IdleSource, MicDevice } from '../idle/IdleHome.vue';

const props = withDefaults(defineProps<{
  sources: IdleSource[];
  selectedId: number;
  cameraOn: boolean;
  micOn: boolean;
  micDevice?: string;
  availableMics?: MicDevice[];
  viewerCount?: number;
  shareUrl?: string;
  previewFrame?: string;
  switchingId?: number | null;
}>(), {
  micDevice: '',
  availableMics: () => [],
  viewerCount: 0,
  shareUrl: 'streamsnipe.live',
  previewFrame: '',
  switchingId: null,
});

const emit = defineEmits<{
  stop: [];
  select: [id: number];
  toggleCamera: [];
  toggleMic: [];
  openSettings: [];
  startRecording: [];
  stopRecording: [];
}>();

const startedAt = ref(Date.now());
const tick = ref(0);
let tickTimer: number | null = null;
let sampleTimer: number | null = null;

const elapsed = computed(() => (tick.value, (Date.now() - startedAt.value) / 1000));

// Real viewer history — sampled from the prop every 1.5s. Starts empty (0 default).
const history = ref<number[]>([0]);

onMounted(() => {
  tickTimer = window.setInterval(() => { tick.value++; }, 1000);
  sampleTimer = window.setInterval(() => {
    const next = Math.max(0, props.viewerCount | 0);
    history.value = [...history.value.slice(-29), next];
  }, 1500);
});

onBeforeUnmount(() => {
  if (tickTimer != null) clearInterval(tickTimer);
  if (sampleTimer != null) clearInterval(sampleTimer);
});

// Push immediately whenever the upstream count changes, so the value never lags
// while we wait for the next 1.5s sample tick.
watch(() => props.viewerCount, (n) => {
  history.value = [...history.value.slice(-29), Math.max(0, n | 0)];
});

const liveViewers = computed(() => Math.max(0, props.viewerCount | 0));

const trend = computed(() => {
  const h = history.value;
  if (h.length < 2) return 0;
  return h[h.length - 1] - (h[h.length - 8] ?? h[0]);
});

const bitrate = computed(() => {
  return (6.0 + Math.sin(tick.value * 0.3) * 0.4 + (Math.random() - 0.5) * 0.2).toFixed(1);
});

const fps = computed(() =>
  60 - Math.max(0, Math.round(Math.sin(tick.value * 0.2) * 0.5))
);

const selectedMicName = computed(() =>
  props.availableMics.find(m => m.id === props.micDevice)?.name
);
const { level: mic } = useMicLevel(() => props.micOn, () => selectedMicName.value);
const game = useSimulatedLevel(() => true, 0.62);

const recording = ref(false);
const recStart = ref<number | null>(null);
const recElapsed = computed(() => {
  tick.value; // re-evaluate each second
  if (!recording.value || recStart.value == null) return 0;
  return (Date.now() - recStart.value) / 1000;
});

function toggleRecording() {
  if (recording.value) {
    recording.value = false;
    recStart.value = null;
    emit('stopRecording');
  } else {
    recording.value = true;
    recStart.value = Date.now();
    emit('startRecording');
  }
}

// Ask the parent to swap the live source; it drives the real backend switch and
// reflects progress back via the `switchingId` prop (which powers the card's
// "SWITCHING…" overlay until the operation completes or fails).
function switchSource(id: number) {
  if (id === props.selectedId || props.switchingId !== null) return;
  emit('select', id);
}

const copied = ref(false);
async function copy() {
  try {
    if (navigator.clipboard) await navigator.clipboard.writeText(props.shareUrl);
  } catch {}
  copied.value = true;
  setTimeout(() => (copied.value = false), 1400);
}

const selectedSource = computed(() =>
  props.sources.find(s => s.id === props.selectedId) || props.sources[0]
);

const trendSub = computed(() => {
  const t = trend.value;
  return t >= 0 ? `+${t}` : `${t}`;
});
const trendTone = computed<'up' | 'down'>(() => (trend.value >= 0 ? 'up' : 'down'));
</script>

<template>
  <div class="live-root">
    <header class="stat-strip">
      <LivePill />
      <div class="uptime">
        <span class="ws-mono uptime-val">{{ fmtDuration(elapsed) }}</span>
        <span class="uptime-label">uptime</span>
      </div>

      <div class="stat-group">
        <Stat label="Viewers" :value="liveViewers" :sub="trendSub" :sub-tone="trendTone">
          <Sparkline :data="history" :width="60" :height="16" />
        </Stat>
        <span class="divider" />
        <Stat label="Bitrate" :value="bitrate" sub="Mbps" />
        <span class="divider" />
        <Stat label="FPS" :value="fps" sub="1080p" />
        <span class="divider" />
        <Stat label="Signal" value="" sub="Excellent" sub-tone="up">
          <template #value>
            <NetworkBars :strength="4" />
          </template>
        </Stat>
      </div>

      <button type="button" class="ghost cog" @click="emit('openSettings')">
        <Icon name="cog" :size="11" />
      </button>
    </header>

    <div class="preview">
      <ScreenPlaceholder
        v-if="selectedSource"
        :kind="selectedSource.kind"
        :thumbnail="previewFrame || selectedSource.thumbnail"
      />
      <ScreenPlaceholder v-else kind="desktop" />

      <div class="preview-label">
        <span class="rec-dot" />
        What viewers see
      </div>

      <div v-if="selectedSource" class="src-title">
        {{ selectedSource.title }}
        <div class="src-proc">{{ selectedSource.processName }}</div>
      </div>

    </div>

    <div class="lower">
      <div class="sources-col">
        <div class="section-head">
          <span class="section-label">Sources</span>
          <span class="swap-hint">Click to swap live</span>
        </div>
        <div class="ws-scroll source-list">
          <SourceCard
            v-for="s in sources"
            :key="s.id"
            :source="{ id: s.id, kind: s.kind, title: s.title, process: s.processName, thumbnail: s.thumbnail }"
            :selected="s.id === selectedId"
            :switching="switchingId === s.id"
            :compact="true"
            @click="switchSource(s.id)"
          />
        </div>
      </div>

      <div class="mixer-col">
        <div class="section-label">Audio</div>
        <div class="mixer-card">
          <MixerRow
            icon="mic"
            label="Mic"
            :value="mic"
            :muted="!micOn"
            @toggle="emit('toggleMic')"
          />
          <MixerRow
            icon="audio"
            label="Game"
            :value="game"
            :muted="false"
            :toggleable="false"
          />
          <div class="disabled-feature">
            <MixerRow
              icon="cam"
              label="Cam"
              :muted="true"
              :show-meter="false"
            />
          </div>
        </div>
      </div>
    </div>

    <footer class="action-bar">
      <button type="button" class="danger" @click="emit('stop')">
        <Icon name="stop" :size="11" />
        Stop stream
      </button>

      <button
        type="button"
        class="ghost rec-btn"
        :class="{ recording }"
        @click="toggleRecording"
      >
        <StatusDot v-if="recording" color="var(--rec)" :size="6" />
        <Icon v-else name="rec" :size="10" />
        <span v-if="recording" class="ws-mono">REC {{ fmtDuration(recElapsed) }}</span>
        <span v-else>Record locally</span>
      </button>

      <div class="share">
        <Icon name="share" :size="11" style="color: var(--text-dim); flex-shrink: 0;" />
        <div class="ws-mono share-url">{{ shareUrl }}</div>
        <button
          type="button"
          class="share-copy"
          :class="{ copied }"
          @click="copy"
        >
          <Icon :name="copied ? 'check' : 'copy'" :size="11" />
          {{ copied ? 'Copied' : 'Copy' }}
        </button>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.live-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.stat-strip {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border-soft);
  background: var(--bg-deep);
  flex-shrink: 0;
}
.uptime {
  display: flex;
  align-items: baseline;
  gap: 5px;
}
.uptime-val {
  font-size: 17px;
  font-weight: 600;
}
.uptime-label {
  font-size: 9px;
  color: var(--text-mute);
  letter-spacing: 0.6px;
  text-transform: uppercase;
}
.stat-group {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 14px;
}
.divider {
  width: 1px;
  height: 22px;
  background: var(--border-soft);
}

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
  cursor: pointer;
}
.cog { padding: 5px; }

.preview {
  position: relative;
  flex: 1 1 0;
  min-height: 120px;
  background: #000;
  overflow: hidden;
  border-bottom: 1px solid var(--border-soft);
}
.preview-label {
  position: absolute;
  left: 12px;
  top: 10px;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 9px;
  color: rgba(255,255,255,0.7);
  font-family: 'Geist Mono', monospace;
  letter-spacing: 0.6px;
  text-transform: uppercase;
}
.rec-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: oklch(0.65 0.22 22);
  box-shadow: 0 0 8px oklch(0.65 0.22 22);
}
.src-title {
  position: absolute;
  left: 12px;
  bottom: 10px;
  font-size: 12px;
  color: #fff;
  font-weight: 500;
  text-shadow: 0 1px 6px rgba(0,0,0,0.6);
}
.src-proc {
  font-size: 9px;
  color: rgba(255,255,255,0.55);
  font-family: 'Geist Mono', monospace;
}
.lower {
  flex: 0 0 auto;
  padding: 10px 14px;
  display: grid;
  grid-template-columns: 1fr 220px;
  gap: 10px;
}
.sources-col {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.section-label {
  font-size: 9px;
  color: var(--text-mute);
  letter-spacing: 0.6px;
  text-transform: uppercase;
  font-weight: 500;
}
.swap-hint {
  font-size: 9px;
  color: var(--text-mute);
  font-family: 'Geist Mono', monospace;
}
.source-list {
  display: flex;
  gap: 8px;
  overflow-x: auto;
  padding-bottom: 4px;
}

.mixer-col {
  display: flex;
  flex-direction: column;
}
.mixer-card {
  padding: 8px;
  border-radius: 6px;
  background: var(--surface-1);
  border: 1px solid var(--border-soft);
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.disabled-feature {
  opacity: 0.4;
  filter: blur(0.5px);
  pointer-events: none;
  user-select: none;
}

.action-bar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-top: 1px solid var(--border-soft);
  background: var(--bg-deep);
}
.danger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: 6px;
  background: oklch(0.32 0.13 22 / 0.4);
  color: oklch(0.92 0.05 22);
  border: 1px solid oklch(0.55 0.18 22 / 0.6);
  font-size: 12px;
  font-weight: 500;
  font-family: inherit;
  cursor: pointer;
}
.rec-btn.recording {
  border-color: var(--rec);
  color: oklch(0.85 0.15 22);
}

.share {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border-radius: 6px;
  background: var(--surface-1);
  border: 1px solid var(--border-soft);
  min-width: 0;
}
.share-url {
  flex: 1;
  font-size: 11px;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.share-copy {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: transparent;
  border: none;
  color: var(--text-dim);
  font-size: 11px;
  font-family: inherit;
  padding: 0;
  cursor: pointer;
}
.share-copy.copied { color: var(--live); }
</style>
