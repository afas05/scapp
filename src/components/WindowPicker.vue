<script setup lang="ts">
import { ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';

interface WindowInfo {
  id: number;
  pid: number;
  title: string;
  process_name: string;
  thumbnail: string;
}

const props = defineProps<{ modelValue: boolean }>();
const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  selected: [payload: { windowHandle: number; processPid: number }];
}>();

const windows = ref<WindowInfo[]>([]);
const loading = ref(false);
const error = ref('');
const hoveredId = ref<number | null>(null);

watch(() => props.modelValue, async (open) => {
  if (!open) return;
  loading.value = true;
  error.value = '';
  windows.value = [];
  try {
    windows.value = await invoke<WindowInfo[]>('list_windows');
  } catch (e: any) {
    error.value = e?.message ?? String(e);
  } finally {
    loading.value = false;
  }
});

function close() {
  emit('update:modelValue', false);
}

function select(win: WindowInfo) {
  emit('selected', { windowHandle: win.id, processPid: win.pid });
  close();
}

function selectScreen() {
  emit('selected', { windowHandle: 0, processPid: 0 });
  close();
}
</script>

<template>
  <Teleport to="body">
    <div v-if="modelValue" class="picker-overlay" @click.self="close">
      <div class="picker-modal">
        <div class="picker-header">
          <h2>Select a window to capture</h2>
          <button class="close-btn" @click="close">✕</button>
        </div>

        <div v-if="loading" class="picker-status">
          <div class="spinner"></div>
          <span>Scanning windows…</span>
        </div>

        <div v-else-if="error" class="picker-status picker-error">
          {{ error }}
        </div>

        <div v-else class="picker-grid">
          <div
            class="window-card"
            :class="{ hovered: hoveredId === 0 }"
            @mouseenter="hoveredId = 0"
            @mouseleave="hoveredId = null"
          >
            <div class="thumbnail-wrapper">
              <div class="thumbnail-placeholder screen-tile">
                <span class="screen-icon">🖥</span>
                <span>Entire Screen</span>
              </div>
              <div v-show="hoveredId === 0" class="hover-overlay">
                <button class="select-btn" @click="selectScreen">Select</button>
              </div>
            </div>
            <p class="window-title">Entire Screen</p>
            <p class="window-process">Primary monitor</p>
          </div>
          <div
            v-for="win in windows"
            :key="win.id"
            class="window-card"
            :class="{ hovered: hoveredId === win.id }"
            @mouseenter="hoveredId = win.id"
            @mouseleave="hoveredId = null"
          >
            <div class="thumbnail-wrapper">
              <img
                v-if="win.thumbnail"
                :src="`data:image/jpeg;base64,${win.thumbnail}`"
                :alt="win.title"
                class="thumbnail-img"
              />
              <div v-else class="thumbnail-placeholder">
                <span>No preview</span>
              </div>
              <div v-show="hoveredId === win.id" class="hover-overlay">
                <button class="select-btn" @click="select(win)">Select</button>
              </div>
            </div>
            <p class="window-title" :title="win.title">{{ win.title }}</p>
            <p class="window-process">{{ win.process_name }}</p>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.picker-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.75);
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
}

.picker-modal {
  background: #1a1a2e;
  border-radius: 12px;
  width: 80vw;
  max-width: 900px;
  max-height: 80vh;
  overflow-y: auto;
  padding: 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.picker-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.picker-header h2 {
  margin: 0;
  font-size: 1.1rem;
  color: #fff;
}

.close-btn {
  background: transparent;
  border: none;
  color: #aaa;
  font-size: 1.1rem;
  cursor: pointer;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
}

.close-btn:hover {
  color: #fff;
  background: rgba(255, 255, 255, 0.1);
}

.picker-status {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  color: #aaa;
  padding: 2rem 0;
  justify-content: center;
}

.picker-error {
  color: #ff6b6b;
}

.spinner {
  width: 20px;
  height: 20px;
  border: 2px solid rgba(255, 255, 255, 0.2);
  border-top-color: #fff;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.picker-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 1rem;
}

.window-card {
  background: #16213e;
  border-radius: 8px;
  overflow: hidden;
  border: 2px solid transparent;
  transition: border-color 0.15s;
  cursor: pointer;
}

.window-card.hovered {
  border-color: #4c036c;
}

.thumbnail-wrapper {
  position: relative;
  aspect-ratio: 16 / 9;
  overflow: hidden;
  background: #0d0d1a;
}

.thumbnail-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.thumbnail-placeholder {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #444;
  font-size: 0.8rem;
}

.screen-tile {
  flex-direction: column;
  gap: 0.5rem;
  background: linear-gradient(135deg, #2a1a4e 0%, #4c036c 100%);
  color: #fff;
  font-size: 0.95rem;
}

.screen-icon {
  font-size: 2.5rem;
  line-height: 1;
}

.hover-overlay {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
}

.select-btn {
  background: #4c036c;
  color: white;
  border: none;
  border-radius: 9999px;
  padding: 0.5rem 1.5rem;
  font-size: 0.9rem;
  cursor: pointer;
  transition: background 0.15s;
}

.select-btn:hover {
  background: #5c037c;
}

.window-title {
  margin: 0.5rem 0.5rem 0.1rem;
  font-size: 0.82rem;
  color: #ddd;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.window-process {
  margin: 0 0.5rem 0.5rem;
  font-size: 0.72rem;
  color: #777;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
