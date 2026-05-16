use gstreamer as gst;
use gstreamer_rtp as gst_rtp;
use glib;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use glib::ObjectExt;
use glib::Cast;
use gstreamer::prelude::{ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt, GstBinExtManual, GstObjectExt, PadExt, PadExtManual};

// Diagnostic counters incremented by pad probes; the stats thread reads and
// resets them once per second to compute per-stage rates.
struct StreamCounters {
    src_buffers: AtomicU64,    // capture-source src pad — raw frames captured per second
    enc_in_buffers: AtomicU64, // encoder sink pad — frames reaching the encoder per second
    enc_buffers: AtomicU64,    // hardware encoder src — encoded frames out per second
    pay_buffers: AtomicU64,    // rtph264pay src — RTP packets per second
    pay_bytes: AtomicU64,
    udp_buffers: AtomicU64,    // udpsink sink — actual network egress
    udp_bytes: AtomicU64,
}

impl StreamCounters {
    fn new() -> Self {
        Self {
            src_buffers: AtomicU64::new(0),
            enc_in_buffers: AtomicU64::new(0),
            enc_buffers: AtomicU64::new(0),
            pay_buffers: AtomicU64::new(0),
            pay_bytes: AtomicU64::new(0),
            udp_buffers: AtomicU64::new(0),
            udp_bytes: AtomicU64::new(0),
        }
    }
}

// Constants
const LOCAL_VIDEO_RTCP_PORT: i32 = 5003;
const LOCAL_AUDIO_RTCP_PORT: i32 = 5005;
const VIDEO_SSRC: u32 = 2222;
const AUDIO_SSRC: u32 = 1111;

// Global state structure
struct GstreamerState {
    pipeline: Option<gst::Pipeline>,
    main_loop: Option<glib::MainLoop>,
    bus_watch_guard: Option<gst::bus::BusWatchGuard>,
    stats_stop: Option<Arc<AtomicBool>>,
}

impl GstreamerState {
    fn new() -> Self {
        GstreamerState {
            pipeline: None,
            main_loop: None,
            bus_watch_guard: None,
            stats_stop: None,
        }
    }
}

// Shared state
lazy_static::lazy_static! {
    static ref STATE: Arc<Mutex<GstreamerState>> = Arc::new(Mutex::new(GstreamerState::new()));
    static ref GSTREAMER_INITIALIZED: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
}

// Initialize GStreamer (can only be called once per process)
pub fn init() -> Result<(), String> {
    // Check if already initialized
    {
        let initialized = GSTREAMER_INITIALIZED.lock().unwrap();
        if *initialized {
            println!("[GStreamer] Already initialized, skipping init");
            return Ok(());
        }
    }

    // Initialize GStreamer (this can only be called once per process)
    match gst::init() {
        Ok(_) => {
            println!("[GStreamer] Initialized successfully");
            let mut initialized = GSTREAMER_INITIALIZED.lock().unwrap();
            *initialized = true;
            Ok(())
        }
        Err(e) => {
            eprintln!("[GStreamer] Error during init: {:?}", e);
            Err(format!("Failed to initialize GStreamer: {:?}", e))
        }
    }
}

// Apply the NVENC property block. nvh264enc (CUDA mode) and nvd3d11h264enc
// (D3D11 mode) share most semantics but differ in property *names*:
//   rate control:  rc-mode (CUDA) vs rate-control (D3D11)
//   B-frame count: bframes (CUDA) vs b-frames     (D3D11)
//   zero-latency:  zerolatency (CUDA, bool) vs zero-reorder-delay (D3D11, bool)
// Types and enum values match. We probe property availability with
// `find_property` so the same helper works for either encoder.
fn configure_nvenc(enc: &gst::Element) {
    enc.set_property("bitrate", 20000u32); // 20 Mbps target (kbps)
    // 20 Mbps × 0.4 s = 8000 kbits — preserves the 400 ms VBV window the
    // original x264 config used to bound per-frame size variance.
    enc.set_property("vbv-buffer-size", 8000u32);
    enc.set_property_from_str("preset", "low-latency-hp");
    enc.set_property("gop-size", 120i32); // 2 s GoP at 60 fps; i32 on NVENC
    enc.set_property("b-adapt", false);
    enc.set_property("aud", false);

    if enc.find_property("rc-mode").is_some() {
        enc.set_property_from_str("rc-mode", "cbr");
    } else if enc.find_property("rate-control").is_some() {
        enc.set_property_from_str("rate-control", "cbr");
    }

    if enc.find_property("bframes").is_some() {
        enc.set_property("bframes", 0u32);
    } else if enc.find_property("b-frames").is_some() {
        enc.set_property("b-frames", 0u32);
    }

    if enc.find_property("zerolatency").is_some() {
        enc.set_property("zerolatency", true);
    } else if enc.find_property("zero-reorder-delay").is_some() {
        enc.set_property("zero-reorder-delay", true);
    }
}

// Caps probe callback
fn caps_probe_cb(_pad: &gst::Pad, info: &mut gst::PadProbeInfo) -> gst::PadProbeReturn {
    if let Some(event) = info.event() {
        if event.type_() == gst::EventType::Caps {
            let caps_event = match event.view() {
                gst::EventView::Caps(caps_event) => caps_event,
                _ => return gst::PadProbeReturn::Ok,
            };

            let caps = caps_event.caps();
            println!("Caps event at src: {}", caps.to_string());
        }
    }

    gst::PadProbeReturn::Ok
}

// Bus message handler
// Walk up the GstObject parent chain from any descendant to its owning
// pipeline. Used by the bus error handler so it can call `by_name` to find
// the audio sub-graph elements without needing them captured by closure.
fn find_pipeline(start: &gst::Object) -> Option<gst::Pipeline> {
    let mut cur: Option<gst::Object> = Some(start.clone());
    while let Some(o) = cur {
        if let Ok(pipe) = o.clone().downcast::<gst::Pipeline>() {
            return Some(pipe);
        }
        cur = o.parent();
    }
    None
}

