<script setup lang="ts">
import { ref, watch } from 'vue';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import Icon from './Icon.vue';
import { useSettingsStore } from '../../stores/settingsStore';

const props = defineProps<{ modelValue: boolean }>();
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();

const settingsStore = useSettingsStore();
const editPath = ref('');

watch(() => props.modelValue, (open) => {
  if (open) {
    editPath.value = settingsStore.recordingPath;
  }
});

function close() {
  emit('update:modelValue', false);
}

async function save() {
  await settingsStore.setRecordingPath(editPath.value);
  close();
}

async function browseFolder() {
  try {
    const selected = await openDialog({
      directory: true,
      defaultPath: editPath.value || undefined,
    });
    if (selected) {
      editPath.value = selected;
    }
  } catch (err) {
    console.warn('[Settings] Failed to open folder picker:', err);
  }
}
</script>

<template>
  <Teleport to="body">
    <div v-if="modelValue" class="settings-overlay" @click.self="close">
      <div class="settings-modal">
        <div class="settings-header">
          <h2>Settings</h2>
          <button class="close-btn" @click="close">
            <Icon name="x" :size="14" />
          </button>
        </div>

        <div class="settings-body">
          <div class="setting-group">
            <label class="setting-label">Recording save path</label>
            <div class="path-row">
              <input
                class="app-input path-input"
                v-model="editPath"
                spellcheck="false"
              />
              <button type="button" class="ghost browse-btn" @click="browseFolder">
                <Icon name="folder" :size="11" />
                Browse
              </button>
            </div>
            <p class="setting-hint">Recordings will be saved to this folder</p>
          </div>
        </div>

        <div class="settings-footer">
          <button type="button" class="ghost" @click="close">Cancel</button>
          <button type="button" class="primary-sm" @click="save">Save</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.settings-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.75);
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
}

.settings-modal {
  background: var(--bg-deep, #1a1a2e);
  border-radius: 12px;
  width: 420px;
  max-width: 90vw;
  max-height: 80vh;
  overflow-y: auto;
  padding: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
  border: 1px solid var(--border-soft);
}

.settings-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.settings-header h2 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}

.close-btn {
  background: transparent;
  border: none;
  color: var(--text-mute);
  cursor: pointer;
  padding: 4px;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.close-btn:hover {
  color: var(--text);
  background: rgba(255, 255, 255, 0.08);
}

.settings-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.setting-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.setting-label {
  font-size: 11px;
  font-weight: 500;
  color: var(--text-dim);
  letter-spacing: 0.2px;
}

.path-row {
  display: flex;
  gap: 6px;
}

.path-input {
  flex: 1;
  min-width: 0;
}

.ghost {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 10px;
  border-radius: 6px;
  background: transparent;
  color: var(--text-dim);
  border: 1px solid var(--border);
  font-size: 11px;
  font-family: inherit;
  cursor: pointer;
}
.ghost:hover {
  color: var(--text);
  border-color: var(--text-mute);
}

.browse-btn {
  flex-shrink: 0;
}

.setting-hint {
  margin: 0;
  font-size: 10px;
  color: var(--text-mute);
}

.settings-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 4px;
  border-top: 1px solid var(--border-soft);
}

.primary-sm {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 14px;
  border-radius: 6px;
  background: linear-gradient(180deg, var(--accent) 0%, var(--accent-soft) 100%);
  color: #fff;
  border: 1px solid var(--accent);
  font-size: 11px;
  font-weight: 500;
  font-family: inherit;
  cursor: pointer;
}
.primary-sm:hover {
  filter: brightness(1.1);
}
</style>
