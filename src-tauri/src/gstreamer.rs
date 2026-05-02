use gstreamer as gst;
use gstreamer_rtp as gst_rtp;
use glib;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::env;
use glib::ObjectExt;
use gstreamer::prelude::{ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt, GstBinExtManual, GstObjectExt, PadExtManual};

// Diagnostic counters incremented by pad probes; the stats thread reads and
// resets them once per second to compute per-stage rates.
struct StreamCounters {
    enc_buffers: AtomicU64,   // x264enc src — encoded frames out per second
    pay_buffers: AtomicU64,   // rtph264pay src — RTP packets per second
    pay_bytes: AtomicU64,
    udp_buffers: AtomicU64,   // udpsink sink — actual network egress
    udp_bytes: AtomicU64,
}

impl StreamCounters {
    fn new() -> Self {
        Self {
            enc_buffers: AtomicU64::new(0),
            pay_buffers: AtomicU64::new(0),
            pay_bytes: AtomicU64::new(0),
            udp_buffers: AtomicU64::new(0),
            udp_bytes: AtomicU64::new(0),
        }
    }
}

// Constants
const LOCAL_RTCP_PORT: i32 = 5003;
const VIDEO_SSRC: u32 = 2222;

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

// Set up GStreamer environment
fn setup_gstreamer_environment() -> Result<(), String> {
    // Get the executable directory
    let exe_dir = match env::current_exe() {
        Ok(exe_path) => match exe_path.parent() {
            Some(dir) => dir.to_path_buf(),
            None => return Err("Failed to get executable directory".to_string()),
        },
        Err(e) => return Err(format!("Failed to get executable path: {}", e)),
    };

    // Set up GStreamer plugin path
    let gst_plugin_path = exe_dir.join("gstreamer").join("lib").join("gstreamer-1.0");
    println!("Setting GST_PLUGIN_PATH to: {}", gst_plugin_path.display());
    env::set_var("GST_PLUGIN_PATH", gst_plugin_path.to_string_lossy().to_string());

    // Add GStreamer bin directory to PATH
    let gst_bin_path = exe_dir.join("gstreamer").join("bin");
    println!("Adding to PATH: {}", gst_bin_path.display());

    // Get current PATH
    let path_var = match env::var("PATH") {
        Ok(path) => path,
        Err(e) => return Err(format!("Failed to get PATH environment variable: {}", e)),
    };

    // Prepend GStreamer bin path to PATH
    let new_path = format!("{};{}", gst_bin_path.to_string_lossy(), path_var);
    env::set_var("PATH", new_path);

    Ok(())
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

    // Set up GStreamer environment
    setup_gstreamer_environment()?;

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
pub fn create_gstreamer_pipeline(host: &str, rtp_port: u16, rtcp_port: u16) -> Result<(), String> {
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

    // Create video elements - use fallback source if d3d12screencapturesrc fails
    let src = match gst::ElementFactory::make("d3d12screencapturesrc")
        .name("d3d12screencapturesrc")
        .build()
    {
        Ok(element) => element,
        Err(_) => {
            println!("d3d12screencapturesrc not available, trying alternative sources...");
            // Try dx9screencapsrc as fallback
            match gst::ElementFactory::make("dx9screencapsrc")
                .name("dx9screencapsrc")
                .build()
            {
                Ok(element) => element,
                Err(_) => {
                    // Last resort: use videotestsrc for testing
                    println!("Using videotestsrc as fallback");
                    gst::ElementFactory::make("videotestsrc")
                        .name("videotestsrc")
                        .build()
                        .map_err(|_| "Failed to create any video source element".to_string())?
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

    let x264enc = gst::ElementFactory::make("x264enc")
        .name("x264enc")
        .build()
        .map_err(|_| "Failed to create x264enc element".to_string())?;

    // Forces the encoder negotiation to High profile so dct8x8 / CABAC are
    // available — meaningfully sharper output for 1080p60 game footage.
    let h264_capsfilter = gst::ElementFactory::make("capsfilter")
        .name("h264_capsfilter")
        .build()
        .map_err(|_| "Failed to create h264 capsfilter element".to_string())?;

    let rtph264pay = gst::ElementFactory::make("rtph264pay")
        .name("rtph264pay")
        .build()
        .map_err(|_| "Failed to create rtph264pay element".to_string())?;

    // Create network elements
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

    // 1080p60 output. d3d12screencapturesrc captures at native screen
    // resolution; videoscale handles down/up-scaling to this target.
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "I420")
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
        let session: u32 = args[1].get().unwrap_or(0);

        let rtxsend = match gst::ElementFactory::make("rtprtxsend").build() {
            Ok(el) => el,
            Err(e) => {
                eprintln!("[GStreamer] Failed to build rtprtxsend: {:?}", e);
                return None;
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
            return None;
        }

        let sink_target = match rtxsend.static_pad("sink") {
            Some(p) => p,
            None => {
                eprintln!("[GStreamer] rtprtxsend has no sink pad");
                return None;
            }
        };
        let src_target = match rtxsend.static_pad("src") {
            Some(p) => p,
            None => {
                eprintln!("[GStreamer] rtprtxsend has no src pad");
                return None;
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
                return None;
            }
        };

        if bin.add_pad(&sink_ghost).is_err() || bin.add_pad(&src_ghost).is_err() {
            eprintln!("[GStreamer] Failed to add ghost pads to aux bin");
            return None;
        }

        Some(glib::Value::from(&bin))
    });

    // Configure source based on type. Enum-typed properties on plugin
    // elements (e.g. videotestsrc.pattern) must be set via the nick string —
    // gstreamer-rs can't coerce a raw i32 to a plugin-defined GEnum.
    if src.factory().unwrap().name() == "d3d12screencapturesrc" || src.factory().unwrap().name() == "dx9screencapsrc" {
        src.set_properties(&[
            ("show-cursor", &true),
            ("monitor-index", &0i32),
        ]);
    } else if src.factory().unwrap().name() == "videotestsrc" {
        src.set_property_from_str("pattern", "smpte");
        src.set_property("is-live", true);
    }

    // iperf3 confirmed the link carries 30 Mbps UDP cleanly with 0% loss to
    // this server — so packet loss in the pipeline is not a bandwidth problem,
    // it's a burstiness problem. x264 emits an entire frame as a single send;
    // an IDR at 60 fps can dump ~200 KB into the socket in microseconds and
    // overflow whatever queue (router, kernel, mediasoup recv buffer) sits at
    // the bottleneck — even though the average rate fits comfortably.
    //
    // 10 Mbps target leaves 3× headroom under the link's clean ceiling.
    // vbv-buf-capacity=400 ms (down from default 600) tightens the rate-control
    // window so per-frame size variance is bounded — IDRs and scene changes
    // can't blow out into a single megaburst.
    x264enc.set_property("bitrate", 12000u32); // 12 Mbps
    x264enc.set_property("vbv-buf-capacity", 400u32);
    x264enc.set_property_from_str("speed-preset", "faster");
    x264enc.set_property_from_str("psy-tune", "film");
    x264enc.set_property_from_str("tune", "zerolatency");
    x264enc.set_property("key-int-max", 120u32); // 2 s GoP at 60 fps
    x264enc.set_property("b-adapt", false);
    x264enc.set_property("bframes", 0u32);
    x264enc.set_property("aud", false);
    x264enc.set_property("dct8x8", true); // requires High profile (enforced by h264_capsfilter)

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
        ("host", &host),
        ("port", &(rtp_port as i32)),
        ("sync", &false),
        ("async", &false),
        // 4 MB SO_SNDBUF — gives the kernel headroom to absorb whole-frame
        // bursts the encoder hands us in a single write, instead of dropping
        // packets on a full socket queue.
        ("buffer-size", &4_194_304i32),
    ]);

    rtcp_sink.set_properties(&[
        ("host", &host),
        ("port", &(rtcp_port as i32)),
        ("sync", &false),
        ("async", &false),
        ("buffer-size", &4_194_304i32),
    ]);

    rtcp_src.set_properties(&[
        ("port", &LOCAL_RTCP_PORT),
        ("buffer-size", &1_048_576i32),
    ]);

    // Add elements to pipeline
    pipeline.add_many(&[
        &rtpbin, &src, &videoconvert, &videoscale,
        &x264enc, &h264_capsfilter, &rtph264pay, &rtp_sink,
        &rtcp_sink, &rtcp_src, &capsfilter,
    ]).map_err(|_| "Failed to add elements to pipeline".to_string())?;

    // d3d12screencapturesrc emits memory:D3D12Memory caps, which videoconvert can't accept.
    // Require d3d12download in that path; other sources link directly to videoconvert.
    if src.factory().unwrap().name() == "d3d12screencapturesrc" {
        let d3d12download = gst::ElementFactory::make("d3d12download")
            .build()
            .map_err(|_| "d3d12screencapturesrc requires the d3d12download element, which is unavailable".to_string())?;
        pipeline.add(&d3d12download).map_err(|_| "Failed to add d3d12download to pipeline".to_string())?;
        gst::Element::link_many(&[&src, &d3d12download, &videoconvert, &videoscale, &capsfilter, &x264enc, &h264_capsfilter, &rtph264pay])
            .map_err(|e| format!("Failed to link video elements with d3d12download: {:?}", e))?;
    } else {
        gst::Element::link_many(&[&src, &videoconvert, &videoscale, &capsfilter, &x264enc, &h264_capsfilter, &rtph264pay])
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

    if let Some(pad) = x264enc.static_pad("src") {
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

            let enc = counters_clone.enc_buffers.swap(0, Ordering::Relaxed);
            let pay = counters_clone.pay_buffers.swap(0, Ordering::Relaxed);
            let pay_b = counters_clone.pay_bytes.swap(0, Ordering::Relaxed);
            let udp = counters_clone.udp_buffers.swap(0, Ordering::Relaxed);
            let udp_b = counters_clone.udp_bytes.swap(0, Ordering::Relaxed);

            eprintln!(
                "[rate dt={:.2}s] enc={:.1}fps  pay={:.0}pkt/s ({:.2}Mbps)  udp={:.0}pkt/s ({:.2}Mbps)",
                dt,
                enc as f64 / dt,
                pay as f64 / dt,
                (pay_b as f64 * 8.0 / 1_000_000.0) / dt,
                udp as f64 / dt,
                (udp_b as f64 * 8.0 / 1_000_000.0) / dt,
            );

            // get-internal-session is an action signal — thread-safe to invoke
            // off the streaming thread; the returned RTPSession's "stats"
            // property snapshot is updated as RTCP arrives.
            let session_obj = rtpbin_clone.emit_by_name::<glib::Object>(
                "get-internal-session",
                &[&0u32],
            );
            let stats: gst::Structure = session_obj.property("stats");
            eprintln!("[rtpsession-0] {}", stats.to_string());
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
pub fn start_streaming(host: String, rtp_port: u16, rtcp_port: u16) -> Result<String, String> {
    // Initialize GStreamer
    init()?;

    println!("host: {}, port: {}, rtcp_port: {}", host, rtp_port, rtcp_port);

    // Create and start the pipeline
    create_gstreamer_pipeline(&host, rtp_port, rtcp_port)?;

    Ok("Streaming started successfully".to_string())
}

// Stop streaming function
pub fn stop_streaming() -> Result<(), String> {
    cleanup();
    Ok(())
}