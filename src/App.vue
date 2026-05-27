<script setup lang="ts">
import { onMounted } from 'vue';
import { useRouter, useRoute } from 'vue-router';
import { useUserStore } from './stores/userStore';
import { useSettingsStore } from './stores/settingsStore';

const userStore = useUserStore();
const settingsStore = useSettingsStore();
const router = useRouter();
const route = useRoute();

onMounted(async () => {
  await Promise.all([userStore.hydrate(), settingsStore.hydrate()]);
  if (userStore.isLoggedIn && route.path !== '/start') {
    await router.replace('/start');
  } else if (!userStore.isLoggedIn && route.meta.requiresAuth) {
    await router.replace('/login');
  }
});
</script>

<template>
  <main class="app-shell">
    <div v-if="userStore.hydrating" class="loading">Loading…</div>
    <router-view v-else />
  </main>
</template>

<style scoped>
.app-shell {
  position: relative;
  width: 100%;
  height: 100%;
  background: var(--bg);
  color: var(--text);
  overflow: hidden;
}
.loading {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-dim);
  font-size: 12px;
}
</style>

<style>
input,
button {
  font-family: inherit;
}
.app-input {
  background: var(--surface-2);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 13px;
  margin: 4px 0;
}
.app-input:focus {
  outline: none;
  border-color: var(--accent);
}
</style>
