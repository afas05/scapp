<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from 'vue';
import Icon from '../shared/Icon.vue';

const props = defineProps<{
  value: string;
  options: string[];
  disabled?: boolean;
}>();

const emit = defineEmits<{
  change: [value: string];
}>();

const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
}

function pick(o: string) {
  emit('change', o);
  open.value = false;
}

function onPointerDown(e: PointerEvent) {
  if (!open.value) return;
  const node = rootRef.value;
  if (node && !node.contains(e.target as Node)) {
    open.value = false;
  }
}

onMounted(() => document.addEventListener('pointerdown', onPointerDown));
onBeforeUnmount(() => document.removeEventListener('pointerdown', onPointerDown));
</script>

<template>
  <div ref="rootRef" class="device-select">
    <button
      type="button"
      :disabled="disabled"
      class="trigger"
      :class="{ disabled }"
      @click="toggle"
    >
      <span class="value">{{ value }}</span>
      <Icon v-if="!disabled" name="chevd" :size="9" class="chev" />
    </button>
    <div v-if="open" class="menu">
      <button
        v-for="o in options"
        :key="o"
        type="button"
        class="option"
        :class="{ selected: o === value }"
        @click="pick(o)"
      >
        <span class="radio" :class="{ on: o === value }">
          <span v-if="o === value" class="radio-dot" />
        </span>
        <span class="option-label">{{ o }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.device-select { position: relative; }
.trigger {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 3px;
  background: transparent;
  border: none;
  padding: 0;
  font-family: inherit;
  font-size: 10px;
  color: var(--text-dim);
  text-align: left;
  cursor: pointer;
  min-width: 0;
}
.trigger.disabled {
  color: var(--text-mute);
  cursor: default;
}
.value {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chev { opacity: 0.6; }
.menu {
  position: absolute;
  top: calc(100% + 6px);
  left: -6px;
  right: -6px;
  z-index: 50;
  background: var(--surface-2);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 4px;
  box-shadow: 0 8px 24px rgba(0,0,0,0.5);
}
.option {
  width: 100%;
  text-align: left;
  padding: 5px 7px;
  border-radius: 4px;
  background: transparent;
  color: var(--text-dim);
  border: none;
  font-family: inherit;
  font-size: 10px;
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
}
.option:hover:not(.selected) { background: var(--surface-3); }
.option.selected {
  background: var(--accent-soft);
  color: var(--text);
}
.radio {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  border: 1.5px solid var(--border);
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.radio.on {
  border-color: var(--accent);
  background: var(--accent);
}
.radio-dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--bg-deep);
}
.option-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
