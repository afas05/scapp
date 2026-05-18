<script setup lang="ts">
withDefaults(defineProps<{
  color?: string;
  pulse?: boolean;
  size?: number;
}>(), {
  color: 'var(--live)',
  pulse: true,
  size: 8,
});
</script>

<template>
  <span
    class="dot-root"
    :style="{ width: size + 'px', height: size + 'px' }"
  >
    <span
      v-if="pulse"
      class="dot-pulse"
      :style="{ background: color }"
    />
    <span
      class="dot-core"
      :style="{ width: size + 'px', height: size + 'px', background: color }"
    />
  </span>
</template>

<style scoped>
.dot-root {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}
.dot-pulse {
  position: absolute;
  inset: -4px;
  border-radius: 50%;
  opacity: 0.25;
  animation: wsPulse 1.6s ease-out infinite;
}
.dot-core {
  border-radius: 50%;
  display: block;
}
</style>
