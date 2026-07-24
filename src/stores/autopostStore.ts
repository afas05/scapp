import { defineStore } from 'pinia'
import { load, Store } from '@tauri-apps/plugin-store'
import { useHttp } from '../composables/useHttp'
import { useUserStore } from './userStore'

const STORE_FILE = 'autopost.json'

let storePromise: Promise<Store> | null = null
function getStore(): Promise<Store> {
    if (!storePromise) {
        storePromise = load(STORE_FILE, { autoSave: false, defaults: {} })
    }
    return storePromise
}

export type Privacy = 'private' | 'unlisted' | 'public'
export type Service = 'youtube' | 'instagram' | 'tiktok'

export interface ProviderStatus {
    configured: boolean
    connected: boolean
    needs_reconsent: boolean
    accounts: { id: number; account_name: string; status: string }[]
    connect_url: string | null
}

export interface AutopostStatus {
    providers: Record<Service, ProviderStatus>
    settings: {
        default_services: Service[]
        default_privacy: Privacy
        watermark: unknown
    }
}

// Store for the clip-studio autopost feature: caches the server's default
// services/privacy (from GET /autopost/settings) and remembers the last-used
// form values locally so the post modal is pre-filled. Mirrors the persistence
// pattern used by settingsStore.
export const useAutopostStore = defineStore('autopost', {
    state: () => ({
        defaultServices: ['youtube'] as Service[],
        defaultPrivacy: 'private' as Privacy,
        lastPrivacy: 'private' as Privacy,
        lastServices: ['youtube'] as Service[],
        lastDescription: '',
        _hydrated: false,
    }),
    actions: {
        async hydrate() {
            if (this._hydrated) return
            try {
                const store = await getStore()
                const p = await store.get<Privacy>('lastPrivacy')
                if (p) this.lastPrivacy = p
                const s = await store.get<Service[]>('lastServices')
                if (Array.isArray(s)) this.lastServices = s
                const d = await store.get<string>('lastDescription')
                if (typeof d === 'string') this.lastDescription = d
            } catch (err) {
                console.error('[autopostStore] hydrate failed:', err)
            } finally {
                this._hydrated = true
            }
        },
        // Pull the account's default services/privacy from the server. Silently
        // no-ops on failure — the local defaults stay in effect.
        async fetchServerSettings() {
            const token = useUserStore().sessionHash
            if (!token) return
            try {
                const res = await useHttp('GET', 'autopost/settings', undefined, token)
                if (!res.ok) return
                const data = await res.json()
                if (Array.isArray(data?.default_services) && data.default_services.length) {
                    this.defaultServices = data.default_services
                    if (!this.lastServices.length) this.lastServices = data.default_services
                }
                if (data?.default_privacy) {
                    this.defaultPrivacy = data.default_privacy
                }
            } catch (err) {
                console.warn('[autopostStore] fetchServerSettings failed:', err)
            }
        },
        async persistLast(privacy: Privacy, services: Service[], description: string) {
            this.lastPrivacy = privacy
            this.lastServices = services
            this.lastDescription = description
            try {
                const store = await getStore()
                await store.set('lastPrivacy', privacy)
                await store.set('lastServices', services)
                await store.set('lastDescription', description)
                await store.save()
            } catch (err) {
                console.error('[autopostStore] persistLast failed:', err)
            }
        },
    },
})
