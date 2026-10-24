import { ref } from 'vue';
import { getVersion } from '@tauri-apps/api/app';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getCurrentWindow } from '@tauri-apps/api/window';

export const DOWNLOAD_URL = 'https://streamsnipe.live/download';

// Set when the server answers 426 (this version is below its minimum). The
// router then pins the app to /update-required. Holds the server's message.
export const updateRequired = ref<string | null>(null);

// Found by the quiet background check; drives the optional "update available" banner.
export const availableUpdate = ref<Update | null>(null);
// The version whose banner the user closed. A newer release shows it again.
export const dismissedVersion = ref<string | null>(null);
// True while connecting or live (set by Start.vue). Installing restarts the app,
// so the banner stays hidden until the stream ends.
export const streaming = ref<boolean>(false);

export const updateStatus = ref<string>('');
export const updateBusy = ref<boolean>(false);
// 0..1 while downloading, null when the size is unknown or nothing is running.
export const updateProgress = ref<number | null>(null);

let versionPromise: Promise<string> | null = null;
export function appVersion(): Promise<string> {
    if (!versionPromise) versionPromise = getVersion().catch(() => '0.0.0');
    return versionPromise;
}

export function requireUpdate(message?: string) {
    updateRequired.value = message || 'This version of StreamSnipe is no longer supported.';
}

const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;
// Focus fires often; at most one check per this gap.
const MIN_CHECK_GAP_MS = 30 * 60 * 1000;
let lastCheckAt = 0;

// Background check: never throws, never installs on its own.
export async function checkQuietly() {
    if (updateBusy.value || updateRequired.value) return;
    if (Date.now() - lastCheckAt < MIN_CHECK_GAP_MS) return;
    lastCheckAt = Date.now();
    try {
        const update = await check();
        // A check that lands mid-install must not swap the update being installed.
        if (!updateBusy.value) availableUpdate.value = update;
    } catch (err) {
        console.warn('[Updater] Background check failed:', err);
    }
}

// Check now, every few hours, and whenever the window gains focus, so a
// release is noticed by apps left open for days. Returns the cleanup.
export function startUpdateChecks(): () => void {
    checkQuietly();
    const timer = setInterval(checkQuietly, CHECK_INTERVAL_MS);
    let unlisten: (() => void) | null = null;
    let stopped = false;
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) checkQuietly();
    }).then(u => { if (stopped) u(); else unlisten = u; }).catch(() => {});
    return () => {
        stopped = true;
        clearInterval(timer);
        unlisten?.();
    };
}

// Check, download, install, relaunch. Returns false if it failed or there was
// nothing to install; updateStatus says which.
export async function installUpdate(): Promise<boolean> {
    if (updateBusy.value) return false;
    updateBusy.value = true;
    updateProgress.value = null;
    updateStatus.value = 'Checking for updates…';
    try {
        const update = availableUpdate.value ?? await check();
        if (!update) {
            updateStatus.value = 'You are on the latest version.';
            return false;
        }
        updateStatus.value = `Downloading ${update.version}…`;
        let total = 0;
        let received = 0;
        await update.downloadAndInstall((event) => {
            if (event.event === 'Started') {
                total = event.data.contentLength ?? 0;
            } else if (event.event === 'Progress') {
                received += event.data.chunkLength;
                if (total > 0) updateProgress.value = Math.min(received / total, 1);
            } else if (event.event === 'Finished') {
                updateProgress.value = 1;
            }
        });
        updateStatus.value = 'Installed. Restarting…';
        await relaunch();
        return true;
    } catch (err: any) {
        console.error('[Updater] Failed:', err);
        updateStatus.value = `Update failed: ${err?.message ?? err}`;
        return false;
    } finally {
        updateBusy.value = false;
    }
}
