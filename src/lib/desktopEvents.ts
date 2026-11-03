// The desktop app's /events socket: a notify-only connection to the signalling
// server, open the whole time the user is signed in (desktop-mcp-remote-control-spec.md).
// While it is open the server counts the app as online, and an AI assistant
// connected over MCP can start and stop the broadcast through it. Only sockets
// opened with the `app` token (the one POST /api/login gives us) get these.
import { ref } from 'vue';
import {
    isPermissionGranted,
    requestPermission,
    sendNotification,
} from '@tauri-apps/plugin-notification';
// @ts-ignore — JS module, no .d.ts
import { createWebSocketTransport } from '../rpc/WebSocketTransport';
// @ts-ignore — JS module, no .d.ts
import { Method } from '../rpc/schema';
// @ts-ignore — JS module, no .d.ts
import { loadConfig } from '../config.js';
import { useUserStore } from '../stores/userStore';
import { useSettingsStore } from '../stores/settingsStore';
import { useHttp } from '../composables/useHttp';
import { appVersion } from './updater';

const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;
// How long a StartStream that arrived away from the Start view waits for it to mount.
const PENDING_START_MS = 5000;

export interface RemoteStart {
    requestId: string;
    access: 'public' | 'private';
    requestedBy: string;
}

export interface RemoteStop {
    requestId: string;
    requestedBy: string;
}

export interface RemoteHandlers {
    start: (cmd: RemoteStart) => Promise<void>;
    stop: (cmd: RemoteStop) => Promise<void>;
}

export interface DesktopEventsHooks {
    // The token was refused: sign out and go to the login screen.
    onUnauthorized: () => Promise<void>;
    // A StartStream arrived while the Start view isn't mounted (e.g. the editor is open).
    openStartView: () => Promise<void>;
}

export const eventsConnected = ref(false);

let hooks: DesktopEventsHooks | null = null;
let handlers: RemoteHandlers | null = null;
let pendingStart: { cmd: RemoteStart; timer: number } | null = null;

let running = false;
// Bumped by stop(): a connect still in flight from an older run closes what it opened.
let runId = 0;
let closeTransport: (() => void) | null = null;
let retries = 0;
let retryTimer: number | null = null;

// One line per command, with the request id the assistant's error message quotes.
export function logRemote(method: 'StartStream' | 'StopStream', requestId: string, outcome: string) {
    console.log(`[remote] ${method} request_id=${requestId || '-'} outcome=${outcome}`);
}

let permission: Promise<boolean> | null = null;

export async function notify(body: string) {
    try {
        if (!permission) {
            permission = (async () =>
                (await isPermissionGranted()) || (await requestPermission()) === 'granted')();
        }
        if (await permission) sendNotification({ title: 'StreamSnipe', body });
    } catch (err) {
        console.warn('[remote] notification failed:', err);
    }
}

// Start.vue registers on mount and passes null on unmount.
export function setRemoteHandlers(next: RemoteHandlers | null) {
    handlers = next;
    if (next && pendingStart) {
        const { cmd, timer } = pendingStart;
        clearTimeout(timer);
        pendingStart = null;
        next.start(cmd).catch((err) => logRemote('StartStream', cmd.requestId, `failed: ${err?.message ?? err}`));
    }
}

function allowed(method: 'StartStream' | 'StopStream', requestId: string) {
    if (useSettingsStore().allowAssistantControl) return true;
    logRemote(method, requestId, 'ignored_setting_off');
    return false;
}

function onStartStream(req: any) {
    const cmd: RemoteStart = {
        requestId: req.requestId() ?? '',
        access: req.access() === 'private' ? 'private' : 'public',
        requestedBy: req.requestedBy() || 'An AI assistant',
    };
    console.log(`[remote] StartStream received request_id=${cmd.requestId} access=${cmd.access} by=${cmd.requestedBy}`);
    if (!allowed('StartStream', cmd.requestId)) return;

    if (handlers) {
        handlers.start(cmd).catch((err) => logRemote('StartStream', cmd.requestId, `failed: ${err?.message ?? err}`));
        return;
    }

    // Not on the Start view: open it and run the command once it registers.
    if (pendingStart) {
        clearTimeout(pendingStart.timer);
        logRemote('StartStream', pendingStart.cmd.requestId, 'failed: superseded');
    }
    const timer = window.setTimeout(() => {
        if (pendingStart?.cmd !== cmd) return;
        pendingStart = null;
        logRemote('StartStream', cmd.requestId, 'failed: start view not ready');
    }, PENDING_START_MS);
    pendingStart = { cmd, timer };
    hooks?.openStartView().catch((err) => console.warn('[remote] could not open the start view:', err));
}

