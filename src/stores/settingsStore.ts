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

// The capture source the user last chose themselves, so a remote start (AI
// assistant) uses it after a restart. Window handles don't survive a restart, so
// windows are matched by process name and title. The auto-selected primary
// monitor is never saved: an assistant must not stream a desktop nobody chose.
export type SavedSource =
    | { kind: 'monitor'; index: number; name: string }
    | { kind: 'window'; processName: string; title: string }

export interface SavedMic {
    on: boolean
    deviceId: string
}

function isSavedSource(v: any): v is SavedSource {
    return (v?.kind === 'monitor' && typeof v.index === 'number' && typeof v.name === 'string')
        || (v?.kind === 'window' && typeof v.processName === 'string' && typeof v.title === 'string')
}

export const useSettingsStore = defineStore('settings', {
    state: () => ({
        recordingPath: '',
        visibility: 'public' as 'public' | 'private',
        // Paid-tier opt-out for the recording watermark. Default false = shown.
        // Ignored for free-tier users (their watermark is always applied).
        hideWatermark: false,
        // "Allow AI assistants to control streaming". Default on: the user
        // already approved it on the OAuth consent screen.
        allowAssistantControl: true,
        lastSource: null as SavedSource | null,
        lastMic: null as SavedMic | null,
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
                const savedVisibility = await store.get<string>('visibility')
                if (savedVisibility === 'private' || savedVisibility === 'public') {
                    this.visibility = savedVisibility
                }
                const savedHideWatermark = await store.get<boolean>('hideWatermark')
                if (typeof savedHideWatermark === 'boolean') {
                    this.hideWatermark = savedHideWatermark
                }
                const savedAllow = await store.get<boolean>('allowAssistantControl')
                if (typeof savedAllow === 'boolean') {
                    this.allowAssistantControl = savedAllow
                }
                const savedSource = await store.get('lastSource')
                if (isSavedSource(savedSource)) {
                    this.lastSource = savedSource
                }
                const savedMic = await store.get<SavedMic>('lastMic')
                if (typeof savedMic?.on === 'boolean' && typeof savedMic?.deviceId === 'string') {
                    this.lastMic = savedMic
                }
            } catch (err) {
                console.error('[settingsStore] hydrate failed:', err)
                this.recordingPath = await defaultRecordingPath()
            } finally {
                this._hydrated = true
            }
        },
        async setVisibility(visibility: 'public' | 'private') {
            this.visibility = visibility
            try {
                const store = await getStore()
                await store.set('visibility', visibility)
                await store.save()
            } catch (err) {
                console.error('[settingsStore] Failed to persist visibility:', err)
            }
        },
        async setHideWatermark(hide: boolean) {
            this.hideWatermark = hide
            try {
                const store = await getStore()
                await store.set('hideWatermark', hide)
                await store.save()
            } catch (err) {
                console.error('[settingsStore] Failed to persist hideWatermark:', err)
            }
        },
        async setAllowAssistantControl(allow: boolean) {
            this.allowAssistantControl = allow
            await this.persist('allowAssistantControl', allow)
        },
        async setLastSource(source: SavedSource) {
            this.lastSource = source
            await this.persist('lastSource', source)
        },
        async setLastMic(mic: SavedMic) {
            this.lastMic = mic
            await this.persist('lastMic', mic)
        },
        async persist(key: string, value: unknown) {
            try {
                const store = await getStore()
                await store.set(key, value)
                await store.save()
            } catch (err) {
                console.error(`[settingsStore] Failed to persist ${key}:`, err)
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
