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

// The user id (the broadcaster id the chat ban endpoints take), as a positive
// integer. Accepts a numeric string too, since chat compares ids strictly.
function toUserId(value: unknown): number | undefined {
    const id = typeof value === 'string' ? Number(value) : value
    return typeof id === 'number' && Number.isInteger(id) && id > 0 ? id : undefined
}

function extractUserId(data: any): number | undefined {
    return toUserId(data?.id) ?? toUserId(data?.user?.id)
}

export const useUserStore = defineStore('user', {
    state: () => ({
        name: '',
        // users.id; the broadcaster id for chat moderation. 0 = unknown.
        userId: 0,
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
        async login(name: string, sessionHash: string, userId?: unknown) {
            this.name = name
            this.userId = toUserId(userId) ?? 0
            this.sessionHash = sessionHash
            this.isLoggedIn = true
            const store = await getStore()
            await store.set('name', name)
            await store.set('userId', this.userId)
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
                    const data = await response.json()
                    const plan = extractPlan(data)
                    if (plan !== undefined) this.subscription_tier_id = plan
                    const id = extractUserId(data)
                    if (id !== undefined) this.userId = id
                }
            } catch (err) {
                console.warn('[userStore] refreshPlan failed:', err)
            }
        },
        async logout() {
            this.name = ''
            this.userId = 0
            this.sessionHash = ''
            this.isLoggedIn = false
            this.subscription_tier_id = 1
            try {
                const store = await getStore()
                await store.delete('name')
                await store.delete('userId')
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
                // Only trusted alongside a stored session; /api/user refreshes it below.
                this.userId = toUserId(await store.get('userId')) ?? 0
                try {
                    const response = await useHttp('GET', 'user', undefined, sessionHash)
                    console.log('[userStore] hydrate: /api/user status=', response.status)
                    if (response.ok) {
                        this.name = name
                        this.sessionHash = sessionHash
                        this.isLoggedIn = true
                        try {
                            const data = await response.json()
                            this.subscription_tier_id = extractPlan(data) ?? this.subscription_tier_id
                            this.userId = extractUserId(data) ?? this.userId
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
