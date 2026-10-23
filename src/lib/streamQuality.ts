// Broadcast quality presets, ported from the site's resources/js/lib/streamQuality.js.
// Free plans stream SD, plus HD while a weekly HD grant is active (useHdGrant);
// unlimited plans stream HD. The plan comes only from the server's
// GET /api/stream-quality `unlimited` flag, never from tier ids.

export type QualityId = '720p30' | '1080p60';

export interface QualityPreset {
  id: QualityId;
  width: number;
  height: number;
  fps: number;
  maxBitrate: number; // bps
}

export const QUALITY_PRESETS: Record<QualityId, QualityPreset> = {
  '720p30':  { id: '720p30',  width: 1280, height: 720,  fps: 30, maxBitrate: 3_500_000 },
  '1080p60': { id: '1080p60', width: 1920, height: 1080, fps: 60, maxBitrate: 8_000_000 },
};
export const SD: QualityId = '720p30';
export const HD: QualityId = '1080p60';
export const MIN_HD_SECONDS = 60; // less than this left = used up (HdBudget::MIN_GRANT_SECONDS)

// The GStreamer stand-in for the web's encodingFor(): the encoder input caps and
// bitrate (StreamQuality in src-tauri/src/gstreamer.rs). videoscale/d3d11convert
// do the scale-down, so no source height is needed.
export function encoderParams(p: QualityPreset) {
  return { width: p.width, height: p.height, fps: p.fps, bitrateKbps: p.maxBitrate / 1000 };
}

export const PRICING_URL = 'https://streamsnipe.live/pricing';

// Site wording (resources/js/locales/en.json, quality.*).
export const COPY = {
  label: 'Quality',
  hdLeft: (minutes: number) => `${minutes} min of HD left this week`,
  hdUsedShort: (day: string) => `HD resets ${day}`,
  noticeUsedUp: 'Your weekly HD hour is used up — now streaming in 720p30.',
  noticeUnavailable: 'No HD time left this week — streaming in 720p30.',
  upgrade: 'Upgrade to Pro',
};

// Plan indicators shown to limited (Free) plans only.
export const PLAN_COPY = {
  freePlan: 'Free plan',
  goPro: 'Go Pro',
  sdAlways: 'Streams in 720p30',
  hdWeekly: (left: number, limit: number) => `1080p60: ${left} of ${limit} min left this week`,
  hdResets: (day: string) => `HD resets ${day}`,
  autoposts: (n: number) => `Autoposts: ${n} per destination per day`,
  proPitch: 'Pro: unlimited 1080p60 and autoposts',
  hdLeftLive: 'HD left',
};
