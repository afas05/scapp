import { ref, watchEffect, onScopeDispose, type Ref } from 'vue';

const SILENCE_FLOOR_DB = -60;

function rmsToDbfs(rms: number): number {
  if (rms <= 1e-7) return -Infinity;
  return 20 * Math.log10(rms);
}

function dbfsToLevel(dbfs: number): number {
  if (!Number.isFinite(dbfs)) return 0;
  const clamped = Math.max(SILENCE_FLOOR_DB, Math.min(0, dbfs));
  return (clamped - SILENCE_FLOOR_DB) / (0 - SILENCE_FLOOR_DB);
}

export interface MicLevel {
  level: Ref<number>;
  dbfs: Ref<number>;
}

// Live mic meter sourced from the browser's Web Audio API. WASAPI shared mode
// lets this open the same device the Rust streaming pipeline holds, so the
// meter keeps working before, during, and after a stream.
export function useMicLevel(
  active: () => boolean,
  deviceLabel?: () => string | undefined,
): MicLevel {
  const resolveLabel = typeof deviceLabel === 'function' ? deviceLabel : () => undefined;
  const level = ref(0);
  const dbfs = ref(-Infinity);

  let raf: number | null = null;
  let stream: MediaStream | null = null;
  let ctx: AudioContext | null = null;
  let analyser: AnalyserNode | null = null;
  let buf: Float32Array | null = null;
  let cancelled = false;

  const teardown = () => {
    cancelled = true;
    if (raf != null) {
      cancelAnimationFrame(raf);
      raf = null;
    }
    if (stream) {
      stream.getTracks().forEach(t => t.stop());
      stream = null;
    }
    if (ctx) {
      ctx.close().catch(() => {});
      ctx = null;
    }
    analyser = null;
    buf = null;
    level.value = 0;
    dbfs.value = -Infinity;
  };

  async function startCapture(label: string | undefined) {
    cancelled = false;
    try {
      // Resolve label → deviceId via enumerateDevices(). Labels are only
      // populated after the page has been granted microphone permission at
      // least once; on the very first call we fall back to the default device
      // (the browser hands us labels after this call succeeds).
      let deviceId: string | undefined;
      if (label && navigator.mediaDevices?.enumerateDevices) {
        const devices = await navigator.mediaDevices.enumerateDevices();
        const match = devices.find(d => d.kind === 'audioinput' && d.label === label);
        deviceId = match?.deviceId;
      }

      const constraints: MediaStreamConstraints = {
        audio: deviceId
          ? { deviceId: { exact: deviceId } }
          : true,
      };
      const s = await navigator.mediaDevices.getUserMedia(constraints);
      if (cancelled) {
        s.getTracks().forEach(t => t.stop());
        return;
      }
      stream = s;
      ctx = new AudioContext();
      const source = ctx.createMediaStreamSource(stream);
      analyser = ctx.createAnalyser();
      analyser.fftSize = 1024;
      analyser.smoothingTimeConstant = 0.6;
      source.connect(analyser);
      buf = new Float32Array(analyser.fftSize);

      const tick = () => {
        if (cancelled || !analyser || !buf) return;
        analyser.getFloatTimeDomainData(buf);
        let sumSq = 0;
        for (let i = 0; i < buf.length; i++) {
          const v = buf[i];
          sumSq += v * v;
        }
        const rms = Math.sqrt(sumSq / buf.length);
        const d = rmsToDbfs(rms);
        dbfs.value = d;
        level.value = dbfsToLevel(d);
        raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    } catch (err) {
      console.warn('[useMicLevel] getUserMedia failed:', err);
      level.value = 0;
      dbfs.value = -Infinity;
    }
  }

  watchEffect(() => {
    teardown();
    if (!active()) return;
    startCapture(resolveLabel());
  });

  onScopeDispose(teardown);

  return { level, dbfs };
}

// Procedural sine + noise envelope, kept for UI placeholders that don't yet
// have a real signal source (e.g. the "game audio" meter on LiveDashboard).
export function useSimulatedLevel(active: () => boolean, target = 0.55): Ref<number> {
  const level = ref(0);
  let raf: number | null = null;
  let t = 0;

  const stop = () => {
    if (raf != null) {
      cancelAnimationFrame(raf);
      raf = null;
    }
  };

  watchEffect(() => {
    stop();
    if (!active()) {
      level.value = 0;
      return;
    }
    const tick = () => {
      t += 0.05;
      const env = 0.5 + 0.3 * Math.sin(t * 0.7) + 0.15 * Math.sin(t * 2.3 + 1);
      const n = (Math.random() - 0.5) * 0.4;
      level.value = Math.max(0.05, Math.min(0.98, env * target + n));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
  });

  onScopeDispose(stop);

  return level;
}

export function fmtDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(r)}` : `${pad(m)}:${pad(r)}`;
}
