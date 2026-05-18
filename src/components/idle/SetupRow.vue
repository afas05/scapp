<script setup lang="ts">
import Icon from '../shared/Icon.vue';

defineProps<{
  icon: string;
  label: string;
  subtext?: string;
  on: boolean;
  toggleable?: boolean;
}>();

defineEmits<{
  toggle: [];
}>();
</script>

<template>
  <div class="setup-row">
    <div class="head">
      <Icon
        :name="icon"
        :size="12"
        :style="{ color: on ? 'var(--accent)' : 'var(--text-mute)' }"
      />
      <div class="text">
        <div class="label">{{ label }}</div>
        <div v-if="subtext != null" class="subtext">{{ subtext }}</div>
        <slot name="value" />
      </div>
      <button
        v-if="toggleable"
        type="button"
        class="toggle"
        :class="{ on }"
        @click="$emit('toggle')"
      >
        <span class="thumb" :class="{ on }" />
      </button>
    </div>
    <slot />
  </div>
</template>

<style scoped>
.setup-row {
  padding: 10px;
  border-radius: 8px;
  background: var(--surface-1);
  border: 1px solid var(--border-soft);
}
.head {
  display: flex;
  align-items: center;
  gap: 7px;
}
.text {
  flex: 1;
  min-width: 0;
}
.label {
  font-size: 11px;
  font-weight: 500;
}
.subtext {
  font-size: 9px;
  color: var(--text-mute);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.toggle {
  position: relative;
  width: 26px;
  height: 14px;
  border-radius: 999px;
  background: var(--surface-3);
  border: none;
  flex-shrink: 0;
  transition: background .15s;
  padding: 0;
  cursor: pointer;
}
.toggle.on { background: var(--accent); }
.thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: #fff;
  transition: left .15s;
}
.thumb.on { left: 14px; }
</style>