// Tear the audio chain out of a running pipeline so a wasapi2src failure
// can't take video down with it. WASAPI failure leaves the source in ERROR
// state, which propagates GST_FLOW_ERROR upstream through rtpbin's session 1
// and blocks data flow on the shared rtpbin — so we unlink the session-1
// pads, set the audio elements to NULL, and remove them entirely.
fn disable_audio_chain(pipeline: &gst::Pipeline) -> bool {
    let names = [
        "wasapi2src", "audioconvert", "audioresample",
        "opus_capsfilter", "opusenc", "rtpopuspay",
        "audio_rtp_sink", "audio_rtcp_sink", "audio_rtcp_src",
    ];
    let elements: Vec<gst::Element> = names.iter()
        .filter_map(|n| pipeline.by_name(n))
        .collect();
    if elements.is_empty() {
        return false;
    }

    // Unlink and release any audio session pads on rtpbin so it isn't left
    // with dangling peers / leaked request pads after we remove the elements.
    // `Pad::unlink` is directional — the receiver must be the SRC pad — so we
    // pick which side of (local, peer) is the source based on pad direction.
    let release_peer_request_pad = |local_pad: &gst::Pad| {
        if let Some(peer) = local_pad.peer() {
            let (src_pad, sink_pad) = if local_pad.direction() == gst::PadDirection::Src {
                (local_pad.clone(), peer.clone())
            } else {
                (peer.clone(), local_pad.clone())
            };
            let _ = src_pad.unlink(&sink_pad);
            if let Some(parent) = peer.parent_element() {
                parent.release_request_pad(&peer);
            }
        }
    };
    if let Some(pay) = pipeline.by_name("rtpopuspay") {
        if let Some(pad) = pay.static_pad("src") {
            release_peer_request_pad(&pad);
        }
    }
    for sink_name in ["audio_rtp_sink", "audio_rtcp_sink"] {
        if let Some(el) = pipeline.by_name(sink_name) {
            if let Some(pad) = el.static_pad("sink") {
                release_peer_request_pad(&pad);
            }
        }
    }
    if let Some(el) = pipeline.by_name("audio_rtcp_src") {
        if let Some(pad) = el.static_pad("src") {
            release_peer_request_pad(&pad);
        }
    }

    for el in &elements {
        let _ = el.set_state(gst::State::Null);
    }
    for el in &elements {
        let _ = pipeline.remove(el);
    }
    true
}

fn bus_call(_bus: &gst::Bus, msg: &gst::Message) -> glib::ControlFlow {
    match msg.view() {
        gst::MessageView::Eos(..) => {
            println!("[GStreamer] End of stream");
            glib::ControlFlow::Break
        }
        gst::MessageView::Error(err) => {
            eprintln!(
                "[GStreamer ERROR] from {:?}: {}\nDebug: {:?}",
                err.src().map(|s| s.path_string()),
                err.error(),
                err.debug()
            );

            // wasapi2src can fail to open at runtime (no playback device,
            // exclusive-mode contention, per-process loopback target with no
            // audio sessions). Isolate the failure to the audio chain so the
            // video stream keeps flowing — returning Break here would kill
            // the bus watch and leave the pipeline with an element stuck in
            // ERROR, blocking data flow upstream through rtpbin.
            let from_audio = err.src()
                .map(|s| s.path_string().to_string())
                .map(|p| p.contains("wasapi2src"))
                .unwrap_or(false);
            if from_audio {
                if let Some(src) = err.src() {
                    if let Some(pipeline) = find_pipeline(&src) {
                        if disable_audio_chain(&pipeline) {
                            eprintln!("[GStreamer] Audio chain disabled after wasapi2src error; video continues");
                            return glib::ControlFlow::Continue;
                        }
                    }
                }
            }

            glib::ControlFlow::Break
        }
        gst::MessageView::Warning(warning) => {
            println!(
                "[GStreamer WARNING] from {:?}: {}\nDebug: {:?}",
                warning.src().map(|s| s.path_string()),
                warning.error(),
                warning.debug()
            );
            glib::ControlFlow::Continue
        }
        gst::MessageView::StateChanged(state_changed) => {
            if state_changed
                .src()
                .map(|s| s == msg.src().unwrap())
                .unwrap_or(false)
            {
                println!(
                    "[GStreamer] Pipeline state changed from {:?} to {:?}",
                    state_changed.old(),
                    state_changed.current()
                );
            }
            glib::ControlFlow::Continue
        }
        gst::MessageView::Latency(_) => {
            println!("[GStreamer] Latency changed, recalculating...");
            glib::ControlFlow::Continue
        }
        gst::MessageView::StreamStatus(_status) => {
            println!("[GStreamer] Stream status message received");
            glib::ControlFlow::Continue
        }
        gst::MessageView::Qos(_) => {
            // QoS messages are posted by an element when it falls behind real
            // time and drops/skips buffers. The source identifies which stage
            // is the bottleneck (encoder / payloader / sink) and the structure
            // contains dropped/processed counts and jitter.
            eprintln!(
                "[QoS] from {:?}: {:?}",
                msg.src().map(|s| s.path_string()),
                msg.structure()
            );
            glib::ControlFlow::Continue
        }
        _ => glib::ControlFlow::Continue,
    }
}

