<script setup lang="ts">
import { computed } from 'vue';

const props = withDefaults(defineProps<{
  value: number;
  bars?: number;
  height?: number;
  color?: string;
  peakColor?: string;
}>(), {
  bars: 18,
  height: 10,
  color: 'var(--accent)',
  peakColor: 'var(--rec)',
});

const items = computed(() => Array.from({ length: props.bars }, (_, i) => {
  const t = (i + 1) / props.bars;
  const lit = props.value >= t - 0.001;
  const high = i >= props.bars - 3;
  const mid = i >= props.bars - 6;
  let bg = 'var(--surface-3)';
  if (lit) bg = high ? props.peakColor : (mid ? 'var(--warn)' : props.color);
  return { lit, bg };
}));
</script>

<template>
  <div class="meter" :style="{ height: height + 'px' }">
    <div
      v-for="(b, i) in items"
      :key="i"
      class="bar"
      :style="{ background: b.bg, opacity: b.lit ? 1 : 0.5 }"
    />
  </div>
</template>

<style scoped>
.meter {
  display: flex;
  gap: 2px;
  align-items: stretch;
  flex: 1;
  min-width: 0;
}
.bar {
  flex: 1;
  border-radius: 1px;
  transition: background .05s;
}
</style>
