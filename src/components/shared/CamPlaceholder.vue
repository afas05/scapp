<script setup lang="ts">
withDefaults(defineProps<{
  on?: boolean;
}>(), {
  on: true,
});
</script>

<template>
  <div v-if="!on" class="cam-off">CAM OFF</div>
  <div v-else class="cam-on">
    <div class="body" />
    <div class="head" />
    <div class="stripes" />
  </div>
</template>

<style scoped>
.cam-off {
  width: 100%;
  height: 100%;
  background: var(--surface-2);
  border: 1px dashed var(--border);
  border-radius: inherit;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-mute);
  font-size: 10px;
  font-family: 'Geist Mono', monospace;
}
.cam-on {
  width: 100%;
  height: 100%;
  position: relative;
  overflow: hidden;
  background: radial-gradient(ellipse at 50% 60%, oklch(0.35 0.05 30) 0%, oklch(0.18 0.02 270) 70%);
}
.body {
  position: absolute;
  left: 50%;
  bottom: -10%;
  transform: translateX(-50%);
  width: 70%;
  height: 70%;
  border-radius: 50% 50% 40% 40%;
  background: oklch(0.13 0.02 270 / 0.8);
}
.head {
  position: absolute;
  left: 50%;
  top: 20%;
  transform: translateX(-50%);
  width: 34%;
  aspect-ratio: 1;
  border-radius: 50%;
  background: oklch(0.13 0.02 270 / 0.85);
}
.stripes {
  position: absolute;
  inset: 0;
  background-image: repeating-linear-gradient(135deg, transparent 0 6px, rgba(255,255,255,0.015) 6px 12px);
}
</style>
