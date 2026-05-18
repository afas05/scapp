<script setup lang="ts">
withDefaults(defineProps<{
  size?: number;
  color?: string;
}>(), {
  size: 14,
  color: 'currentColor',
});
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    style="display: block;"
  >
    <path d="M12 2l10 10-10 10L2 12z" :stroke="color" stroke-width="1.6" />
    <path d="M12 7v10M7 12h10" :stroke="color" stroke-width="1.2" stroke-opacity="0.55" />
    <circle cx="12" cy="12" r="1.6" :fill="color" />
  </svg>
</template>
