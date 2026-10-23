import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useHttp } from './useHttp';
import { useUserStore } from '../stores/userStore';
import { useQualityStore } from '../stores/qualityStore';
import { HD, SD, QUALITY_PRESETS, encoderParams, type QualityId } from '../lib/streamQuality';

export type HdNotice = '' | 'hd_unavailable' | 'hd_used_up';

interface HdGrant {
  id: number;
  // Both clocks: the monotonic one can't be moved by changing the wall clock, but
  // may pause during system sleep, which the wall-clock one catches.
  deadlineMono: number;
  deadlineWall: number;
}

const WATCHDOG_MS = 30_000;
const LOCK_RETRY_MS = 500;

function sleep(ms: number) {
  return new Promise<void>(r => setTimeout(r, ms));
}

// The weekly HD grant for one broadcast at a time, ported from the site's
// useBroadcast.js (claimQuality / scheduleHdEnd / releaseHd). The server counts
// from the moment it grants, so the deadline is set as soon as the grant
// arrives, and HD ends in place (set_stream_quality) when it passes.
export function useHdGrant() {
  const userStore = useUserStore();
  const qualityStore = useQualityStore();

  const notice = ref<HdNotice>('');
  // For display while live: the quality going out, and when HD ends (wall-clock
  // ms; null = no grant, i.e. SD or unlimited HD).
  const liveQuality = ref<QualityId>(SD);
  const hdEndsAt = ref<number | null>(null);
  let grant: HdGrant | null = null;
  let hdTimer: number | null = null;
  let watchdog: number | null = null;
  // The grant request in flight, so a release (or app quit) during it waits for
  // the answer and stops what it granted. Bumping `epoch` in release() tells the
  // pending claim that its broadcast was torn down.
  let pending: Promise<QualityId> | null = null;
  let epoch = 0;

  function api(method: string, url: string, data?: object) {
    return useHttp(method, url, data, userStore.sessionHash);
  }

  async function readJson(res: Response): Promise<any> {
    try { return await res.json(); } catch { return null; }
  }

  function cancelled() {
    const err = new Error('Stream start cancelled');
    err.name = 'AbortError';
    return err;
  }

  // Returns the quality to go live with. Only HD on a limited plan asks the server.
  // Throws AbortError if released while the request was in flight (the release
  // has already stopped the grant).
  async function claimQuality(picked: QualityId): Promise<QualityId> {
    notice.value = '';
    const myEpoch = epoch;
    const request = requestGrant(picked);
    pending = request;
    try {
      const quality = await request;
      if (myEpoch !== epoch) throw cancelled();
      liveQuality.value = quality;
      hdEndsAt.value = grant?.deadlineWall ?? null;
      return quality;
    } finally {
      if (pending === request) pending = null;
    }
  }

  async function requestGrant(picked: QualityId): Promise<QualityId> {
    if (picked !== HD) return SD;
    if (qualityStore.unlimited) return HD;
    try {
      let res = await api('POST', 'stream-quality/hd');
      if (res.status === 409) {
        await sleep(LOCK_RETRY_MS);
        res = await api('POST', 'stream-quality/hd');
      }
      const body = await readJson(res);
      if (res.status === 201 && typeof body?.id === 'number' && typeof body?.granted_seconds === 'number') {
        const ms = body.granted_seconds * 1000;
        grant = { id: body.id, deadlineMono: performance.now() + ms, deadlineWall: Date.now() + ms };
        qualityStore.setStatus(body.status);
        return HD;
      }
      if (res.status === 200 && body?.id == null) {
        qualityStore.setStatus(body?.status);
        return HD;
      }
      qualityStore.setStatus(body?.status);
      console.warn('[HD] grant refused:', res.status, body?.code ?? body?.message);
    } catch (err) {
      console.warn('[HD] grant request failed:', err);
    }
    notice.value = 'hd_unavailable';
    return SD;
  }

  function clearTimers() {
    if (hdTimer != null) { clearTimeout(hdTimer); hdTimer = null; }
    if (watchdog != null) { clearInterval(watchdog); watchdog = null; }
    document.removeEventListener('visibilitychange', checkDeadline);
  }

  function checkDeadline() {
    if (grant && (performance.now() >= grant.deadlineMono || Date.now() >= grant.deadlineWall)) {
      expire();
    }
  }

  // The broadcast is up. Link the grant to its stream session, so the server
  // stops counting when the SFU ends it even if `stop` never arrives, and arm
  // the HD end.
  function onProducer(producerId: string) {
    if (!grant) return;
    api('POST', `stream-quality/hd/${grant.id}/attach`, { producer_id: producerId })
      .catch((err) => console.warn('[HD] attach failed:', err));
    clearTimers();
    hdTimer = window.setTimeout(expire, Math.max(0, grant.deadlineMono - performance.now()));
    watchdog = window.setInterval(checkDeadline, WATCHDOG_MS);
    document.addEventListener('visibilitychange', checkDeadline);
    checkDeadline();
  }

  // The grant ran out mid-stream: drop to SD without touching the producer.
  async function expire() {
    if (!grant) return;
    grant = null;
    clearTimers();
    liveQuality.value = SD;
    hdEndsAt.value = null;
    try {
      await invoke('set_stream_quality', { quality: encoderParams(QUALITY_PRESETS[SD]) });
    } catch (err) {
      console.error('[HD] quality_downgrade_failed:', err);
    }
    notice.value = 'hd_used_up';
    qualityStore.fetchStatus();
  }

  // Close the books. Called on every end path; never throws.
  async function release() {
    epoch++;
    clearTimers();
    if (pending) await pending.catch(() => {});
    hdEndsAt.value = null;
    const g = grant;
    grant = null;
    if (!g) return;
    try {
      const res = await api('POST', `stream-quality/hd/${g.id}/stop`);
      const body = await readJson(res);
      qualityStore.setStatus(body?.status);
    } catch (err) {
      console.warn('[HD] stop failed:', err);
    }
  }

  // For app quit: don't hold the window open for long. If `stop` never gets
  // through, `attach` plus the SFU ending the session still close the books.
  function releaseWithTimeout(ms = 1000) {
    return Promise.race([release(), sleep(ms)]);
  }

  // A grant held, or one that may be on its way.
  function hasGrant() {
    return grant !== null || pending !== null;
  }

  return { notice, liveQuality, hdEndsAt, claimQuality, onProducer, release, releaseWithTimeout, hasGrant };
}
