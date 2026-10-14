import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { createWebSocketTransport } from '../rpc/WebSocketTransport';
import { Method } from '../rpc/schema';
import {
    CreateProducerPlainTransportRequestT,
    ProduceRequestT,
    ProducerClosedRequestT,
} from '../rpc/generated/mediasoup-signaling_generated';

// Must match rtph264pay properties in src-tauri/src/gstreamer.rs
const VIDEO_PAYLOAD_TYPE = 100;
const VIDEO_RTX_PAYLOAD_TYPE = 101;
const VIDEO_SSRC = 2222;
const VIDEO_RTX_SSRC = 2223;
const VIDEO_CLOCK_RATE = 90000;

// Must match rtpopuspay properties in src-tauri/src/gstreamer.rs
const AUDIO_PAYLOAD_TYPE = 96;
const AUDIO_SSRC = 1111;
const AUDIO_CLOCK_RATE = 48000;
const AUDIO_CHANNELS = 2;

export function useMediaSoup() {
    const isConnected = ref(false);
    const viewerCount = ref(0);

    let rpc = null;
    let closeTransport = null;
    let videoProducerId = null;
    let audioProducerId = null;
    let pipelineRunning = false;
    // The in-flight start_stream, if any. A teardown waits for it and stops what it
    // brought up, so a pipeline from a cancelled start can't come up after the next
    // session's (the pipeline is global on the Rust side: a late stop would kill it).
    let pipelineStarting = null;

    // Bumped by every teardown. A start sequence or socket from an older
    // generation is stale: it must not touch state, and must undo what it started.
    let generation = 0;
    let endedGeneration = -1;
    let teardown = null;

    let onStreamEnded = null;
    let onProducerCreated = null;
    let onViewerCount = null;
    let onChatMessage = null;
    let onChatDeleted = null;

    function on(event, callback) {
        switch (event) {
            case 'streamEnded': onStreamEnded = callback; break;
            case 'producerCreated': onProducerCreated = callback; break;
            case 'viewerCount': onViewerCount = callback; break;
            case 'chatMessage': onChatMessage = callback; break;
            case 'chatDeleted': onChatDeleted = callback; break;
        }
    }

    function cancelled() {
        const err = new Error('Stream start cancelled');
        err.name = 'AbortError';
        return err;
    }

    // The server ended the broadcast ('ended': moderator or lost media node) or the
    // socket dropped ('disconnected'). Reported once per session; the caller tears down.
    function serverEnded(gen, reason) {
        if (gen !== generation || endedGeneration === gen) return;
        endedGeneration = gen;
        isConnected.value = false;
        if (onStreamEnded) onStreamEnded(reason);
    }

    // sfuWsUrl  = 'wss://host:port'  (no trailing slash, no ?auth=)
    async function init(token, { sfuWsUrl }) {
        const gen = generation;
        const transport = await createWebSocketTransport(
            sfuWsUrl + '?auth=',
            token,
            () => serverEnded(gen, 'disconnected'),
        );
        if (gen !== generation) {
            transport.close();
            throw cancelled();
        }
        rpc = transport.rpc;
        closeTransport = transport.close;
        isConnected.value = true;

        rpc.registerHandler(Method.ViewerCount, (req) => {
            viewerCount.value = req.count();
            if (onViewerCount) onViewerCount(req.count());
        });

        // Chat, saved by Laravel and fanned out by the SFU to the broadcaster's
        // sessions too. Registered before Produce so no push is missed. uint64
        // fields arrive as bigint.
        rpc.registerHandler(Method.ChatMessage, (req) => {
            if (onChatMessage) onChatMessage({
                id: Number(req.id()),
                user_id: req.userId(),
                name: req.name() ?? '',
                body: req.body() ?? '',
                sent_at: Number(req.sentAt()),
            });
        });

        rpc.registerHandler(Method.ChatDeleted, (req) => {
            const ids = [];
            for (let i = 0; i < req.idsLength(); i++) ids.push(Number(req.ids(i)));
            if (onChatDeleted) onChatDeleted(ids);
        });

        rpc.registerHandler(Method.StreamEnded, () => serverEnded(gen, 'ended'));
    }

    async function startSharing(windowHandle = null, processPid = null, micEnabled = false, micDeviceId = null, monitorIndex = null, access = 'public') {
        // A teardown during any await below cancels the start: nulls rpc, closes
        // the socket. Check after each one before going on.
        const gen = generation;
        const alive = () => { if (gen !== generation) throw cancelled(); };
        // A request rejected by the teardown closing the socket is a cancel too.
        const call = async (method, req) => {
            let res;
            try { res = await rpc.request(method, req); } catch (e) { alive(); throw e; }
            alive();
            return res;
        };
        alive();

        // 1a. Video PlainTransport (comedia=true; we just send RTP at it).
        const videoPt = await call(
            Method.CreateProducerPlainTransport,
            new CreateProducerPlainTransportRequestT('video'),
        );

        // 1b. Audio PlainTransport on the same WS session — separate UDP port pair.
        const audioPt = await call(
            Method.CreateProducerPlainTransport,
            new CreateProducerPlainTransportRequestT('audio'),
        );

        // 2a. Declare the video producer to mediasoup. profile-level-id MUST
        //     match the router's H264 codec config exactly — mediasoup's
        //     isSameProfile rejects mismatches even outside strict mode.
        //     Declares both the primary H.264 codec and its RTX (RFC 4588)
        //     sidecar so mediasoup will accept retransmissions sent by our
        //     pipeline. Without the RTX codec entry, mediasoup silently
        //     discards any packet on the RTX SSRC — we'd burn upload bandwidth
        //     on retransmits that never reach the consumer.
        const videoRtpParameters = {
            codecs: [
                {
                    mimeType: 'video/H264',
                    payloadType: VIDEO_PAYLOAD_TYPE,
                    clockRate: VIDEO_CLOCK_RATE,
                    parameters: {
                        'packetization-mode': 1,
                        'profile-level-id': '42e01f', // baseline
                        'level-asymmetry-allowed': 1,
                    },
                    rtcpFeedback: [
                        { type: 'nack' },
                        { type: 'nack', parameter: 'pli' },
                        { type: 'goog-remb' },
                        { type: 'transport-cc' },
                    ],
                },
                {
                    mimeType: 'video/rtx',
                    payloadType: VIDEO_RTX_PAYLOAD_TYPE,
                    clockRate: VIDEO_CLOCK_RATE,
                    parameters: {
                        apt: VIDEO_PAYLOAD_TYPE,
                    },
                },
            ],
            encodings: [{ ssrc: VIDEO_SSRC, rtx: { ssrc: VIDEO_RTX_SSRC } }],
            rtcp: { cname: 'gstreamer' },
        };

        const videoProduceResp = await call(
            Method.Produce,
            new ProduceRequestT('video', JSON.stringify(videoRtpParameters), access),
        );
        videoProducerId = videoProduceResp.id();

        // 2b. Declare the audio producer (Opus 48kHz stereo).
        const audioRtpParameters = {
            codecs: [
                {
                    mimeType: 'audio/opus',
                    payloadType: AUDIO_PAYLOAD_TYPE,
                    clockRate: AUDIO_CLOCK_RATE,
                    channels: AUDIO_CHANNELS,
                },
            ],
            encodings: [{ ssrc: AUDIO_SSRC }],
            rtcp: { cname: 'gstreamer' },
        };

        const audioProduceResp = await call(
            Method.Produce,
            new ProduceRequestT('audio', JSON.stringify(audioRtpParameters), access),
        );
        audioProducerId = audioProduceResp.id();

        // 3. Spin up the native GStreamer pipeline targeting both PlainTransports.
        //    comedia=true means the SFU latches onto whatever source ip:port our
        //    RTP arrives from — no connect step needed for either kind.
        //    The pipeline degrades rather than failing when an audio device
        //    can't be opened, and reports what it settled for.
        const starting = invoke('start_stream', {
            videoHost: videoPt.ip(),
            videoRtpPort: videoPt.port(),
            videoRtcpPort: videoPt.rtcpPort(),
            audioHost: audioPt.ip(),
            audioRtpPort: audioPt.port(),
            audioRtcpPort: audioPt.rtcpPort(),
            windowHandle: windowHandle ?? null,
            processPid: processPid ?? null,
            monitorIndex: monitorIndex ?? null,
            micEnabled: !!micEnabled,
            micDeviceId: micEnabled ? (micDeviceId ?? null) : null,
            micInitiallyMuted: !micEnabled,
        });
        pipelineStarting = starting;
        let status;
        try {
            status = await starting;
        } catch (e) {
            alive();
            throw e;
        } finally {
            if (pipelineStarting === starting) pipelineStarting = null;
        }

        // Torn down while the pipeline was starting: that teardown waited for it
        // and stopped it.
        alive();
        pipelineRunning = true;

        // The viewer subscribes to the video producer id; audio is paired by
        // the SFU under the same session. Surface the video id to the UI.
        if (onProducerCreated) onProducerCreated(videoProduceResp.id(), videoProduceResp.userSession());

        return status;
    }

    async function stopProducing() {
        if (pipelineStarting) {
            const starting = pipelineStarting;
            pipelineStarting = null;
            try {
                await starting;
                pipelineRunning = true;
            } catch { /* never came up: nothing to stop */ }
        }
        if (pipelineRunning) {
            pipelineRunning = false;
            try { await invoke('stop_stream'); } catch (e) { console.error('[MediaSoup] stop_stream failed:', e); }
        }
        // A single ProducerClosed tears down both producers + transports for
        // the session on the SFU.
        if (rpc && (videoProducerId || audioProducerId)) {
            try { rpc.notify(Method.ProducerClosed, new ProducerClosedRequestT()); } catch (e) { console.error('[MediaSoup] ProducerClosed failed:', e); }
        }
        videoProducerId = null;
        audioProducerId = null;
    }

    // Single-flight: concurrent callers (user Stop racing a server end) share one
    // teardown instead of stopping the pipeline twice.
    function destroy() {
        if (!teardown) {
            generation++;
            teardown = (async () => {
                // Whatever stopProducing does, the socket must still be closed, or
                // the next init() overwrites rpc/closeTransport and leaks it.
                try {
                    await stopProducing();
                } finally {
                    try { if (rpc) rpc.clean('destroy'); } catch (e) { console.error('[MediaSoup] rpc.clean failed:', e); }
                    try { if (closeTransport) closeTransport(); } catch (e) { console.error('[MediaSoup] close failed:', e); }
                    rpc = null;
                    closeTransport = null;
                    isConnected.value = false;
                }
            })().finally(() => { teardown = null; });
        }
        return teardown;
    }

    // Re-target the live capture source without touching the RPC/SFU session:
    // GStreamer swaps only its head source element, so the producer keeps running.
    async function switchSource(windowHandle = null, monitorIndex = null) {
        await invoke('switch_source', { windowHandle, monitorIndex });
    }

    return { isConnected, viewerCount, init, on, startSharing, switchSource, destroy };
}
