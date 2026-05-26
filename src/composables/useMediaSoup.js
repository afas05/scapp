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

    let onStreamEnded = null;
    let onProducerCreated = null;
    let onProducerClosed = null;
    let onViewerCount = null;

    function on(event, callback) {
        switch (event) {
            case 'streamEnded': onStreamEnded = callback; break;
            case 'producerCreated': onProducerCreated = callback; break;
            case 'producerClosed': onProducerClosed = callback; break;
            case 'viewerCount': onViewerCount = callback; break;
        }
    }

    // sfuWsUrl  = 'wss://host:port'  (no trailing slash, no ?auth=)
    async function init(token, { sfuWsUrl }) {
        const transport = await createWebSocketTransport(sfuWsUrl + '?auth=', token);
        rpc = transport.rpc;
        closeTransport = transport.close;
        isConnected.value = true;

        rpc.registerHandler(Method.ViewerCount, (req) => {
            viewerCount.value = req.count();
            if (onViewerCount) onViewerCount(req.count());
        });

        rpc.registerHandler(Method.StreamEnded, () => {
            if (onStreamEnded) onStreamEnded();
        });
    }

    async function startSharing(windowHandle = null, processPid = null, micEnabled = false, micDeviceId = null, monitorIndex = null) {
        // 1a. Video PlainTransport (comedia=true; we just send RTP at it).
        const videoPt = await rpc.request(
            Method.CreateProducerPlainTransport,
            new CreateProducerPlainTransportRequestT('video'),
        );

        // 1b. Audio PlainTransport on the same WS session — separate UDP port pair.
        const audioPt = await rpc.request(
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

        const videoProduceResp = await rpc.request(
            Method.Produce,
            new ProduceRequestT('video', JSON.stringify(videoRtpParameters)),
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

        const audioProduceResp = await rpc.request(
            Method.Produce,
            new ProduceRequestT('audio', JSON.stringify(audioRtpParameters)),
        );
        audioProducerId = audioProduceResp.id();

        // 3. Spin up the native GStreamer pipeline targeting both PlainTransports.
        //    comedia=true means the SFU latches onto whatever source ip:port our
        //    RTP arrives from — no connect step needed for either kind.
        await invoke('start_stream', {
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

        pipelineRunning = true;

        // The viewer subscribes to the video producer id; audio is paired by
        // the SFU under the same session. Surface the video id to the UI.
        if (onProducerCreated) onProducerCreated(videoProduceResp.id(), videoProduceResp.userSession());
    }

    async function stopProducing() {
        if (pipelineRunning) {
            try { await invoke('stop_stream'); } catch (e) { console.error('[MediaSoup] stop_stream failed:', e); }
            pipelineRunning = false;
        }
        // A single ProducerClosed tears down both producers + transports for
        // the session on the SFU.
        if (rpc && (videoProducerId || audioProducerId)) {
            rpc.notify(Method.ProducerClosed, new ProducerClosedRequestT());
        }
        videoProducerId = null;
        audioProducerId = null;
        if (onProducerClosed) onProducerClosed();
    }

    async function destroy() {
        await stopProducing();
        if (rpc) rpc.clean('destroy');
        if (closeTransport) closeTransport();
        rpc = null;
        closeTransport = null;
        isConnected.value = false;
    }

    return { isConnected, viewerCount, init, on, startSharing, stopProducing, destroy };
}
