<script setup lang="ts">
withDefaults(defineProps<{
  label: string;
  value: string | number;
  sub?: string;
  subTone?: 'up' | 'down' | 'neutral';
}>(), {
  subTone: 'neutral',
});

const tones = {
  up: 'var(--live)',
  down: 'oklch(0.78 0.16 30)',
  neutral: 'var(--text-mute)',
};
</script>

<template>
  <div class="stat">
    <div class="text">
      <span class="label">{{ label }}</span>
      <span class="value-row">
        <span class="ws-mono value">
          <slot name="value">{{ value }}</slot>
        </span>
        <span v-if="sub" class="sub" :style="{ color: tones[subTone] }">{{ sub }}</span>
      </span>
    </div>
    <slot />
  </div>
</template>

<style scoped>
.stat {
  display: flex;
  align-items: center;
  gap: 8px;
}
.text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  align-items: flex-start;
}
.label {
  font-size: 8.5px;
  color: var(--text-mute);
  letter-spacing: 0.6px;
  text-transform: uppercase;
}
.value-row {
  display: flex;
  align-items: baseline;
  gap: 4px;
}
.value {
  font-size: 14px;
  font-weight: 600;
}
.sub {
  font-size: 9px;
  font-family: 'Geist Mono', monospace;
}
</style>
