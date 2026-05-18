<script setup lang="ts">
import Icon from '../shared/Icon.vue';
import MeterBars from '../shared/MeterBars.vue';

withDefaults(defineProps<{
  icon: string;
  label: string;
  value?: number;
  muted: boolean;
  showMeter?: boolean;
  toggleable?: boolean;
}>(), {
  value: 0,
  showMeter: true,
  toggleable: true,
});

defineEmits<{
  toggle: [];
}>();
</script>

<template>
  <div class="mixer-row">
    <button
      type="button"
      class="mute"
      :class="{ on: !muted }"
      :disabled="!toggleable"
      @click="toggleable && $emit('toggle')"
    >
      <Icon :name="icon" :size="11" />
    </button>
    <span class="label">{{ label }}</span>
    <MeterBars
      v-if="showMeter"
      :value="muted ? 0 : value"
      :bars="14"
      :height="6"
    />
    <span v-else class="ws-mono state">{{ muted ? 'off' : 'on' }}</span>
  </div>
</template>

<style scoped>
.mixer-row {
  display: flex;
  align-items: center;
  gap: 7px;
}
.mute {
  width: 22px;
  height: 22px;
  border-radius: 4px;
  flex-shrink: 0;
  background: var(--surface-3);
  border: 1px solid var(--border);
  color: var(--text-mute);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
}
.mute.on {
  background: oklch(0.32 0.13 158 / 0.4);
  border-color: var(--live-soft);
  color: var(--live);
}
.mute:disabled { opacity: 0.7; cursor: default; }
.label {
  width: 32px;
  font-size: 10px;
  color: var(--text-dim);
}
.state {
  flex: 1;
  font-size: 10px;
  color: var(--text-mute);
}
</style>
