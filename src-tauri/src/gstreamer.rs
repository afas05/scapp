use gstreamer as gst;
use gstreamer_rtp as gst_rtp;
use glib;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use glib::ObjectExt;
use glib::Cast;
use gstreamer::prelude::{DeviceExt, DeviceMonitorExt, DeviceMonitorExtManual, ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt, GstBinExtManual, GstObjectExt, PadExt, PadExtManual};
use base64::{engine::general_purpose::STANDARD, Engine};

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

#[derive(serde::Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MicDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

// Snapshot the GstDevice list and extract a stable wasapi endpoint id for
// each — used both for the frontend listing (`MicDevice.id`) and to look the
// device back up at start_streaming time so wasapi2src is built via
// `Device::create_element`, which configures every internal endpoint property
// the plugin needs (not just the `device` GObject property).
#[cfg(target_os = "linux")]
fn enumerate_audio_input_devices() -> Vec<(gst::Device, String, String, bool)> {
    let monitor = gst::DeviceMonitor::new();
    let caps = gst::Caps::new_empty_simple("audio/x-raw");
    let _ = monitor.add_filter(Some("Audio/Source"), Some(&caps));
    if monitor.start().is_err() {
        return Vec::new();
    }

    let mut out: Vec<(gst::Device, String, String, bool)> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for device in monitor.devices() {
        let name = device.display_name().to_string();
        let props = device.properties();

        // Accept pulseaudio and pipewire sources only.
        let api_ok = props
            .as_ref()
            .and_then(|p| p.get::<String>("device.api").ok())
            .map(|v| v == "pulseaudio" || v == "pipewire")
            .unwrap_or(false);
        if !api_ok {
            continue;
        }

        // Skip monitor sources (system audio loopback) — not mic inputs.
        let is_monitor = props.as_ref()
            .and_then(|p| p.get::<String>("device.class").ok())
            .map(|c| c == "monitor")
            .unwrap_or(false);
        if is_monitor {
            continue;
        }

        let id = props.as_ref()
            .and_then(|p| {
                p.get::<String>("node.name")
                    .or_else(|_| p.get::<String>("device.name"))
                    .or_else(|_| p.get::<String>("object.id"))
                    .ok()
            })
            .unwrap_or_else(|| name.clone());

        if id.is_empty() || !seen_ids.insert(id.clone()) {
            continue;
        }

        let is_default = props.as_ref()
            .and_then(|p| p.get::<bool>("is-default").ok())
            .unwrap_or(false);

        out.push((device, id, name, is_default));
    }

    monitor.stop();

    if !out.is_empty() && !out.iter().any(|(_, _, _, def)| *def) {
        out[0].3 = true;
    }

    out
}

