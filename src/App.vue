<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { useUserStore } from './stores/userStore'

const userStore = useUserStore()
const router = useRouter()
const route = useRoute()

const updateStatus = ref('')
const updateBusy = ref(false)

onMounted(async () => {
  await userStore.hydrate()
  if (userStore.isLoggedIn && route.path !== '/start') {
    await router.replace('/start')
  } else if (!userStore.isLoggedIn && route.meta.requiresAuth) {
    await router.replace('/login')
  }
})

async function onLogout() {
  await userStore.logout()
  await router.replace('/login')
}

async function checkForUpdate() {
  if (updateBusy.value) return
  updateBusy.value = true
  updateStatus.value = 'Checking for updates...'
  try {
    const update = await check()
    if (!update) {
      updateStatus.value = 'You are on the latest version.'
      return
    }
    updateStatus.value = `Downloading ${update.version}...`
    await update.downloadAndInstall()
    updateStatus.value = 'Installed. Restarting...'
    await relaunch()
  } catch (err: any) {
    console.error('[Updater] Failed:', err)
    updateStatus.value = `Update failed: ${err?.message ?? err}`
  } finally {
    updateBusy.value = false
  }
}
</script>

<template>
  <main class="container">
    <header v-if="userStore.isLoggedIn" class="app-header">
      <button class="logout-button" @click="onLogout">Logout</button>
    </header>

    <h2>StreamSnipe</h2>

    <div v-if="userStore.hydrating" class="loading">Loading...</div>
    <router-view v-else />

    <footer v-if="userStore.isLoggedIn" class="app-footer">
      <button class="update-button" @click="checkForUpdate" :disabled="updateBusy">
        Check for updates
      </button>
      <span v-if="updateStatus" class="update-status">{{ updateStatus }}</span>
    </footer>
  </main>
</template>

<style scoped>
.app-header {
  position: fixed;
  top: 0;
  right: 0;
  padding: 0.75rem 1rem;
  z-index: 10;
}
.logout-button {
  border-radius: 9999px;
  background-color: transparent;
  border: 1px solid #888;
  color: #ccc;
  font-size: 0.85rem;
  padding: 0.4rem 1rem;
  cursor: pointer;
}
.logout-button:hover {
  border-color: #ccc;
  color: white;
}
.app-footer {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem;
  z-index: 10;
}
.update-button {
  border-radius: 9999px;
  background-color: transparent;
  border: 1px solid #888;
  color: #ccc;
  font-size: 0.85rem;
  padding: 0.4rem 1rem;
  cursor: pointer;
}
.update-button:hover:not(:disabled) {
  border-color: #ccc;
  color: white;
}
.update-button:disabled {
  opacity: 0.5;
  cursor: default;
}
.update-status {
  color: #ccc;
  font-size: 0.85rem;
}
.loading {
  color: white;
}
.logo.vite:hover {
  filter: drop-shadow(0 0 2em #747bff);
}
.logo.vue:hover {
  filter: drop-shadow(0 0 2em #249b73);
}
</style>
<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  font-weight: 400;

  color: #0f0f0f;
  background-color: #f6f6f6;

  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  -webkit-text-size-adjust: 100%;
}

.container {
  margin: 0;
  padding-top: 10vh;
  display: flex;
  flex-direction: column;
  justify-content: center;
  text-align: center;
}

.logo {
  height: 6em;
  padding: 1.5em;
  will-change: filter;
  transition: 0.75s;
}

.logo.tauri:hover {
  filter: drop-shadow(0 0 2em #24c8db);
}

.row {
  display: flex;
  justify-content: center;
}

h1 {
  text-align: center;
}

input,
button {
  border-radius: 8px;
  border: 1px solid transparent;
  padding: 0.6em 1.2em;
  font-size: 1em;
  font-weight: 500;
  font-family: inherit;
  color: #0f0f0f;
  background-color: #ffffff;
  transition: border-color 0.25s;
  box-shadow: 0 2px 2px rgba(0, 0, 0, 0.2);
}

button {
  cursor: pointer;
}

button:hover {
  border-color: #396cd8;
}
button:active {
  border-color: #396cd8;
  background-color: #e8e8e8;
}

input,
button {
  outline: none;
}

.app-input {
  background-color: white;
  color: black;
  margin: 0.5em;
}

@media (prefers-color-scheme: dark) {
  :root {
    color: #f6f6f6;
    background-color: rgb(9, 9, 11);
  }

  a:hover {
    color: #24c8db;
  }

  input,
  button {
    color: #ffffff;
    background-color: rgb(31, 41, 55);
  }
  button:active {
    background-color: #0f0f0f69;
  }
}

</style>
