<script setup lang="ts">
import { computed } from 'vue';

const props = withDefaults(defineProps<{
  kind?: 'game' | 'browser' | 'app' | 'desktop';
  label?: string;
  thumbnail?: string;
}>(), {
  kind: 'game',
  label: '',
  thumbnail: '',
});

const gradients = {
  game:    'linear-gradient(135deg, oklch(0.22 0.10 280) 0%, oklch(0.32 0.14 320) 60%, oklch(0.20 0.10 240) 100%)',
  browser: 'linear-gradient(135deg, oklch(0.22 0.04 240) 0%, oklch(0.28 0.05 220) 100%)',
  app:     'linear-gradient(135deg, oklch(0.22 0.02 290) 0%, oklch(0.28 0.03 290) 100%)',
  desktop: 'linear-gradient(135deg, oklch(0.18 0.02 250) 0%, oklch(0.24 0.04 280) 100%)',
};

const bgStyle = computed(() => {
  if (props.thumbnail) {
    return {
      backgroundImage: `url(data:image/jpeg;base64,${props.thumbnail})`,
      backgroundSize: 'contain',
      backgroundPosition: 'center',
      backgroundRepeat: 'no-repeat',
      backgroundColor: '#000',
    };
  }
  return { background: gradients[props.kind] || gradients.game };
});
</script>

<template>
  <div class="screen" :style="bgStyle">
    <div v-if="!thumbnail" class="stripes" />
    <div v-if="label" class="label">{{ label }}</div>
  </div>
</template>

<style scoped>
.screen {
  width: 100%;
  height: 100%;
  position: relative;
  overflow: hidden;
}
.stripes {
  position: absolute;
  inset: 0;
  background-image: repeating-linear-gradient(135deg, transparent 0 14px, rgba(255,255,255,0.025) 14px 28px);
}
.label {
  position: absolute;
  left: 8px;
  bottom: 6px;
  font-family: 'Geist Mono', monospace;
  font-size: 9px;
  color: rgba(255,255,255,0.5);
  letter-spacing: 0.5px;
  text-transform: uppercase;
}
</style>
