<script setup lang="ts">
import Icon from '../shared/Icon.vue';
import ScreenPlaceholder from '../shared/ScreenPlaceholder.vue';

export interface SourceCardData {
  id: string | number;
  kind?: 'game' | 'browser' | 'app' | 'desktop';
  title: string;
  process: string;
  thumbnail?: string;
}

withDefaults(defineProps<{
  source: SourceCardData;
  selected?: boolean;
  switching?: boolean;
  fill?: boolean;
  compact?: boolean;
}>(), {
  selected: false,
  switching: false,
  fill: false,
  compact: true,
});

defineEmits<{
  click: [];
}>();
</script>

<template>
  <button
    type="button"
    class="source-card"
    :class="{ selected, fill, compactWidth: !fill && compact, regularWidth: !fill && !compact }"
    @click="$emit('click')"
  >
    <div class="thumb">
      <ScreenPlaceholder :kind="source.kind" :thumbnail="source.thumbnail" />
      <div v-if="selected" class="check">
        <Icon name="check" :size="9" :stroke="2.5" />
      </div>
      <div v-if="switching" class="switching">SWITCHING…</div>
    </div>
    <div class="caption">
      <div class="title">{{ source.title }}</div>
      <div class="process">{{ source.process }}</div>
    </div>
  </button>
</template>

<style scoped>
.source-card {
  border-radius: 6px;
  overflow: hidden;
  position: relative;
  background: var(--surface-1);
  border: 1.5px solid var(--border-soft);
  padding: 0;
  font-family: inherit;
  text-align: left;
  transition: border-color .12s, box-shadow .12s;
  cursor: pointer;
  color: var(--text);
}
.fill { flex: 1; min-width: 0; }
.compactWidth { flex-shrink: 0; width: 92px; }
.regularWidth { flex-shrink: 0; width: 108px; }
.selected {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-glow);
}
.thumb {
  aspect-ratio: 16 / 10;
  position: relative;
}
.check {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 14px;
  height: 14px;
  border-radius: 999px;
  background: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
}
.switching {
  position: absolute;
  inset: 0;
  background: rgba(0,0,0,0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 9px;
  font-family: 'Geist Mono', monospace;
  letter-spacing: 0.4px;
}
.caption { padding: 5px 6px; }
.title {
  font-size: 10px;
  color: var(--text);
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.process {
  font-size: 8px;
  color: var(--text-mute);
  font-family: 'Geist Mono', monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
