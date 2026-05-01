use gstreamer as gst;
use gstreamer_rtp as gst_rtp;
use glib;
use std::sync::{Arc, Mutex};
use std::env;
use glib::ObjectExt;
use gstreamer::prelude::{ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt, GstBinExtManual, GstObjectExt, PadExtManual};

// Constants
const LOCAL_RTCP_PORT: i32 = 5003;
const VIDEO_SSRC: u32 = 2222;

// Global state structure
struct GstreamerState {
    pipeline: Option<gst::Pipeline>,
    main_loop: Option<glib::MainLoop>,
    bus_watch_guard: Option<gst::bus::BusWatchGuard>,
}

impl GstreamerState {
    fn new() -> Self {
        GstreamerState {
            pipeline: None,
            main_loop: None,
            bus_watch_guard: None,
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
    rtpbin.set_properties(&[
        ("do-retransmission", &false),
        ("rtp-profile", &gst_rtp::RTPProfile::Avpf),
    ]);

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

    // Tuned for 1080p60 game streaming: high bitrate, "faster" preset for
    // motion sharpness, zerolatency to keep glass-to-glass low, no B-frames.
    // psy-tune=film preserves perceptual detail in fast scenes.
    //
    // Bitrate target: matching Discord's 12 Mbps@720p60 visual quality at
    // 1080p60 needs ~27 Mbps for the 2.25× pixel count, plus ~25% headroom
    // for zerolatency's efficiency penalty. 35 Mbps comfortably exceeds it
    // for sharp motion in dynamic scenes. Stays within H.264 Level 4.2's
    // 50 Mbps ceiling.
    x264enc.set_property("bitrate", 35000u32); // 35 Mbps
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

    // Store state
    state.pipeline = Some(pipeline);
    state.bus_watch_guard = Some(bus_watch_guard);
    state.main_loop = Some(main_loop);

    Ok(())
}

// Cleanup function
pub fn cleanup() {
    let mut state = STATE.lock().unwrap();

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