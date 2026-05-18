<script setup lang="ts">
withDefaults(defineProps<{
  strength?: number;
}>(), {
  strength: 4,
});

const heights = [3, 6, 9, 12];
</script>

<template>
  <span class="bars">
    <span
      v-for="(h, i) in heights"
      :key="i"
      class="bar"
      :style="{
        height: h + 'px',
        background: i < strength ? 'var(--live)' : 'var(--surface-3)',
      }"
    />
  </span>
</template>

<style scoped>
.bars {
  display: inline-flex;
  align-items: flex-end;
  gap: 1.5px;
  height: 12px;
}
.bar {
  width: 2px;
  border-radius: 1px;
}
</style>