#[cfg(windows)]
fn enumerate_audio_input_devices() -> Vec<(gst::Device, String, String, bool)> {
    let monitor = gst::DeviceMonitor::new();
    let caps = gst::Caps::new_empty_simple("audio/x-raw");
    let _ = monitor.add_filter(Some("Audio/Source"), Some(&caps));
    if monitor.start().is_err() {
        return Vec::new();
    }

    let mut out: Vec<(gst::Device, String, String, bool)> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for device in monitor.devices() {
        let name = device.display_name().to_string();
        let props = device.properties();

        let api_ok = props
            .as_ref()
            .and_then(|p| p.get::<String>("device.api").ok())
            .as_deref()
            .map(|v| v == "wasapi2")
            .unwrap_or(false);
        if !api_ok {
            continue;
        }

        let temp_el = device.create_element(None).ok();
        let is_loopback = temp_el
            .as_ref()
            .filter(|el| el.find_property("loopback").is_some())
            .map(|el| el.property::<bool>("loopback"))
            .unwrap_or(false);
        if is_loopback {
            continue;
        }

        let id: Option<String> = props
            .as_ref()
            .and_then(|p| {
                p.get::<String>("device.strid")
                    .or_else(|_| p.get::<String>("device.id"))
                    .or_else(|_| p.get::<String>("wasapi2.device.id"))
                    .ok()
            })
            .or_else(|| {
                temp_el.as_ref().and_then(|el| {
                    if el.find_property("device").is_some() {
                        el.property::<Option<String>>("device")
                    } else {
                        None
                    }
                })
            });

        let id = match id {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        if !seen_ids.insert(id.clone()) {
            continue;
        }

        let is_default = props
            .as_ref()
            .and_then(|p| p.get::<bool>("is-default").ok())
            .unwrap_or(false);

        out.push((device, id, name, is_default));
    }

    monitor.stop();

    if !out.is_empty() && !out.iter().any(|(_, _, _, def)| *def) {
        out[0].3 = true;
    }

    out
}

pub fn list_audio_input_devices() -> Result<Vec<MicDevice>, String> {
    init()?;
    Ok(enumerate_audio_input_devices()
        .into_iter()
        .map(|(_, id, name, is_default)| MicDevice { id, name, is_default })
        .collect())
}

// Toggle the mic branch's `valve.drop` property on the running pipeline.
// Returns Ok(()) with no effect if there's no pipeline or no mic branch.
pub fn set_mic_muted(muted: bool) -> Result<(), String> {
    let state = STATE.lock().unwrap();
    let pipeline = match &state.pipeline {
        Some(p) => p.clone(),
        None => return Ok(()),
    };
    if let Some(valve) = pipeline.by_name("mic_valve") {
        valve.set_property("drop", muted);
    }
    Ok(())
}

// Global state structure
struct GstreamerState {
    pipeline: Option<gst::Pipeline>,
    main_loop: Option<glib::MainLoop>,
    bus_watch_guard: Option<gst::bus::BusWatchGuard>,
    stats_stop: Option<Arc<AtomicBool>>,
    recording: bool,
}

impl GstreamerState {
    fn new() -> Self {
        GstreamerState {
            pipeline: None,
            main_loop: None,
            bus_watch_guard: None,
            stats_stop: None,
            recording: false,
        }
    }
}

// Shared state
lazy_static::lazy_static! {
    static ref STATE: Arc<Mutex<GstreamerState>> = Arc::new(Mutex::new(GstreamerState::new()));
    static ref GSTREAMER_INITIALIZED: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    static ref PREVIEW_FRAME: Mutex<String> = Mutex::new(String::new());
}

pub fn get_preview_frame() -> Result<String, String> {
    Ok(PREVIEW_FRAME.lock().unwrap().clone())
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

fn configure_encoder(enc: &gst::Element) {
    let factory = enc.factory().map(|f| f.name().to_string()).unwrap_or_default();
    if factory.contains("nvh264enc") || factory.contains("nvd3d11h264enc") {
        configure_nvenc(enc);
    } else if factory.contains("vaapi") || factory.contains("va") {
        if enc.find_property("rate-control").is_some() {
            enc.set_property_from_str("rate-control", "cbr");
        }
        if enc.find_property("bitrate").is_some() {
            enc.set_property("bitrate", 20000u32);
        }
        if enc.find_property("keyframe-period").is_some() {
            enc.set_property("keyframe-period", 120u32);
        }
        if enc.find_property("cabac").is_some() {
            enc.set_property("cabac", true);
        }
    } else if factory == "x264enc" {
        enc.set_property("bitrate", 20000u32);
        enc.set_property_from_str("tune", "zerolatency");
        enc.set_property_from_str("speed-preset", "superfast");
        enc.set_property("key-int-max", 120u32);
        enc.set_property("bframes", 0u32);
    }
}

// --- Platform-specific video source creation ---

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
        || std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn create_video_source(window_handle: Option<u64>) -> Result<gst::Element, String> {
    let use_window_capture = window_handle.filter(|&h| h != 0).is_some();

    if use_window_capture {
        if let Ok(el) = gst::ElementFactory::make("ximagesrc").name("screensrc").build() {
            return Ok(el);
        }
        return Err("ximagesrc unavailable — required for X11 window capture".to_string());
    }

    if is_wayland() {
        println!("[GStreamer] Wayland session detected, using pipewiresrc");
        if let Ok(el) = gst::ElementFactory::make("pipewiresrc").name("screensrc").build() {
            return Ok(el);
        }
        println!("[GStreamer] pipewiresrc not available, trying ximagesrc...");
    } else {
        println!("[GStreamer] X11 session detected, using ximagesrc");
    }

    if let Ok(el) = gst::ElementFactory::make("ximagesrc").name("screensrc").build() {
        return Ok(el);
    }
    println!("[GStreamer] ximagesrc not available, using videotestsrc fallback");
    gst::ElementFactory::make("videotestsrc")
        .name("screensrc")
        .build()
        .map_err(|_| "Failed to create any video source element".to_string())
}

#[cfg(windows)]
fn create_video_source(window_handle: Option<u64>) -> Result<gst::Element, String> {
    let use_window_capture = window_handle.filter(|&h| h != 0).is_some();

    if use_window_capture {
        return gst::ElementFactory::make("d3d11screencapturesrc")
            .name("d3d11screencapturesrc")
            .build()
            .map_err(|_| "d3d11screencapturesrc unavailable — required for window capture".to_string());
    }

    match gst::ElementFactory::make("d3d11screencapturesrc")
        .name("d3d11screencapturesrc")
        .build()
    {
        Ok(element) => Ok(element),
        Err(_) => {
            println!("d3d11screencapturesrc not available, trying d3d12...");
            match gst::ElementFactory::make("d3d12screencapturesrc")
                .name("d3d12screencapturesrc")
                .build()
            {
                Ok(element) => Ok(element),
                Err(_) => {
                    println!("d3d12screencapturesrc not available, trying alternative sources...");
                    match gst::ElementFactory::make("dx9screencapsrc")
                        .name("dx9screencapsrc")
                        .build()
                    {
                        Ok(element) => Ok(element),
                        Err(_) => {
                            println!("Using videotestsrc as fallback");
                            gst::ElementFactory::make("videotestsrc")
                                .name("videotestsrc")
                                .build()
                                .map_err(|_| "Failed to create any video source element".to_string())
                        }
                    }
                }
            }
        }
    }
}

// --- Platform-specific video source configuration ---

#[cfg(target_os = "linux")]
fn configure_video_source(src: &gst::Element, window_handle: Option<u64>, monitor_index: Option<u32>) {
    let factory = src.factory().unwrap().name();
    if factory == "ximagesrc" {
        src.set_property("use-damage", false);
        src.set_property("show-pointer", true);
        if let Some(xid) = window_handle.filter(|&h| h != 0) {
            src.set_property("xid", xid);
        } else if let Some(idx) = monitor_index {
            if let Ok(monitors) = crate::windows_capture::enumerate_monitors() {
                if let Some(mon) = monitors.get(idx as usize) {
                    src.set_property("startx", mon.x as u32);
                    src.set_property("starty", mon.y as u32);
                    src.set_property("endx", (mon.x + mon.width as i32 - 1) as u32);
                    src.set_property("endy", (mon.y + mon.height as i32 - 1) as u32);
                    println!(
                        "[GStreamer] ximagesrc region: {}x{} at ({},{})",
                        mon.width, mon.height, mon.x, mon.y
                    );
                }
            }
        }
    } else if factory == "videotestsrc" {
        src.set_property_from_str("pattern", "smpte");
        src.set_property("is-live", true);
    }
    // pipewiresrc: screen selection handled by xdg-desktop-portal dialog
}

#[cfg(windows)]
fn configure_video_source(src: &gst::Element, window_handle: Option<u64>, monitor_index: Option<u32>) {
    let src_factory = src.factory().unwrap().name();
    let mon_idx = monitor_index.map(|i| i as i32).unwrap_or(0);
    if src_factory == "d3d11screencapturesrc" {
        src.set_property("show-cursor", &true);
        if let Some(hwnd) = window_handle.filter(|&h| h != 0) {
            src.set_property_from_str("capture-api", "wgc");
            src.set_property("window-handle", hwnd);
        } else {
            src.set_property_from_str("capture-api", "wgc");
            src.set_property("monitor-index", &mon_idx);
        }
    } else if src_factory == "d3d12screencapturesrc" || src_factory == "dx9screencapsrc" {
        src.set_property("show-cursor", &true);
        src.set_property("monitor-index", &mon_idx);
    } else if src_factory == "videotestsrc" {
        src.set_property_from_str("pattern", "smpte");
        src.set_property("is-live", true);
    }
}

// --- Platform-specific video encoder creation ---

#[cfg(target_os = "linux")]
fn create_video_encoder(_src_factory_name: &str) -> Result<gst::Element, String> {
    let candidates = ["nvh264enc", "vaapih264enc", "vah264enc", "x264enc"];
    for name in candidates {
        if let Ok(enc) = gst::ElementFactory::make(name).name("videoenc").build() {
            println!("[GStreamer] Using video encoder: {}", name);
            return Ok(enc);
        }
    }
    Err("No H.264 encoder available (tried nvh264enc, vaapih264enc, vah264enc, x264enc)".to_string())
}

#[cfg(windows)]
fn create_video_encoder(src_factory_name: &str) -> Result<gst::Element, String> {
    let videoenc_factory = if src_factory_name == "d3d11screencapturesrc" {
        "nvd3d11h264enc"
    } else {
        "nvh264enc"
    };
    gst::ElementFactory::make(videoenc_factory)
        .name("videoenc")
        .build()
        .map_err(|_| format!("{} unavailable — required for hardware H.264 encode", videoenc_factory))
}

// --- Platform-specific system audio source creation ---

#[cfg(target_os = "linux")]
fn create_system_audio_source(_process_pid: Option<u32>) -> Result<gst::Element, String> {
    let src = gst::ElementFactory::make("pulsesrc")
        .name("system_audio_src")
        .build()
        .map_err(|_| "Failed to create pulsesrc for system audio".to_string())?;

    // PulseAudio monitor sources capture whatever is playing on an output
    // sink. Find the default output's monitor for system audio loopback.
    let monitor = find_default_monitor_source();
    if let Some(monitor_name) = monitor {
        println!("[GStreamer] Using PulseAudio monitor: {}", monitor_name);
        src.set_property("device", &monitor_name);
    }

    Ok(src)
}

#[cfg(target_os = "linux")]
fn find_default_monitor_source() -> Option<String> {
    let monitor = gst::DeviceMonitor::new();
    let caps = gst::Caps::new_empty_simple("audio/x-raw");
    let _ = monitor.add_filter(Some("Audio/Source"), Some(&caps));
    if monitor.start().is_err() {
        return None;
    }

    let mut result = None;
    for device in monitor.devices() {
        let props = device.properties();
        let is_monitor = props.as_ref()
            .and_then(|p| p.get::<String>("device.class").ok())
            .map(|c| c == "monitor")
            .unwrap_or(false);
        if !is_monitor {
            continue;
        }
        let device_name = props.as_ref()
            .and_then(|p| p.get::<String>("node.name")
                .or_else(|_| p.get::<String>("device.name"))
                .ok());
        if let Some(name) = device_name {
            result = Some(name);
            break;
        }
    }
    monitor.stop();
    result
}

#[cfg(windows)]
fn create_system_audio_source(process_pid: Option<u32>) -> Result<gst::Element, String> {
    let audio_src = gst::ElementFactory::make("wasapi2src")
        .name("system_audio_src")
        .build()
        .map_err(|_| "Failed to create wasapi2src element".to_string())?;
    audio_src.set_property("loopback", true);
    audio_src.set_property("low-latency", true);
    if let Some(pid) = process_pid.filter(|&p| p != 0) {
        let mode_set = audio_src.find_property("loopback-mode").is_some();
        let pid_set = audio_src.find_property("loopback-target-pid").is_some();
        if mode_set && pid_set {
            audio_src.set_property_from_str("loopback-mode", "include-process-tree");
            audio_src.set_property("loopback-target-pid", pid);
        }
    }
    Ok(audio_src)
}

// --- Platform-specific mic source creation ---

#[cfg(target_os = "linux")]
fn create_mic_source(dev_id: &str) -> Result<gst::Element, String> {
    let devices = enumerate_audio_input_devices();
    let device_match = devices.into_iter().find(|(_, id, _, _)| id == dev_id);
    match device_match {
        Some((dev, _, _, _)) => dev
            .create_element(Some("mic_src"))
            .map_err(|e| format!("Failed to create mic element from device: {:?}", e)),
        None => {
            let el = gst::ElementFactory::make("pulsesrc")
                .name("mic_src")
                .build()
                .map_err(|_| "Failed to create pulsesrc for mic input".to_string())?;
            el.set_property("device", dev_id);
            Ok(el)
        }
    }
}

#[cfg(windows)]
fn create_mic_source(dev_id: &str) -> Result<gst::Element, String> {
    let devices = enumerate_audio_input_devices();
    let device_match = devices.into_iter().find(|(_, id, _, _)| id == dev_id);
    match device_match {
        Some((dev, _, _, _)) => dev
            .create_element(Some("mic_src"))
            .map_err(|e| format!("Failed to create mic element from device: {:?}", e)),
        None => {
            let el = gst::ElementFactory::make("wasapi2src")
                .name("mic_src")
                .build()
                .map_err(|_| "Failed to create wasapi2src for mic input".to_string())?;
            el.set_property("device", dev_id);
            Ok(el)
        }
    }
}

// --- Platform-specific video chain linking ---

#[cfg(target_os = "linux")]
fn link_video_chain(
    pipeline: &gst::Pipeline,
    _src_factory_name: &str,
    chain_start: &gst::Element,
    videoconvert: &gst::Element,
    videoscale: &gst::Element,
    capsfilter: &gst::Element,
    videoenc: &gst::Element,
    h264_capsfilter: &gst::Element,
) -> Result<(), String> {
    pipeline.add_many(&[videoconvert, videoscale, capsfilter])
        .map_err(|_| "Failed to add convert/scale/capsfilter to pipeline".to_string())?;
    gst::Element::link_many(&[chain_start, videoconvert, videoscale, capsfilter, videoenc, h264_capsfilter])
        .map_err(|e| format!("Failed to link video elements: {:?}", e))
}

#[cfg(windows)]
fn link_video_chain(
    pipeline: &gst::Pipeline,
    src_factory_name: &str,
    chain_start: &gst::Element,
    videoconvert: &gst::Element,
    videoscale: &gst::Element,
    capsfilter: &gst::Element,
    videoenc: &gst::Element,
    h264_capsfilter: &gst::Element,
) -> Result<(), String> {
    if src_factory_name == "d3d11screencapturesrc" {
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
        gst::Element::link_many(&[chain_start, &d3d11convert, &d3d11_capsfilter, videoenc, h264_capsfilter])
            .map_err(|e| format!("Failed to link d3d11 zero-copy chain: {:?}", e))
    } else if src_factory_name == "d3d12screencapturesrc" {
        pipeline.add_many(&[videoconvert, videoscale, capsfilter])
            .map_err(|_| "Failed to add convert/scale/capsfilter to pipeline".to_string())?;
        let d3d12download = gst::ElementFactory::make("d3d12download")
            .build()
            .map_err(|_| "d3d12screencapturesrc requires the d3d12download element".to_string())?;
        pipeline.add(&d3d12download).map_err(|_| "Failed to add d3d12download to pipeline".to_string())?;
        gst::Element::link_many(&[chain_start, &d3d12download, videoconvert, videoscale, capsfilter, videoenc, h264_capsfilter])
            .map_err(|e| format!("Failed to link video elements with d3d12download: {:?}", e))
    } else {
        pipeline.add_many(&[videoconvert, videoscale, capsfilter])
            .map_err(|_| "Failed to add convert/scale/capsfilter to pipeline".to_string())?;
        gst::Element::link_many(&[chain_start, videoconvert, videoscale, capsfilter, videoenc, h264_capsfilter])
            .map_err(|e| format!("Failed to link video elements: {:?}", e))
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

// Tear the audio chain out of a running pipeline so an audio source failure
// can't take video down with it. The source failure leaves the element in
// ERROR state, which propagates GST_FLOW_ERROR upstream through rtpbin's
// session 1 and blocks data flow on the shared rtpbin — so we unlink the
// session-1 pads, set the audio elements to NULL, and remove them entirely.
fn disable_audio_chain(pipeline: &gst::Pipeline) -> bool {
    let names = [
        "system_audio_src", "audioconvert", "audioresample", "sys_audio_caps",
        "mic_src", "mic_audioconvert", "mic_audioresample", "mic_caps", "mic_valve",
        "audio_mixer",
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

// Pop any queued error messages from the pipeline bus. Used when a synchronous
// state-change call fails so the surfaced Tauri error includes the actual
// upstream cause (which element / what went wrong) instead of the generic
// `StateChangeError`. Drains for up to a short timeout each pop.
fn drain_bus_errors(pipeline: &gst::Pipeline) -> String {
    let bus = match pipeline.bus() {
        Some(b) => b,
        None => return String::new(),
    };
    let mut out = String::new();
    while let Some(msg) = bus.timed_pop_filtered(
        Some(gst::ClockTime::from_mseconds(100)),
        &[gst::MessageType::Error, gst::MessageType::Warning],
    ) {
        if let gst::MessageView::Error(err) = msg.view() {
            out.push_str(&format!(
                " — {} from {:?}: {}  debug={:?}",
                if msg.type_() == gst::MessageType::Error { "error" } else { "warning" },
                err.src().map(|s| s.path_string().to_string()),
                err.error(),
                err.debug(),
            ));
        }
    }
    out
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

            // Audio source can fail to open at runtime (no playback device,
            // exclusive-mode contention, missing PulseAudio/WASAPI session).
            // Isolate the failure to the audio chain so the video stream
            // keeps flowing.
            let from_audio = err.src()
                .map(|s| s.path_string().to_string())
                .map(|p| p.contains("system_audio_src") || p.contains("mic_src"))
                .unwrap_or(false);
            if from_audio {
                if let Some(src) = err.src() {
                    if let Some(pipeline) = find_pipeline(&src) {
                        if disable_audio_chain(&pipeline) {
                            eprintln!("[GStreamer] Audio chain disabled after audio source error; video continues");
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

fn build_preview_branch(
    pipeline: &gst::Pipeline,
    tee: &gst::Element,
    src_factory_name: &str,
) {
    let make = |factory: &str, name: &str| -> Option<gst::Element> {
        gst::ElementFactory::make(factory).name(name).build().ok()
    };

    let queue = match make("queue", "preview_queue") {
        Some(q) => q,
        None => { eprintln!("[Preview] queue unavailable, skipping preview"); return; }
    };
    queue.set_property_from_str("leaky", "downstream");
    queue.set_property("max-size-buffers", 2u32);
    queue.set_property("max-size-time", 0u64);
    queue.set_property("max-size-bytes", 0u32);

    let download: Option<gst::Element> = match src_factory_name {
        "d3d11screencapturesrc" => make("d3d11download", "preview_download"),
        "d3d12screencapturesrc" => make("d3d12download", "preview_download"),
        _ => None,
    };

    let videoconvert = match make("videoconvert", "preview_videoconvert") {
        Some(v) => v,
        None => { eprintln!("[Preview] videoconvert unavailable, skipping preview"); return; }
    };
    let videoscale = match make("videoscale", "preview_videoscale") {
        Some(v) => v,
        None => { eprintln!("[Preview] videoscale unavailable, skipping preview"); return; }
    };
    let videorate = match make("videorate", "preview_videorate") {
        Some(v) => v,
        None => { eprintln!("[Preview] videorate unavailable, skipping preview"); return; }
    };
    let capsfilter = match make("capsfilter", "preview_capsfilter") {
        Some(c) => c,
        None => { eprintln!("[Preview] capsfilter unavailable, skipping preview"); return; }
    };
    let jpegenc = match make("jpegenc", "preview_jpegenc") {
        Some(j) => j,
        None => { eprintln!("[Preview] jpegenc unavailable, skipping preview"); return; }
    };
    let fakesink = match make("fakesink", "preview_fakesink") {
        Some(f) => f,
        None => { eprintln!("[Preview] fakesink unavailable, skipping preview"); return; }
    };

    let caps = gst::Caps::builder("video/x-raw")
        .field("width", 1920i32)
        .field("height", 1080i32)
        .field("framerate", gst::Fraction::new(15, 1))
        .build();
    capsfilter.set_property("caps", &caps);
    jpegenc.set_property("quality", 90i32);
    fakesink.set_property("sync", false);

    if let Some(pad) = jpegenc.static_pad("src") {
        pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if let Some(buffer) = info.buffer() {
                if let Ok(map) = buffer.map_readable() {
                    let b64 = STANDARD.encode(map.as_slice());
                    *PREVIEW_FRAME.lock().unwrap() = b64;
                }
            }
            gst::PadProbeReturn::Ok
        });
    }

    let mut elements: Vec<&gst::Element> = vec![&queue];
    if let Some(ref dl) = download {
        elements.push(dl);
    }
    elements.extend_from_slice(&[&videoconvert, &videoscale, &videorate, &capsfilter, &jpegenc, &fakesink]);

    if pipeline.add_many(&elements.iter().copied().collect::<Vec<_>>()).is_err() {
        eprintln!("[Preview] Failed to add preview elements to pipeline, skipping");
        return;
    }

    if gst::Element::link_many(&elements).is_err() {
        eprintln!("[Preview] Failed to link preview chain, skipping");
        return;
    }

    if tee.link(&queue).is_err() {
        eprintln!("[Preview] Failed to link tee → preview queue, skipping");
        return;
    }

    println!("[Preview] Preview branch attached ({} → 1920x1080 @15fps JPEG q90)", src_factory_name);
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
    monitor_index: Option<u32>,
    mic_enabled: bool,
    mic_device_id: Option<String>,
    mic_initially_muted: bool,
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

    let src = create_video_source(window_handle)?;
    let src_factory_name = src.factory().unwrap().name().to_string();

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

    let videoenc = create_video_encoder(&src_factory_name)?;

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

    let audio_src = create_system_audio_source(process_pid)?;

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

    // audiomixer enforces a single negotiated format across all its sinks, so
    // we pin format=F32LE (wasapi2src's native output) on both branches'
    // upstream capsfilters. Without an explicit format the two chains can
    // pick S16LE vs F32LE independently and the mixer refuses to link.
    //
    // `channel-mask` is REQUIRED for >1 channel streams — audioconvert's
    // mono → stereo upmix without an explicit channel-mask falls back to a
    // 0x0 ("no positions specified") layout and silently produces zero
    // samples on both channels. Pinning the canonical FL+FR mask (0x3) makes
    // the upmix actually duplicate the mic's mono signal across L+R.
    let mixer_in_caps = gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("layout", "interleaved")
        .field("rate", 48000i32)
        .field("channels", 2i32)
        .field("channel-mask", gst::Bitmask::new(0x3))
        .build();

    // Mic branch wiring: only built when the frontend provided a device id.
    // System audio goes to mixer sink_0; mic goes through a valve (for live
    // mute) into sink_1. When no mic device is supplied, the original
    // single-source audio chain is used unchanged.
    let mic_branch = if let Some(ref dev_id) = mic_device_id {
        let mic_src = create_mic_source(dev_id)?;
        let mic_audioconvert = gst::ElementFactory::make("audioconvert")
            .name("mic_audioconvert")
            .build()
            .map_err(|_| "Failed to create mic audioconvert".to_string())?;
        let mic_audioresample = gst::ElementFactory::make("audioresample")
            .name("mic_audioresample")
            .build()
            .map_err(|_| "Failed to create mic audioresample".to_string())?;
        let mic_caps = gst::ElementFactory::make("capsfilter")
            .name("mic_caps")
            .build()
            .map_err(|_| "Failed to create mic capsfilter".to_string())?;
        mic_caps.set_property("caps", &mixer_in_caps);
        let mic_valve = gst::ElementFactory::make("valve")
            .name("mic_valve")
            .build()
            .map_err(|_| "Failed to create mic valve element".to_string())?;
        // drop=true → discards buffers immediately (mute). The mic branch is
        // always wired into the mixer; the valve gates flow at runtime.
        mic_valve.set_property("drop", mic_initially_muted || !mic_enabled);
        Some((mic_src, mic_audioconvert, mic_audioresample, mic_caps, mic_valve))
    } else {
        None
    };

    let sys_audio_caps = gst::ElementFactory::make("capsfilter")
        .name("sys_audio_caps")
        .build()
        .map_err(|_| "Failed to create sys_audio_caps capsfilter".to_string())?;
    // When mixing we need a fully-specified format; without a mixer the
    // existing opus_capsfilter downstream is sufficient and we keep the lighter
    // rate/channels-only caps so audioconvert/audioresample can pick a format
    // that opusenc happily ingests.
    if mic_device_id.is_some() {
        sys_audio_caps.set_property("caps", &mixer_in_caps);
    } else {
        sys_audio_caps.set_property("caps", &opus_in_caps);
    }

    let audio_mixer = if mic_branch.is_some() {
        let mixer = gst::ElementFactory::make("audiomixer")
            .name("audio_mixer")
            .build()
            .map_err(|_| "Failed to create audiomixer element".to_string())?;
        // Be lenient about live-source alignment. wasapi2src for a fresh
        // capture endpoint can hand its first buffers up with timestamps that
        // are several tens of ms behind the already-running loopback source;
        // audiomixer's default alignment-threshold (40 ms) classifies those
        // as "too late" and silently drops them, so the viewer hears the
        // game/system audio but never the mic. Widening the window keeps both
        // streams mixed instead of dropped.
        if mixer.find_property("alignment-threshold").is_some() {
            // 200 ms (in nanoseconds)
            mixer.set_property("alignment-threshold", 200_000_000u64);
        }
        if mixer.find_property("latency").is_some() {
            // Mixer holds output for `latency` ns to allow late inputs to
            // arrive — same reason as above, just on the output side.
            mixer.set_property("latency", 100_000_000u64);
        }
        Some(mixer)
    } else {
        None
    };

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

    configure_video_source(&src, window_handle, monitor_index);
    configure_encoder(&videoenc);

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

    // Tee splits the capture source into the main encode/RTP path and a
    // lightweight preview branch that feeds JPEG snapshots to the frontend.
    let tee = gst::ElementFactory::make("tee")
        .name("video_tee")
        .build()
        .map_err(|_| "Failed to create tee element".to_string())?;
    let queue_main = gst::ElementFactory::make("queue")
        .name("queue_main")
        .build()
        .map_err(|_| "Failed to create queue_main element".to_string())?;

    // h264_tee splits the encoded H264 stream so recording can tap it
    // without re-encoding. The recording branch is added dynamically
    // by start_recording().
    let h264_tee = gst::ElementFactory::make("tee")
        .name("h264_tee")
        .build()
        .map_err(|_| "Failed to create h264_tee element".to_string())?;
    let queue_rtp = gst::ElementFactory::make("queue")
        .name("queue_rtp")
        .build()
        .map_err(|_| "Failed to create queue_rtp element".to_string())?;

    // Add elements to pipeline. Audio is always wired up here; if the WASAPI
    // device fails to open at runtime, the bus error handler tears down the
    // audio sub-graph at runtime (see `disable_audio_chain`) so the video
    // chain isn't blocked by an element stuck in ERROR state propagating
    // through the shared rtpbin.
    pipeline.add_many(&[
        &rtpbin, &src, &tee, &queue_main,
        &videoenc, &h264_capsfilter, &h264_tee, &queue_rtp, &rtph264pay, &rtp_sink,
        &rtcp_sink, &rtcp_src,
        &audio_src, &audioconvert, &audioresample, &sys_audio_caps, &opus_capsfilter,
        &opusenc, &rtpopuspay, &audio_rtp_sink, &audio_rtcp_sink, &audio_rtcp_src,
    ]).map_err(|_| "Failed to add elements to pipeline".to_string())?;

    if let Some((ref mic_src, ref mic_ac, ref mic_ar, ref mic_caps, ref mic_valve)) = mic_branch {
        pipeline.add_many(&[mic_src, mic_ac, mic_ar, mic_caps, mic_valve])
            .map_err(|_| "Failed to add mic branch to pipeline".to_string())?;
    }
    if let Some(ref mixer) = audio_mixer {
        pipeline.add(mixer).map_err(|_| "Failed to add audiomixer to pipeline".to_string())?;
    }

    gst::Element::link_many(&[&src, &tee])
        .map_err(|e| format!("Failed to link src → tee: {:?}", e))?;
    tee.link(&queue_main)
        .map_err(|e| format!("Failed to link tee → queue_main: {:?}", e))?;

    link_video_chain(&pipeline, &src_factory_name, &queue_main, &videoconvert, &videoscale, &capsfilter, &videoenc, &h264_capsfilter)?;

    // h264_capsfilter → h264_tee → queue_rtp → rtph264pay
    gst::Element::link_many(&[&h264_capsfilter, &h264_tee])
        .map_err(|e| format!("Failed to link h264_capsfilter → h264_tee: {:?}", e))?;
    h264_tee.link(&queue_rtp)
        .map_err(|e| format!("Failed to link h264_tee → queue_rtp: {:?}", e))?;
    gst::Element::link_many(&[&queue_rtp, &rtph264pay])
        .map_err(|e| format!("Failed to link queue_rtp → rtph264pay: {:?}", e))?;

    build_preview_branch(&pipeline, &tee, &src_factory_name);

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

    // Audio chain: capture → convert/resample → 48k/2ch caps. When a mic was
    // selected, both the system audio and mic feeds run into an audiomixer
    // (mic via a `valve` for live mute/unmute), and the mixer's single output
    // goes through Opus → RTP. Without a mic, system audio feeds the encoder
    // directly. Rides rtpbin session 1; the bus error handler removes this
    // whole sub-graph at runtime if WASAPI fails to open the endpoint.
    if let (Some((mic_src, mic_ac, mic_ar, mic_caps, mic_valve)), Some(mixer)) =
        (mic_branch.as_ref(), audio_mixer.as_ref())
    {
        gst::Element::link_many(&[
            &audio_src, &audioconvert, &audioresample, &sys_audio_caps,
        ]).map_err(|e| format!("Failed to link system audio chain: {:?}", e))?;
        gst::Element::link_many(&[
            mic_src, mic_ac, mic_ar, mic_caps, mic_valve,
        ]).map_err(|e| format!("Failed to link mic audio chain: {:?}", e))?;

        // audiomixer exposes request sink pads named sink_%u. Request them
        // explicitly so each branch lands on its own pad — link() without an
        // explicit pad name picks sink_0 for both branches and fails.
        let sys_sink = mixer.request_pad_simple("sink_%u")
            .ok_or_else(|| "audiomixer refused sink request for system audio".to_string())?;
        let mic_sink = mixer.request_pad_simple("sink_%u")
            .ok_or_else(|| "audiomixer refused sink request for mic".to_string())?;
        sys_audio_caps.static_pad("src")
            .ok_or_else(|| "sys_audio_caps has no src pad".to_string())?
            .link(&sys_sink)
            .map_err(|e| format!("Failed to link system audio into mixer: {:?}", e))?;
        mic_valve.static_pad("src")
            .ok_or_else(|| "mic_valve has no src pad".to_string())?
            .link(&mic_sink)
            .map_err(|e| format!("Failed to link mic into mixer: {:?}", e))?;

        // audiomixer's src pad was observed negotiating MONO 48k F32 even
        // when both its sinks were configured stereo via capsfilters with an
        // explicit channel-mask — opus_capsfilter alone (channels=2, no
        // channel-mask) wasn't enough to keep stereo on the mixer's src side,
        // and the resulting mono Opus stream rendered silent on consumers
        // expecting 2 channels. The post-mixer audioconvert reshapes
        // whatever the mixer emits back to stereo before Opus encode.
        let mixer_out_convert = gst::ElementFactory::make("audioconvert")
            .name("mixer_out_convert")
            .build()
            .map_err(|_| "Failed to create mixer_out_convert".to_string())?;
        pipeline.add(&mixer_out_convert)
            .map_err(|_| "Failed to add mixer_out_convert to pipeline".to_string())?;

        gst::Element::link_many(&[mixer, &mixer_out_convert, &opus_capsfilter, &opusenc, &rtpopuspay])
            .map_err(|e| format!("Failed to link mixer to opus chain: {:?}", e))?;
    } else {
        gst::Element::link_many(&[
            &audio_src, &audioconvert, &audioresample, &sys_audio_caps, &opus_capsfilter,
            &opusenc, &rtpopuspay,
        ]).map_err(|e| format!("Failed to link audio elements: {:?}", e))?;
    }

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
        Err(e) => {
            let extra = drain_bus_errors(&pipeline);
            return Err(format!("Failed to start pipeline: {:?}{}", e, extra));
        }
    }

    // Wait for state change to complete with timeout
    let (state_result, current_state, pending_state) = pipeline.state(gst::ClockTime::from_seconds(5));
    match state_result {
        Ok(_) => {
            println!("Pipeline current state: {:?}, pending: {:?}", current_state, pending_state);
        }
        Err(e) => {
            let extra = drain_bus_errors(&pipeline);
            return Err(format!("Failed to get pipeline state: {:?}{}", e, extra));
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
    // Stop recording first if active, so the MP4 is finalized
    {
        let state = STATE.lock().unwrap();
        if state.recording {
            drop(state);
            let _ = stop_recording();
        }
    }

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

    *PREVIEW_FRAME.lock().unwrap() = String::new();
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
    monitor_index: Option<u32>,
    mic_enabled: bool,
    mic_device_id: Option<String>,
    mic_initially_muted: bool,
) -> Result<String, String> {
    init()?;

    println!(
        "video {}:{}/{}  audio {}:{}/{}  monitor_index={:?} mic_enabled={} mic_device={:?} mic_initially_muted={}",
        video_host, video_rtp_port, video_rtcp_port,
        audio_host, audio_rtp_port, audio_rtcp_port,
        monitor_index, mic_enabled, mic_device_id, mic_initially_muted,
    );

    create_gstreamer_pipeline(
        &video_host, video_rtp_port, video_rtcp_port,
        &audio_host, audio_rtp_port, audio_rtcp_port,
        window_handle, process_pid, monitor_index,
        mic_enabled, mic_device_id, mic_initially_muted,
    )?;

    Ok("Streaming started successfully".to_string())
}

// Stop streaming function
pub fn stop_streaming() -> Result<(), String> {
    cleanup();
    Ok(())
}

pub fn start_recording(path: String) -> Result<(), String> {
    let mut state = STATE.lock().unwrap();
    let pipeline = state.pipeline.as_ref()
        .ok_or_else(|| "No active pipeline".to_string())?;

    if state.recording {
        return Err("Already recording".to_string());
    }

    let h264_tee = pipeline.by_name("h264_tee")
        .ok_or_else(|| "h264_tee not found in pipeline".to_string())?;

    let make = |factory: &str, name: &str| -> Result<gst::Element, String> {
        gst::ElementFactory::make(factory).name(name).build()
            .map_err(|_| format!("Failed to create {} ({})", name, factory))
    };

    let queue = make("queue", "rec_queue")?;
    queue.set_property("max-size-buffers", 300u32);
    queue.set_property_from_str("leaky", "downstream");

    let h264parse = make("h264parse", "rec_h264parse")?;

    let mux = make("mp4mux", "rec_mux")?;
    mux.set_property_from_str("fragment-duration", "1000");

    let filesink = make("filesink", "rec_filesink")?;
    filesink.set_property("location", &path);
    filesink.set_property("sync", false);
    filesink.set_property("async", false);

    pipeline.add_many(&[&queue, &h264parse, &mux, &filesink])
        .map_err(|_| "Failed to add recording elements to pipeline".to_string())?;

    gst::Element::link_many(&[&queue, &h264parse, &mux, &filesink])
        .map_err(|e| format!("Failed to link recording chain: {:?}", e))?;

    // Sync element states with the pipeline before linking to the tee,
    // so they are PLAYING when data arrives.
    for el in [&queue, &h264parse, &mux, &filesink] {
        el.sync_state_with_parent()
            .map_err(|_| format!("Failed to sync state for {}", el.name()))?;
    }

    h264_tee.link(&queue)
        .map_err(|e| format!("Failed to link h264_tee → rec_queue: {:?}", e))?;

    state.recording = true;
    println!("[Recording] Started recording to {}", path);
    Ok(())
}

pub fn stop_recording() -> Result<(), String> {
    let mut state = STATE.lock().unwrap();
    let pipeline = state.pipeline.as_ref()
        .ok_or_else(|| "No active pipeline".to_string())?;

    if !state.recording {
        return Err("Not recording".to_string());
    }

    let h264_tee = pipeline.by_name("h264_tee")
        .ok_or_else(|| "h264_tee not found in pipeline".to_string())?;
    let rec_queue = pipeline.by_name("rec_queue")
        .ok_or_else(|| "rec_queue not found in pipeline".to_string())?;

    // Unlink tee from recording queue
    if let Some(sink_pad) = rec_queue.static_pad("sink") {
        if let Some(tee_src_pad) = sink_pad.peer() {
            let _ = tee_src_pad.unlink(&sink_pad);
            h264_tee.release_request_pad(&tee_src_pad);
        }
    }

    // Send EOS to the recording queue to flush and finalize the MP4
    if let Some(sink_pad) = rec_queue.static_pad("sink") {
        sink_pad.send_event(gst::event::Eos::new());
    }

    // Wait briefly for EOS to propagate through mux → filesink
    std::thread::sleep(Duration::from_millis(500));

    let rec_elements: Vec<gst::Element> = ["rec_queue", "rec_h264parse", "rec_mux", "rec_filesink"]
        .iter()
        .filter_map(|name| pipeline.by_name(name))
        .collect();

    for el in &rec_elements {
        let _ = el.set_state(gst::State::Null);
    }
    for el in &rec_elements {
        let _ = pipeline.remove(el);
    }

    state.recording = false;
    println!("[Recording] Stopped recording");
    Ok(())
}