// Create and start the GStreamer pipeline
pub fn create_gstreamer_pipeline(
    video_host: &str,
    video_rtp_port: u16,
    video_rtcp_port: u16,
    audio_host: &str,
    audio_rtp_port: u16,
    audio_rtcp_port: u16,
    window_handle: Option<u64>,
    process_pid: Option<u32>,
) -> Result<(), String> {
    // Initialize GStreamer if not already initialized
    if let Err(e) = init() {
        return Err(e);
    }

    let mut state = STATE.lock().unwrap();

    // Check if pipeline already exists
    if state.pipeline.is_some() {
        eprintln!("[GStreamer] Pipeline already exists, stopping old one");
        drop(state); // Release lock before cleanup
        cleanup();
        state = STATE.lock().unwrap();
    }

    // Create a pipeline
    let pipeline = gst::Pipeline::new();
    let rtpbin = gst::ElementFactory::make("rtpbin")
        .name("rtpbin")
        .build()
        .map_err(|_| "Failed to create rtpbin element".to_string())?;

    // d3d12screencapturesrc has no window-handle property; only d3d11 does.
    // Pick the source based on whether the user asked to capture a window.
    let use_window_capture = window_handle.filter(|&h| h != 0).is_some();

    // Prefer d3d11screencapturesrc for monitor capture too — it keeps frames
    // in D3D11Memory through the zero-copy d3d11convert + nvd3d11h264enc
    // chain below, avoiding the d3d12download + videoconvert + videoscale CPU
    // hop that the d3d12 branch was forced through (nvh264enc can't ingest
    // D3D12Memory directly). Fall back to d3d12 → dx9 → videotestsrc if d3d11
    // isn't available on this build.
    let src = if use_window_capture {
        gst::ElementFactory::make("d3d11screencapturesrc")
            .name("d3d11screencapturesrc")
            .build()
            .map_err(|_| "d3d11screencapturesrc unavailable — required for window capture".to_string())?
    } else {
        match gst::ElementFactory::make("d3d11screencapturesrc")
            .name("d3d11screencapturesrc")
            .build()
        {
            Ok(element) => element,
            Err(_) => {
                println!("d3d11screencapturesrc not available, trying d3d12...");
                match gst::ElementFactory::make("d3d12screencapturesrc")
                    .name("d3d12screencapturesrc")
                    .build()
                {
                    Ok(element) => element,
                    Err(_) => {
                        println!("d3d12screencapturesrc not available, trying alternative sources...");
                        match gst::ElementFactory::make("dx9screencapsrc")
                            .name("dx9screencapsrc")
                            .build()
                        {
                            Ok(element) => element,
                            Err(_) => {
                                println!("Using videotestsrc as fallback");
                                gst::ElementFactory::make("videotestsrc")
                                    .name("videotestsrc")
                                    .build()
                                    .map_err(|_| "Failed to create any video source element".to_string())?
                            }
                        }
                    }
                }
            }
        }
    };

    let videoconvert = gst::ElementFactory::make("videoconvert")
        .name("videoconvert")
        .build()
        .map_err(|_| "Failed to create videoconvert element".to_string())?;

    let videoscale = gst::ElementFactory::make("videoscale")
        .name("videoscale")
        .build()
        .map_err(|_| "Failed to create videoscale element".to_string())?;

    let capsfilter = gst::ElementFactory::make("capsfilter")
        .name("capsfilter")
        .build()
        .map_err(|_| "Failed to create capsfilter element".to_string())?;

    // Encoder choice depends on the upstream memory type. For
    // d3d11screencapturesrc we keep frames in D3D11 memory all the way through
    // a GPU-resident convert/scale (`d3d11convert`) into `nvd3d11h264enc`,
    // which consumes D3D11Memory directly. That avoids the CPU bottleneck of
    // d3d11download + videoconvert + videoscale on 2560x1392 BGRA@60fps which
    // can starve nvh264enc entirely. Other sources stay on `nvh264enc` with
    // the existing system-memory chain.
    let src_factory_name = src.factory().unwrap().name().to_string();
    let videoenc_factory = if src_factory_name == "d3d11screencapturesrc" {
        "nvd3d11h264enc"
    } else {
        "nvh264enc"
    };
    let videoenc = gst::ElementFactory::make(videoenc_factory)
        .name("videoenc")
        .build()
        .map_err(|_| format!("{} unavailable — required for hardware H.264 encode", videoenc_factory))?;

    // Forces the encoder negotiation to High profile — sharper output for
    // 1080p60 game footage. Must match profile-level-id advertised by the
    // frontend's rtpParameters.
    let h264_capsfilter = gst::ElementFactory::make("capsfilter")
        .name("h264_capsfilter")
        .build()
        .map_err(|_| "Failed to create h264 capsfilter element".to_string())?;

    let rtph264pay = gst::ElementFactory::make("rtph264pay")
        .name("rtph264pay")
        .build()
        .map_err(|_| "Failed to create rtph264pay element".to_string())?;

    // Video network elements
    let rtp_sink = gst::ElementFactory::make("udpsink")
        .name("rtp_sink")
        .build()
        .map_err(|_| "Failed to create udpsink element".to_string())?;

    let rtcp_sink = gst::ElementFactory::make("udpsink")
        .name("rtcp_sink")
        .build()
        .map_err(|_| "Failed to create udpsink element".to_string())?;

    let rtcp_src = gst::ElementFactory::make("udpsrc")
        .name("rtcp_src")
        .build()
        .map_err(|_| "Failed to create udpsrc element".to_string())?;

    // Audio capture + encode chain. wasapi2src loopback=true captures the
    // system audio mix (what's playing on the streamer's speakers) — same
    // semantics as a screen capture for the audio side. low-latency=true
    // keeps the WASAPI buffer small so audio doesn't drift behind video.
    let audio_src = gst::ElementFactory::make("wasapi2src")
        .name("wasapi2src")
        .build()
        .map_err(|_| "Failed to create wasapi2src element".to_string())?;

    let audioconvert = gst::ElementFactory::make("audioconvert")
        .name("audioconvert")
        .build()
        .map_err(|_| "Failed to create audioconvert element".to_string())?;

    let audioresample = gst::ElementFactory::make("audioresample")
        .name("audioresample")
        .build()
        .map_err(|_| "Failed to create audioresample element".to_string())?;

    let opus_capsfilter = gst::ElementFactory::make("capsfilter")
        .name("opus_capsfilter")
        .build()
        .map_err(|_| "Failed to create opus capsfilter element".to_string())?;

    let opusenc = gst::ElementFactory::make("opusenc")
        .name("opusenc")
        .build()
        .map_err(|_| "Failed to create opusenc element".to_string())?;

    let rtpopuspay = gst::ElementFactory::make("rtpopuspay")
        .name("rtpopuspay")
        .build()
        .map_err(|_| "Failed to create rtpopuspay element".to_string())?;

    // Audio network elements (separate UDP socket pair from video).
    let audio_rtp_sink = gst::ElementFactory::make("udpsink")
        .name("audio_rtp_sink")
        .build()
        .map_err(|_| "Failed to create audio udpsink element".to_string())?;

    let audio_rtcp_sink = gst::ElementFactory::make("udpsink")
        .name("audio_rtcp_sink")
        .build()
        .map_err(|_| "Failed to create audio rtcp udpsink element".to_string())?;

    let audio_rtcp_src = gst::ElementFactory::make("udpsrc")
        .name("audio_rtcp_src")
        .build()
        .map_err(|_| "Failed to create audio rtcp udpsrc element".to_string())?;

    // 1080p60 NV12 — NVENC's native input format; skips an internal color
    // conversion that I420 would force. d3d12/d3d11 capture sources emit at
    // the native screen resolution; videoscale handles the rescale.
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "NV12")
        .field("width", 1920i32)
        .field("height", 1080i32)
        .field("framerate", gst::Fraction::new(60, 1))
        .build();

    capsfilter.set_property("caps", &caps);

    // High profile @ Level 4.2 — enough headroom for 1080p60. Must match
    // profile-level-id advertised by the frontend's rtpParameters.
    let h264_caps = gst::Caps::builder("video/x-h264")
        .field("profile", "high")
        .build();

    h264_capsfilter.set_property("caps", &h264_caps);

    // Force opusenc into the exact capsline mediasoup expects on the wire:
    // 48 kHz, 2 channels. wasapi2src loopback typically produces F32LE at the
    // device rate (often 48 kHz already), but if the device clock differs
    // audioresample handles the conversion before this filter.
    let opus_in_caps = gst::Caps::builder("audio/x-raw")
        .field("rate", 48000i32)
        .field("channels", 2i32)
        .build();
    opus_capsfilter.set_property("caps", &opus_in_caps);

    // Loopback capture of the default render endpoint (system mix). The
    // existing screen-capture mirror: viewers hear what the streamer hears.
    audio_src.set_property("loopback", true);
    audio_src.set_property("low-latency", true);
    if let Some(pid) = process_pid.filter(|&p| p != 0) {
        // Per-process loopback needs BOTH loopback-mode=include-process-tree
        // AND loopback-target-pid. Default loopback-mode captures the full
        // system mix even when target-pid is set.
        let mode_set = audio_src.find_property("loopback-mode").is_some();
        let pid_set = audio_src.find_property("loopback-target-pid").is_some();
        if mode_set && pid_set {
            audio_src.set_property_from_str("loopback-mode", "include-process-tree");
            audio_src.set_property("loopback-target-pid", pid);
        }
    }

    // 128 kbps stereo Opus tuned for general audio. frame-size=20 ms is the
    // common WebRTC default and matches what browser consumers expect.
    opusenc.set_property("bitrate", 128_000i32);
    opusenc.set_property_from_str("audio-type", "generic");
    opusenc.set_property("inband-fec", true);
    opusenc.set_property("packet-loss-percentage", 5i32);

    rtpopuspay.set_property("ssrc", AUDIO_SSRC);
    rtpopuspay.set_property("pt", 96u32); // Must match audio payloadType in useMediaSoup.js
    rtpopuspay.set_property("mtu", 1400u32);

    // Configure rtpbin for mediasoup compatibility.
    // ntp-time-source defaults to "ntp" (NTP time based on realtime clock),
    // which is what we want for valid RTCP SR timestamps — leave unset.
    // do-retransmission=true makes rtpbin honor incoming NACK feedback by
    // retransmitting lost RTP packets. Confirmed via webrtc-internals that
    // the viewer negotiated RTX (rtxSsrc present) and was sending hundreds
    // of NACKs per session — without RTX enabled, every lost packet that
    // belonged to an IDR took out the whole keyframe and triggered a PLI,
    // producing the visible 3–5 second freeze cadence.
    rtpbin.set_properties(&[
        ("do-retransmission", &true),
        ("rtp-profile", &gst_rtp::RTPProfile::Avpf),
    ]);

    // Provide a pre-configured rtprtxsend so retransmits use the exact PT
    // (101) and SSRC (2223) declared by the frontend's rtpParameters. The
    // default rtpbin-built RTX element picks values mediasoup doesn't
    // recognize, so it discards every retransmit.
    //
    // rtpbin expects the aux sender to be a Bin exposing ghost pads named
    // `sink_<session>` and `src_<session>`. A bare rtprtxsend exposes only
    // `sink` / `src`, so rtpbin can't link `send_rtp_sink_0` to it.
    rtpbin.connect("request-aux-sender", false, |args| {
        // glib-rs marshals "no aux element" as a Value of type GstElement
        // holding NULL — returning bare Rust None panics the closure
        // marshaller because the signal's declared return type is GstElement.
        let no_aux = || Some(glib::value::ToValue::to_value(&None::<gst::Element>));

        let session: u32 = args[1].get().unwrap_or(0);

        // Only the video session (0) carries an RTX sidecar. Audio is Opus
        // with no RTX codec declared on the consumer side.
        if session != 0 {
            return no_aux();
        }

        let rtxsend = match gst::ElementFactory::make("rtprtxsend").build() {
            Ok(el) => el,
            Err(e) => {
                eprintln!("[GStreamer] Failed to build rtprtxsend: {:?}", e);
                return no_aux();
            }
        };

        let pt_map = gst::Structure::builder("application/x-rtp-pt-map")
            .field("100", 101u32)
            .build();
        rtxsend.set_property("payload-type-map", &pt_map);

        let ssrc_map = gst::Structure::builder("application/x-rtp-ssrc-map")
            .field("2222", 2223u32)
            .build();
        rtxsend.set_property("ssrc-map", &ssrc_map);

        // Retain ~1 s of sent packets for retransmission. Default is 100
        // packets which at ~1300 pkt/s holds only ~77 ms of history — too
        // short for any NACK that takes longer than a fast RTT to come back.
        // Late NACKs hit an empty buffer and the loss escalates to PLI.
        rtxsend.set_property("max-size-time", 1000u32);
        rtxsend.set_property("max-size-packets", 0u32); // disable packet-count cap

        let bin = gst::Bin::new();
        if let Err(e) = bin.add(&rtxsend) {
            eprintln!("[GStreamer] Failed to add rtprtxsend to aux bin: {:?}", e);
            return no_aux();
        }

        let sink_target = match rtxsend.static_pad("sink") {
            Some(p) => p,
            None => {
                eprintln!("[GStreamer] rtprtxsend has no sink pad");
                return no_aux();
            }
        };
        let src_target = match rtxsend.static_pad("src") {
            Some(p) => p,
            None => {
                eprintln!("[GStreamer] rtprtxsend has no src pad");
                return no_aux();
            }
        };

        let sink_ghost = gst::GhostPad::builder_with_target(&sink_target)
            .map(|b| b.name(format!("sink_{}", session)).build())
            .ok();
        let src_ghost = gst::GhostPad::builder_with_target(&src_target)
            .map(|b| b.name(format!("src_{}", session)).build())
            .ok();

        let (sink_ghost, src_ghost) = match (sink_ghost, src_ghost) {
            (Some(s), Some(r)) => (s, r),
            _ => {
                eprintln!("[GStreamer] Failed to build ghost pads for aux sender");
                return no_aux();
            }
        };

        if bin.add_pad(&sink_ghost).is_err() || bin.add_pad(&src_ghost).is_err() {
            eprintln!("[GStreamer] Failed to add ghost pads to aux bin");
            return no_aux();
        }

        Some(glib::Value::from(&bin))
    });

    // Configure source based on type. Enum-typed properties on plugin
    // elements (e.g. videotestsrc.pattern) must be set via the nick string —
    // gstreamer-rs can't coerce a raw i32 to a plugin-defined GEnum.
    let src_factory = src.factory().unwrap().name();
    if src_factory == "d3d11screencapturesrc" {
        src.set_property("show-cursor", &true);
        if let Some(hwnd) = window_handle.filter(|&h| h != 0) {
            // window-handle is honored only by the WGC backend; the default
            // DXGI Desktop Duplication path ignores it and produces no frames.
            src.set_property_from_str("capture-api", "wgc");
            src.set_property("window-handle", hwnd);
        } else {
            // WGC backend also handles monitor capture and tracks the actual
            // present rate of the target surface — important for fullscreen
            // games where DXGI Desktop Duplication clamps to a fraction of
            // monitor refresh (observed: 48 fps capture from a 144 Hz / 144 fps
            // game). WGC requires Windows 10 1903+; falls back to DXGI if not
            // available on this build.
            src.set_property_from_str("capture-api", "wgc");
            src.set_property("monitor-index", &0i32);
        }
    } else if src_factory == "d3d12screencapturesrc" || src_factory == "dx9screencapsrc" {
        src.set_property("show-cursor", &true);
        src.set_property("monitor-index", &0i32);
    } else if src_factory == "videotestsrc" {
        src.set_property_from_str("pattern", "smpte");
        src.set_property("is-live", true);
    }

    // iperf3 confirmed the link carries 30 Mbps UDP cleanly with 0% loss to
    // this server — so packet loss in the pipeline is not a bandwidth problem,
    // it's a burstiness problem. NVENC emits an entire frame as a single send;
    // an IDR at 60 fps can dump ~200 KB into the socket in microseconds and
    // overflow whatever queue (router, kernel, mediasoup recv buffer) sits at
    // the bottleneck — even though the average rate fits comfortably.
    //
    // 20 Mbps CBR + a 400 ms VBV window bounds per-frame size variance so
    // IDRs and scene changes can't blow out into a single megaburst.
    configure_nvenc(&videoenc);

    rtph264pay.set_property("ssrc", VIDEO_SSRC);
    rtph264pay.set_property("pt", 100u32); // Must match payloadType declared by frontend
    rtph264pay.set_property("config-interval", -1i32); // SPS/PPS with every IDR for fast UDP recovery
    rtph264pay.set_property("mtu", 1400u32);
    // aggregate-mode=zero-latency switches the payloader from single-NAL
    // (packetization-mode 0) to STAP-A / fragmented (packetization-mode 1).
    // Must agree with packetization-mode=1 declared in useMediaSoup.js
    // — mediasoup matches this even in non-strict mode.
    rtph264pay.set_property_from_str("aggregate-mode", "zero-latency");

    rtp_sink.set_properties(&[
        ("host", &video_host),
        ("port", &(video_rtp_port as i32)),
        ("sync", &false),
        ("async", &false),
        // 4 MB SO_SNDBUF — gives the kernel headroom to absorb whole-frame
        // bursts the encoder hands us in a single write, instead of dropping
        // packets on a full socket queue.
        ("buffer-size", &4_194_304i32),
    ]);

    rtcp_sink.set_properties(&[
        ("host", &video_host),
        ("port", &(video_rtcp_port as i32)),
        ("sync", &false),
        ("async", &false),
        ("buffer-size", &4_194_304i32),
    ]);

    rtcp_src.set_properties(&[
        ("port", &LOCAL_VIDEO_RTCP_PORT),
        ("buffer-size", &1_048_576i32),
    ]);

    // Audio sinks/source — separate UDP sockets, smaller send buffer is fine
    // (Opus at 128 kbps is ~16 KB/s, no burstiness like H.264 IDRs).
    audio_rtp_sink.set_properties(&[
        ("host", &audio_host),
        ("port", &(audio_rtp_port as i32)),
        ("sync", &false),
        ("async", &false),
        ("buffer-size", &262_144i32),
    ]);

    audio_rtcp_sink.set_properties(&[
        ("host", &audio_host),
        ("port", &(audio_rtcp_port as i32)),
        ("sync", &false),
        ("async", &false),
        ("buffer-size", &262_144i32),
    ]);

    audio_rtcp_src.set_properties(&[
        ("port", &LOCAL_AUDIO_RTCP_PORT),
        ("buffer-size", &262_144i32),
    ]);

    // Add elements to pipeline. Audio is always wired up here; if the WASAPI
    // device fails to open at runtime, the bus error handler tears down the
    // audio sub-graph at runtime (see `disable_audio_chain`) so the video
    // chain isn't blocked by an element stuck in ERROR state propagating
    // through the shared rtpbin.
    // Elements that are always in the pipeline regardless of source branch.
    // `videoconvert`/`videoscale`/`capsfilter` are only added for the
    // system-memory branches below — the d3d11 zero-copy path doesn't use
    // them, and including an orphan element in `add_many` will block the
    // pipeline state transition.
    pipeline.add_many(&[
        &rtpbin, &src,
        &videoenc, &h264_capsfilter, &rtph264pay, &rtp_sink,
        &rtcp_sink, &rtcp_src,
        &audio_src, &audioconvert, &audioresample, &opus_capsfilter,
        &opusenc, &rtpopuspay, &audio_rtp_sink, &audio_rtcp_sink, &audio_rtcp_src,
    ]).map_err(|_| "Failed to add elements to pipeline".to_string())?;

    let src_factory = src.factory().unwrap().name();
    if src_factory == "d3d11screencapturesrc" {
        // Zero-copy GPU chain: capture stays in D3D11Memory, d3d11convert
        // handles BGRA→NV12 + rescale to 1920x1080 on the GPU,
        // nvd3d11h264enc encodes directly from D3D11Memory via NVENC. This
        // eliminates the d3d11download + videoconvert + videoscale CPU
        // bottleneck that starves nvh264enc on a 2560x1392@60 BGRA stream.
        let d3d11convert = gst::ElementFactory::make("d3d11convert")
            .name("d3d11convert")
            .build()
            .map_err(|_| "d3d11convert unavailable — required for d3d11 zero-copy path".to_string())?;
        let d3d11_capsfilter = gst::ElementFactory::make("capsfilter")
            .name("d3d11_capsfilter")
            .build()
            .map_err(|_| "Failed to create d3d11 capsfilter element".to_string())?;
        let d3d11_caps = gst::Caps::builder("video/x-raw")
            .features(["memory:D3D11Memory"])
            .field("format", "NV12")
            .field("width", 1920i32)
            .field("height", 1080i32)
            .field("framerate", gst::Fraction::new(60, 1))
            .build();
        d3d11_capsfilter.set_property("caps", &d3d11_caps);
        pipeline.add_many(&[&d3d11convert, &d3d11_capsfilter])
            .map_err(|_| "Failed to add d3d11 zero-copy elements to pipeline".to_string())?;
        gst::Element::link_many(&[&src, &d3d11convert, &d3d11_capsfilter, &videoenc, &h264_capsfilter, &rtph264pay])
            .map_err(|e| format!("Failed to link d3d11 zero-copy chain: {:?}", e))?;
    } else if src_factory == "d3d12screencapturesrc" {
        // d3d12 path: still uses the system-memory chain because nvh264enc
        // doesn't accept D3D12Memory directly. d3d12download lands frames in
        // sysmem; videoconvert + videoscale handle BGRA→NV12 + rescale.
        pipeline.add_many(&[&videoconvert, &videoscale, &capsfilter])
            .map_err(|_| "Failed to add convert/scale/capsfilter to pipeline".to_string())?;
        let d3d12download = gst::ElementFactory::make("d3d12download")
            .build()
            .map_err(|_| "d3d12screencapturesrc requires the d3d12download element, which is unavailable".to_string())?;
        pipeline.add(&d3d12download).map_err(|_| "Failed to add d3d12download to pipeline".to_string())?;
        gst::Element::link_many(&[&src, &d3d12download, &videoconvert, &videoscale, &capsfilter, &videoenc, &h264_capsfilter, &rtph264pay])
            .map_err(|e| format!("Failed to link video elements with d3d12download: {:?}", e))?;
    } else {
        // Fallback (videotestsrc, dx9screencapsrc): system-memory chain.
        pipeline.add_many(&[&videoconvert, &videoscale, &capsfilter])
            .map_err(|_| "Failed to add convert/scale/capsfilter to pipeline".to_string())?;
        gst::Element::link_many(&[&src, &videoconvert, &videoscale, &capsfilter, &videoenc, &h264_capsfilter, &rtph264pay])
            .map_err(|e| format!("Failed to link video elements: {:?}", e))?;
    }

    // Add probe to rtph264pay source pad for debugging
    if let Some(src_pad) = rtph264pay.static_pad("src") {
        src_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, |_pad, info| {
            if let Some(event) = info.event() {
                match event.view() {
                    gst::EventView::Caps(caps_ev) => {
                        println!("RTP Payload caps: {}", caps_ev.caps());
                    }
                    _ => {}
                }
            }
            gst::PadProbeReturn::Ok
        });
    }

    // Link RTP elements on rtpbin session 0 (the trailing _0 is a session id, not an index)
    rtph264pay.link_pads(Some("src"), &rtpbin, Some("send_rtp_sink_0"))
        .map_err(|e| format!("Failed to link rtph264pay to rtpbin: {:?}", e))?;

    rtpbin.link_pads(Some("send_rtp_src_0"), &rtp_sink, Some("sink"))
        .map_err(|e| format!("Failed to link rtpbin to rtp_sink: {:?}", e))?;

    rtpbin.link_pads(Some("send_rtcp_src_0"), &rtcp_sink, Some("sink"))
        .map_err(|e| format!("Failed to link rtpbin to rtcp_sink: {:?}", e))?;

    rtcp_src.link_pads(Some("src"), &rtpbin, Some("recv_rtcp_sink_0"))
        .map_err(|e| format!("Failed to link rtcp_src to rtpbin: {:?}", e))?;

    // Audio chain: capture → convert/resample → 48k/2ch caps → opus encode → RTP payload.
    // Rides session 1 on the same rtpbin instance — keeps per-session SR/RR
    // bookkeeping isolated from video while reusing one main loop / clock.
    // The bus error handler removes this whole sub-graph at runtime if
    // wasapi2src fails to open its endpoint.
    gst::Element::link_many(&[
        &audio_src, &audioconvert, &audioresample, &opus_capsfilter,
        &opusenc, &rtpopuspay,
    ]).map_err(|e| format!("Failed to link audio elements: {:?}", e))?;

    rtpopuspay.link_pads(Some("src"), &rtpbin, Some("send_rtp_sink_1"))
        .map_err(|e| format!("Failed to link rtpopuspay to rtpbin: {:?}", e))?;

    rtpbin.link_pads(Some("send_rtp_src_1"), &audio_rtp_sink, Some("sink"))
        .map_err(|e| format!("Failed to link rtpbin to audio_rtp_sink: {:?}", e))?;

    rtpbin.link_pads(Some("send_rtcp_src_1"), &audio_rtcp_sink, Some("sink"))
        .map_err(|e| format!("Failed to link rtpbin to audio_rtcp_sink: {:?}", e))?;

    audio_rtcp_src.link_pads(Some("src"), &rtpbin, Some("recv_rtcp_sink_1"))
        .map_err(|e| format!("Failed to link audio_rtcp_src to rtpbin: {:?}", e))?;

    // Add caps probe for debugging
    if let Some(src_pad) = src.static_pad("src") {
        src_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, caps_probe_cb);
    }

    // === Diagnostic instrumentation ===
    // Counters incremented from probes; reset every second by the stats thread.
    // The relative rates (encoder fps vs payloader pps vs udp pps) localize a
    // freeze: if all stay nominal but the receiver still freezes, the loss is
    // in transit and will show up as fraction-lost in the rtpsession stats.
    let counters = Arc::new(StreamCounters::new());

    // Count raw frames leaving the capture source — the upstream-most stage.
    if let Some(pad) = src.static_pad("src") {
        let c = counters.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if info.buffer().is_some() {
                c.src_buffers.fetch_add(1, Ordering::Relaxed);
            }
            gst::PadProbeReturn::Ok
        });
    }

    // Count frames arriving at the encoder sink — if this stays at 0 while
    // src is non-zero, the convert/scale chain between them is blocking flow.
    if let Some(pad) = videoenc.static_pad("sink") {
        let c = counters.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if info.buffer().is_some() {
                c.enc_in_buffers.fetch_add(1, Ordering::Relaxed);
            }
            gst::PadProbeReturn::Ok
        });
    }

    if let Some(pad) = videoenc.static_pad("src") {
        let c = counters.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if info.buffer().is_some() {
                c.enc_buffers.fetch_add(1, Ordering::Relaxed);
            }
            gst::PadProbeReturn::Ok
        });
    }

    // rtph264pay (and downstream udpsink) emit RTP packets as buffer-lists
    // — one list per frame containing each MTU-sized fragment. A BUFFER-only
    // probe misses them entirely, undercounting by ~3×, so we accept both.
    if let Some(pad) = rtph264pay.static_pad("src") {
        let c = counters.clone();
        pad.add_probe(
            gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
            move |_pad, info| {
                if let Some(buf) = info.buffer() {
                    c.pay_buffers.fetch_add(1, Ordering::Relaxed);
                    c.pay_bytes.fetch_add(buf.size() as u64, Ordering::Relaxed);
                } else if let Some(list) = info.buffer_list() {
                    let mut n: u64 = 0;
                    let mut b: u64 = 0;
                    list.foreach(|buf, _idx| {
                        n += 1;
                        b += buf.size() as u64;
                        std::ops::ControlFlow::Continue(())
                    });
                    c.pay_buffers.fetch_add(n, Ordering::Relaxed);
                    c.pay_bytes.fetch_add(b, Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            },
        );
    }

    if let Some(pad) = rtp_sink.static_pad("sink") {
        let c = counters.clone();
        pad.add_probe(
            gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
            move |_pad, info| {
                if let Some(buf) = info.buffer() {
                    c.udp_buffers.fetch_add(1, Ordering::Relaxed);
                    c.udp_bytes.fetch_add(buf.size() as u64, Ordering::Relaxed);
                } else if let Some(list) = info.buffer_list() {
                    let mut n: u64 = 0;
                    let mut b: u64 = 0;
                    list.foreach(|buf, _idx| {
                        n += 1;
                        b += buf.size() as u64;
                        std::ops::ControlFlow::Continue(())
                    });
                    c.udp_buffers.fetch_add(n, Ordering::Relaxed);
                    c.udp_bytes.fetch_add(b, Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            },
        );
    }

    // (RTCP-from-peer arrival is observable via the rtpsession stats: the
    // `sent-rb-*` fields update only when an RR is received, so a frozen
    // round-trip / fractionlost across multiple polls means feedback is silent.)

    // Set up bus watching
    let bus = pipeline.bus().unwrap();
    let bus_watch_guard = bus.add_watch(bus_call)
        .map_err(|_| "Failed to add bus watch".to_string())?;

    // Share a single GSocket between rtcp_src and rtcp_sink so RTCP egresses
    // from the same port we listen on. Required for mediasoup PlainTransport
    // (comedia=true): the SFU echoes RTCP back to whatever ip:port our RTCP
    // arrived from. Without socket sharing, that source port is ephemeral and
    // we never receive RR/PLI/REMB feedback.
    rtcp_src.set_state(gst::State::Ready)
        .map_err(|e| format!("Failed to set rtcp_src to Ready: {:?}", e))?;
    let rtcp_socket_value = rtcp_src.property_value("used-socket");
    rtcp_sink.set_property_from_value("socket", &rtcp_socket_value);
    rtcp_sink.set_property("close-socket", false);

    // Same comedia socket-sharing for the audio RTCP pair.
    audio_rtcp_src.set_state(gst::State::Ready)
        .map_err(|e| format!("Failed to set audio_rtcp_src to Ready: {:?}", e))?;
    let audio_rtcp_socket_value = audio_rtcp_src.property_value("used-socket");
    audio_rtcp_sink.set_property_from_value("socket", &audio_rtcp_socket_value);
    audio_rtcp_sink.set_property("close-socket", false);

    // Start playing
    let state_change_result = pipeline.set_state(gst::State::Playing);
    match state_change_result {
        Ok(_) => println!("Pipeline state change initiated successfully"),
        Err(e) => return Err(format!("Failed to start pipeline: {:?}", e)),
    }

    // Wait for state change to complete with timeout
    let (state_result, current_state, pending_state) = pipeline.state(gst::ClockTime::from_seconds(5));
    match state_result {
        Ok(_) => {
            println!("Pipeline current state: {:?}, pending: {:?}", current_state, pending_state);
        }
        Err(e) => {
            return Err(format!("Failed to get pipeline state: {:?}", e));
        }
    }

    println!("GStreamer pipeline started successfully");

    // Create and start a main loop to process bus messages
    let main_loop = glib::MainLoop::new(None, false);
    let main_loop_clone = main_loop.clone();

    // Spawn a thread to run the main loop
    std::thread::spawn(move || {
        main_loop_clone.run();
    });

    // Stats logger — once per second, prints per-stage rates (so you can see
    // exactly which stage stalls during a freeze) and the rtpsession stats
    // structure, which carries the receiver-reported fraction-lost / jitter /
    // round-trip extracted from incoming RTCP RR packets.
    let stats_stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stats_stop.clone();
    let counters_clone = counters.clone();
    let rtpbin_clone = rtpbin.clone();
    std::thread::spawn(move || {
        let mut prev = Instant::now();
        while !stop_clone.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            if stop_clone.load(Ordering::Relaxed) {
                break;
            }
            let now = Instant::now();
            let dt = now.duration_since(prev).as_secs_f64().max(0.001);
            prev = now;

            let src_c = counters_clone.src_buffers.swap(0, Ordering::Relaxed);
            let enc_in = counters_clone.enc_in_buffers.swap(0, Ordering::Relaxed);
            let enc = counters_clone.enc_buffers.swap(0, Ordering::Relaxed);
            let pay = counters_clone.pay_buffers.swap(0, Ordering::Relaxed);
            let pay_b = counters_clone.pay_bytes.swap(0, Ordering::Relaxed);
            let udp = counters_clone.udp_buffers.swap(0, Ordering::Relaxed);
            let udp_b = counters_clone.udp_bytes.swap(0, Ordering::Relaxed);

            eprintln!(
                "[rate dt={:.2}s] src={:.1}fps  enc_in={:.1}fps  enc={:.1}fps  pay={:.0}pkt/s ({:.2}Mbps)  udp={:.0}pkt/s ({:.2}Mbps)",
                dt,
                src_c as f64 / dt,
                enc_in as f64 / dt,
                enc as f64 / dt,
                pay as f64 / dt,
                (pay_b as f64 * 8.0 / 1_000_000.0) / dt,
                udp as f64 / dt,
                (udp_b as f64 * 8.0 / 1_000_000.0) / dt,
            );

            // get-internal-session is an action signal — thread-safe to invoke
            // off the streaming thread; the returned RTPSession's "stats"
            // property snapshot is updated as RTCP arrives. The signal's
            // declared return type is the concrete `GstRTPSession`; using
            // `emit_by_name::<glib::Object>` panics in glib-rs 0.18 with
            // "expected GObject, got RTPSession" because the closure return
            // typecheck requires an exact static-type match. The
            // `emit_by_name_with_values` path returns a raw Value and then
            // `Value::get::<glib::Object>` does the subclass upcast for us.
            let log_session = |label: &str, session_id: u32| {
                use glib::value::ToValue;
                let value_opt = rtpbin_clone.emit_by_name_with_values(
                    "get-internal-session",
                    &[session_id.to_value()],
                );
                let session = match value_opt.and_then(|v| v.get::<glib::Object>().ok()) {
                    Some(s) => s,
                    None => return, // session not present (e.g. audio torn down)
                };
                let stats: gst::Structure = session.property("stats");
                eprintln!("[{}] {}", label, stats);
            };
            log_session("rtpsession-0/video", 0);
        }
        eprintln!("[stats] logger thread exiting");
    });

    // Store state
    state.pipeline = Some(pipeline);
    state.bus_watch_guard = Some(bus_watch_guard);
    state.main_loop = Some(main_loop);
    state.stats_stop = Some(stats_stop);

    Ok(())
}

// Cleanup function
pub fn cleanup() {
    let mut state = STATE.lock().unwrap();

    if let Some(stop) = state.stats_stop.take() {
        stop.store(true, Ordering::Relaxed);
    }

    if let Some(pipeline) = state.pipeline.take() {
        if let Some(bus_watch_guard) = state.bus_watch_guard.take() {
            // BusWatchGuard automatically removes itself when dropped
            drop(bus_watch_guard);
        }

        let _ = pipeline.set_state(gst::State::Null);
        // Wait for state change
        let _ = pipeline.state(gst::ClockTime::from_seconds(2));
    }

    if let Some(main_loop) = state.main_loop.take() {
        if main_loop.is_running() {
            main_loop.quit();
        }
    }
}

// Start streaming function to be called from lib.rs
pub fn start_streaming(
    video_host: String,
    video_rtp_port: u16,
    video_rtcp_port: u16,
    audio_host: String,
    audio_rtp_port: u16,
    audio_rtcp_port: u16,
    window_handle: Option<u64>,
    process_pid: Option<u32>,
) -> Result<String, String> {
    init()?;

    println!(
        "video {}:{}/{}  audio {}:{}/{}",
        video_host, video_rtp_port, video_rtcp_port,
        audio_host, audio_rtp_port, audio_rtcp_port,
    );

    create_gstreamer_pipeline(
        &video_host, video_rtp_port, video_rtcp_port,
        &audio_host, audio_rtp_port, audio_rtcp_port,
        window_handle, process_pid,
    )?;

    Ok("Streaming started successfully".to_string())
}

// Stop streaming function
pub fn stop_streaming() -> Result<(), String> {
    cleanup();
    Ok(())
}