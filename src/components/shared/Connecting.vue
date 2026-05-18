<script setup lang="ts">
withDefaults(defineProps<{
  host?: string;
}>(), {
  host: 'sfu-eu-west-1.streamsnipe.live',
});
</script>

<template>
  <div class="connecting">
    <div class="spinner" />
    <div class="text">
      <div class="title">Connecting to SFU</div>
      <div class="ws-mono sub">{{ host }} · negotiating media…</div>
    </div>
  </div>
</template>

<style scoped>
.connecting {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  background: var(--bg-deep);
  z-index: 30;
}
.spinner {
  width: 30px;
  height: 30px;
  border-radius: 50%;
  border: 2.5px solid var(--surface-3);
  border-top-color: var(--accent);
  animation: wsSpin 0.8s linear infinite;
}
.text { text-align: center; }
.title { font-size: 13px; font-weight: 600; }
.sub { font-size: 10px; color: var(--text-mute); margin-top: 4px; }
</style>
