<script setup lang="ts">
import StatusDot from './StatusDot.vue';

withDefaults(defineProps<{
  size?: 'sm' | 'md';
}>(), {
  size: 'sm',
});
</script>

<template>
  <div class="live-pill" :class="size">
    <StatusDot color="oklch(0.65 0.22 22)" :size="6" />
    LIVE
  </div>
</template>

<style scoped>
.live-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border-radius: 4px;
  background: oklch(0.32 0.13 22 / 0.25);
  border: 1px solid oklch(0.50 0.18 22 / 0.5);
  color: oklch(0.85 0.15 22);
  font-weight: 600;
  letter-spacing: 0.8px;
}
.sm {
  padding: 3px 7px;
  font-size: 10px;
}
.md {
  padding: 4px 9px;
  font-size: 11px;
}
</style>
