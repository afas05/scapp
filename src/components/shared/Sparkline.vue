<script setup lang="ts">
import { computed } from 'vue';

const props = withDefaults(defineProps<{
  data: number[];
  width?: number;
  height?: number;
  color?: string;
  area?: boolean;
}>(), {
  width: 64,
  height: 18,
  color: 'var(--accent)',
  area: true,
});

const paths = computed(() => {
  const data = props.data;
  if (!data.length) return null;
  const max = Math.max(...data, 1);
  const min = Math.min(...data, 0);
  const range = Math.max(1, max - min);
  const pts = data.map((v, i) => {
    const x = (i / Math.max(1, data.length - 1)) * props.width;
    const y = props.height - 1 - ((v - min) / range) * (props.height - 2);
    return [x, y] as [number, number];
  });
  const path = pts.map(([x, y], i) => (i === 0 ? `M${x},${y}` : `L${x},${y}`)).join(' ');
  const areaP = path + ` L${props.width},${props.height} L0,${props.height} Z`;
  const last = pts[pts.length - 1];
  return { path, areaP, last };
});
</script>

<template>
  <svg
    :width="width"
    :height="height"
    style="display: block; overflow: visible;"
  >
    <template v-if="paths">
      <path v-if="area" :d="paths.areaP" :fill="color" fill-opacity="0.18" />
      <path
        :d="paths.path"
        fill="none"
        :stroke="color"
        stroke-width="1.4"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
      <circle :cx="paths.last[0]" :cy="paths.last[1]" r="1.6" :fill="color" />
    </template>
  </svg>
</template>
