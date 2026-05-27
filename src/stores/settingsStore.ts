import { defineStore } from 'pinia'
import { load, Store } from '@tauri-apps/plugin-store'
import { videoDir, homeDir } from '@tauri-apps/api/path'

const STORE_FILE = 'settings.json'

let storePromise: Promise<Store> | null = null
function getStore(): Promise<Store> {
    if (!storePromise) {
        storePromise = load(STORE_FILE, { autoSave: false, defaults: {} })
    }
    return storePromise
}

async function defaultRecordingPath(): Promise<string> {
    try {
        return await videoDir()
    } catch {
        return await homeDir()
    }
}

export const useSettingsStore = defineStore('settings', {
    state: () => ({
        recordingPath: '',
        _hydrated: false,
    }),
    actions: {
        async hydrate() {
            try {
                const store = await getStore()
                const saved = await store.get<string>('recordingPath')
                if (saved) {
                    this.recordingPath = saved
                } else {
                    this.recordingPath = await defaultRecordingPath()
                    await store.set('recordingPath', this.recordingPath)
                    await store.save()
                }
            } catch (err) {
                console.error('[settingsStore] hydrate failed:', err)
                this.recordingPath = await defaultRecordingPath()
            } finally {
                this._hydrated = true
            }
        },
        async setRecordingPath(path: string) {
            this.recordingPath = path
            try {
                const store = await getStore()
                await store.set('recordingPath', path)
                await store.save()
            } catch (err) {
                console.error('[settingsStore] Failed to persist recordingPath:', err)
            }
        },
    },
})
