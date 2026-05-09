import { defineStore } from 'pinia'
import { load, Store } from '@tauri-apps/plugin-store'
import { useHttp } from '../composables/useHttp'

const STORE_FILE = 'auth.json'

let storePromise: Promise<Store> | null = null
function getStore(): Promise<Store> {
    if (!storePromise) {
        storePromise = load(STORE_FILE, { autoSave: false, defaults: {} })
    }
    return storePromise
}

export const useUserStore = defineStore('user', {
    state: () => ({
        name: '',
        sessionHash: '',
        isLoggedIn: false,
        hydrating: true,
    }),
    actions: {
        async login(name: string, sessionHash: string) {
            this.name = name
            this.sessionHash = sessionHash
            this.isLoggedIn = true
            const store = await getStore()
            await store.set('name', name)
            await store.set('sessionHash', sessionHash)
            await store.save()
        },
        async logout() {
            this.name = ''
            this.sessionHash = ''
            this.isLoggedIn = false
            try {
                const store = await getStore()
                await store.delete('name')
                await store.delete('sessionHash')
                await store.save()
            } catch (err) {
                console.error('[userStore] Failed to clear persisted auth:', err)
            }
        },
        async hydrate() {
            try {
                const store = await getStore()
                const name = await store.get<string>('name')
                const sessionHash = await store.get<string>('sessionHash')
                if (!name || !sessionHash) {
                    return
                }
                try {
                    const response = await useHttp('GET', 'user', undefined, sessionHash)
                    if (response.ok) {
                        this.name = name
                        this.sessionHash = sessionHash
                        this.isLoggedIn = true
                    } else {
                        await this.logout()
                    }
                } catch (err) {
                    console.error('[userStore] Session validation failed:', err)
                    await this.logout()
                }
            } finally {
                this.hydrating = false
            }
        },
    },
})