function onStopStream(req: any) {
    const cmd: RemoteStop = {
        requestId: req.requestId() ?? '',
        requestedBy: req.requestedBy() || 'An AI assistant',
    };
    console.log(`[remote] StopStream received request_id=${cmd.requestId} by=${cmd.requestedBy}`);
    if (!allowed('StopStream', cmd.requestId)) return;

    // No Start view means no broadcast (leaving it tears the stream down).
    if (!handlers) {
        logRemote('StopStream', cmd.requestId, 'ignored_not_live');
        return;
    }
    handlers.stop(cmd).catch((err) => logRemote('StopStream', cmd.requestId, `failed: ${err?.message ?? err}`));
}

async function connect() {
    const run = runId;
    if (!running) return;
    const token = useUserStore().sessionHash;
    if (!token) return;

    let url = '';
    try {
        const [config, version] = await Promise.all([loadConfig(), appVersion()]);
        url = `${config.sfuWsUrl}/events?v=${encodeURIComponent(version)}&auth=`;
        const transport = await createWebSocketTransport(url, token, () => {
            if (run !== runId) return;
            closeTransport = null;
            eventsConnected.value = false;
            console.warn('[remote] events socket closed; reconnecting.');
            scheduleReconnect();
        });
        if (run !== runId || !running) {
            transport.close();
            return;
        }
        closeTransport = transport.close;
        eventsConnected.value = true;
        retries = 0;
        // Re-registered on every connect: each socket gets a fresh FbRpc.
        transport.rpc.registerHandler(Method.StartStream, onStartStream);
        transport.rpc.registerHandler(Method.StopStream, onStopStream);
        console.log('[remote] events socket open');
    } catch {
        if (run !== runId) return;
        console.warn(`[remote] could not reach ${url ? url.replace(/auth=.*/, 'auth=…') : 'the events socket'} (attempt ${retries + 1}); retrying.`);
        // The server refuses a bad token with an HTTP 401 at the upgrade, which a
        // WebSocket can't see. Ask the API instead, and log out like any other 401.
        if (await tokenRejected(token)) {
            if (run !== runId) return;
            console.warn('[remote] token rejected by server, logging out');
            stopDesktopEvents();
            await hooks?.onUnauthorized();
            return;
        }
        scheduleReconnect();
    }
}

async function tokenRejected(token: string) {
    try {
        const response = await useHttp('GET', 'user', undefined, token);
        return response.status === 401 || response.status === 403;
    } catch {
        return false; // Network down: keep retrying.
    }
}

function scheduleReconnect() {
    if (!running || retryTimer !== null) return;
    const delay = Math.min(RECONNECT_BASE_MS * 2 ** retries, RECONNECT_MAX_MS);
    retries += 1;
    retryTimer = window.setTimeout(() => {
        retryTimer = null;
        connect();
    }, delay);
}

// Called when the user is signed in. Idempotent.
export function startDesktopEvents(next: DesktopEventsHooks) {
    hooks = next;
    if (running) return;
    running = true;
    retries = 0;
    connect();
}

// Called on sign-out. Closing the socket is what tells the assistant the app is offline.
export function stopDesktopEvents() {
    running = false;
    runId += 1;
    if (retryTimer !== null) clearTimeout(retryTimer);
    retryTimer = null;
    if (pendingStart) {
        clearTimeout(pendingStart.timer);
        pendingStart = null;
    }
    eventsConnected.value = false;
    try { closeTransport?.(); } catch { /* already closed */ }
    closeTransport = null;
}
