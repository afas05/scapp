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

// Pull the numeric subscription plan out of a /api/user payload, tolerating
// either a top-level `plan` or a nested `user.subscription_tier_id`. Returns undefined if
// neither is present so callers keep the previous value.
function extractPlan(data: any): number | undefined {
    if (typeof data?.subscription_tier_id === 'number') return data.subscription_tier_id
    if (typeof data?.user?.subscription_tier_id === 'number') return data.user.subscription_tier_id
    return undefined
}

export const useUserStore = defineStore('user', {
    state: () => ({
        name: '',
        sessionHash: '',
        isLoggedIn: false,
        hydrating: true,
        // Subscription tier from GET /api/user. 1 = free (watermark forced on);
        // anything else = paid (watermark optional). Default 1 is the safe
        // fallback so an unknown plan is always watermarked.
        subscription_tier_id: 1,
    }),
    getters: {
        // Alias used by components to gate paid-only features (watermark toggle,
        // etc.). Kept in sync with `subscription_tier_id` so a missing/undefined
        // read can't accidentally unlock a paid feature. 1 = free.
        plan: (state) => state.subscription_tier_id,
    },
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
        // Re-fetch the subscription plan from /api/user. Used after a fresh
        // login (the login response does not include the plan) and any time the
        // tier needs refreshing. Failures leave the current plan untouched.
        async refreshPlan() {
            if (!this.sessionHash) return
            try {
                const response = await useHttp('GET', 'user', undefined, this.sessionHash)
                if (response.ok) {
                    const plan = extractPlan(await response.json())
                if (plan !== undefined) this.subscription_tier_id = plan
                }
            } catch (err) {
                console.warn('[userStore] refreshPlan failed:', err)
            }
        },
        async logout() {
            this.name = ''
            this.sessionHash = ''
            this.isLoggedIn = false
            this.subscription_tier_id = 1
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
                console.log('[userStore] hydrate: stored name=', name, 'hasToken=', !!sessionHash)
                if (!name || !sessionHash) {
                    return
                }
                try {
                    const response = await useHttp('GET', 'user', undefined, sessionHash)
                    console.log('[userStore] hydrate: /api/user status=', response.status)
                    if (response.ok) {
                        this.name = name
                        this.sessionHash = sessionHash
                        this.isLoggedIn = true
                        try {
                            this.subscription_tier_id = extractPlan(await response.json()) ?? this.subscription_tier_id
                        } catch { /* keep default plan */ }
                    } else if (response.status === 401 || response.status === 403) {
                        console.warn('[userStore] Token rejected by server (', response.status, '), clearing credentials')
                        await this.logout()
                    } else {
                        console.warn('[userStore] Unexpected status from /api/user:', response.status, '— keeping stored session')
                        this.name = name
                        this.sessionHash = sessionHash
                        this.isLoggedIn = true
                    }
                } catch (err) {
                    console.warn('[userStore] Network error during session validation, keeping stored session:', err)
                    this.name = name
                    this.sessionHash = sessionHash
                    this.isLoggedIn = true
                }
            } finally {
                this.hydrating = false
            }
        },
    },
})
