<script setup lang="ts">
// "Go Pro" call to action for limited plans. The parent opens the pricing page
// in the system browser (PRICING_URL); show it only when the plan is limited.
withDefaults(defineProps<{
  label?: string;
  block?: boolean;
}>(), {
  label: 'Go Pro',
  block: false,
});

defineEmits<{ click: [] }>();
</script>

<template>
  <button type="button" class="upgrade-btn" :class="{ block }" @click="$emit('click')">
    <span class="spark" aria-hidden="true">★</span>
    {{ label }}
    <span class="ext" aria-hidden="true">↗</span>
  </button>
</template>

<style scoped>
.upgrade-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 5px 10px;
  border-radius: 6px;
  background: linear-gradient(180deg, #f0b429 0%, #d6951a 100%);
  color: #1a1206;
  border: 1px solid #f0b429;
  font-family: inherit;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}
.upgrade-btn:hover { filter: brightness(1.08); }
.upgrade-btn.block { width: 100%; padding: 7px 10px; }
.spark { font-size: 10px; }
.ext { font-size: 9px; opacity: 0.7; }
</style>
