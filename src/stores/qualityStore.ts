import { defineStore } from 'pinia'
import { useHttp } from '../composables/useHttp'
import { useUserStore } from './userStore'
import { HD, SD, MIN_HD_SECONDS, type QualityId } from '../lib/streamQuality'

// GET /api/stream-quality. On unlimited plans the other fields are null.
export interface QualityStatus {
    unlimited: boolean
    limit_seconds: number | null
    used_seconds: number | null
    remaining_seconds: number | null
    resets_at: string | null
}

// Offline or a failed call: Free with nothing left (720p30 only), the safe side.
const FAIL_CLOSED: QualityStatus = {
    unlimited: false,
    limit_seconds: null,
    used_seconds: null,
    remaining_seconds: 0,
    resets_at: null,
}

// The server's HD books plus the go-live picker. The pick is kept for the
// session only, so a Free user starts each app run on 720p30.
export const useQualityStore = defineStore('quality', {
    state: () => ({
        status: { ...FAIL_CLOSED } as QualityStatus,
        picked: null as QualityId | null,
        // The books came from the server. Until then the fail-closed status
        // only limits the picker; plan indicators ("Free plan", Go Pro) wait
        // for this so an offline Pro user isn't told they're on Free.
        loaded: false,
    }),
    getters: {
        unlimited: (state) => state.status.unlimited,
        // Known to be on a limited (Free) plan.
        limited: (state) => state.loaded && !state.status.unlimited,
        hdLimitMinutes: (state) => Math.floor((state.status.limit_seconds ?? 0) / 60),
        hdAvailable: (state) =>
            state.status.unlimited || (state.status.remaining_seconds ?? 0) >= MIN_HD_SECONDS,
        hdLeftMinutes: (state) => Math.floor((state.status.remaining_seconds ?? 0) / 60),
        // Weekday of the reset in local time, e.g. "Mon".
        resetsDay: (state) => {
            if (!state.status.resets_at) return ''
            const d = new Date(state.status.resets_at)
            return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString(undefined, { weekday: 'short' })
        },
        // Unlimited defaults to HD; Free to SD so a test stream doesn't spend
        // the hour. A disabled HD choice falls back to SD.
        effectivePick(): QualityId {
            const pick = this.picked ?? (this.status.unlimited ? HD : SD)
            return pick === HD && !this.hdAvailable ? SD : pick
        },
    },
    actions: {
        async fetchStatus() {
            const token = useUserStore().sessionHash
            if (!token) { this.status = { ...FAIL_CLOSED }; this.loaded = false; return }
            try {
                const res = await useHttp('GET', 'stream-quality', undefined, token)
                if (!res.ok) throw new Error(`HTTP ${res.status}`)
                this.setStatus(await res.json())
            } catch (err) {
                console.warn('[qualityStore] fetchStatus failed:', err)
                this.status = { ...FAIL_CLOSED }
                this.loaded = false
            }
        },
        // From a /stream-quality response or the `status` in a grant/stop/403 body.
        setStatus(s: unknown) {
            if (!s || typeof s !== 'object' || typeof (s as any).unlimited !== 'boolean') return
            this.status = { ...FAIL_CLOSED, ...(s as QualityStatus) }
            this.loaded = true
        },
        pick(q: QualityId) {
            this.picked = q
        },
    },
})
