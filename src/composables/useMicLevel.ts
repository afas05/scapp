import { ref, watchEffect, onScopeDispose } from 'vue';

export function useMicLevel(active: () => boolean, target = 0.55) {
  const level = ref(0.3);
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
