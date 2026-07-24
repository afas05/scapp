// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod gstreamer;
mod discord_presence;
mod windows_capture;
#[cfg(target_os = "linux")]
mod linux_capture;

#[tauri::command]
async fn list_windows() -> Result<Vec<windows_capture::WindowInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| windows_capture::enumerate_windows())
        .await
        .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command]
async fn list_monitors() -> Result<Vec<windows_capture::MonitorInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| windows_capture::enumerate_monitors())
        .await
        .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command]
async fn list_audio_inputs() -> Result<Vec<gstreamer::MicDevice>, String> {
    tauri::async_runtime::spawn_blocking(|| gstreamer::list_audio_input_devices())
        .await
        .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command]
async fn get_preview_frame() -> Result<String, String> {
    gstreamer::get_preview_frame()
}

#[tauri::command]
async fn start_stream(
    video_host: String,
    video_rtp_port: u16,
    video_rtcp_port: u16,
    audio_host: String,
    audio_rtp_port: u16,
    audio_rtcp_port: u16,
    window_handle: Option<u64>,
    process_pid: Option<u32>,
    monitor_index: Option<u32>,
    mic_enabled: Option<bool>,
    mic_device_id: Option<String>,
    mic_initially_muted: Option<bool>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        gstreamer::start_streaming(
            video_host,
            video_rtp_port,
            video_rtcp_port,
            audio_host,
            audio_rtp_port,
            audio_rtcp_port,
            window_handle,
            process_pid,
            monitor_index,
            mic_enabled.unwrap_or(false),
            mic_device_id,
            mic_initially_muted.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn stop_stream() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        gstreamer::stop_streaming()
    })
    .await
    .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn set_mic_muted(muted: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || gstreamer::set_mic_muted(muted))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn switch_source(window_handle: Option<u64>, monitor_index: Option<u32>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || gstreamer::switch_source(window_handle, monitor_index))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn start_recording(app: tauri::AppHandle, path: String, watermark: bool) -> Result<(), String> {
    // The frontend decides whether the watermark applies (free tier always;
    // paid tier unless the user opted out). Resolve the bundled PNG only when
    // requested; a missing file degrades to an un-watermarked recording.
    let watermark_path = if watermark {
        use tauri::Manager;
        match app.path().resource_dir() {
            Ok(dir) => {
                let p = dir.join("watermark.png");
                if p.exists() {
                    Some(p)
                } else {
                    eprintln!("[Recording] watermark.png not found at {:?}; recording without watermark", p);
                    None
                }
            }
            Err(e) => {
                eprintln!("[Recording] resource_dir() failed ({e}); recording without watermark");
                None
            }
        }
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || gstreamer::start_recording(path, watermark_path))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn stop_recording() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| gstreamer::stop_recording())
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn list_recordings(dir: String) -> Result<Vec<gstreamer::RecordingInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || gstreamer::list_recordings(&dir))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn export_clip(
    app: tauri::AppHandle,
    clip_id: String,
    input: String,
    output: String,
    start_secs: f64,
    end_secs: f64,
    crop_offset: f32,
    watermark: bool,
) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    // Same watermark resolution as start_recording: burn the bundled PNG only
    // when requested and present; otherwise export clean.
    let watermark_path = if watermark {
        app.path()
            .resource_dir()
            .ok()
            .map(|d| d.join("watermark.png"))
            .filter(|p| p.exists())
    } else {
        None
    };
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        gstreamer::export_clip(
            &input,
            &output,
            start_secs,
            end_secs,
            crop_offset,
            watermark_path,
            move |pct| {
                let _ = app_for_progress.emit(
                    "clip-export-progress",
                    serde_json::json!({ "clipId": clip_id, "percent": pct }),
                );
            },
        )
    })
    .await
    .map_err(|e| format!("Failed to join export task: {}", e))?
}

// Upload an exported clip to the autopost API as multipart/form-data. The file
// is read on a blocking thread and sent via reqwest (already in the dep tree).
// Returns { status, body } so the frontend can distinguish 202 Accepted from a
// 409 services_not_connected without exception handling on the wire.
#[tauri::command]
async fn autopost_submit(
    token: String,
    file_path: String,
    title: String,
    description: Option<String>,
    tags: Option<Vec<String>>,
    privacy: Option<String>,
    services: Option<Vec<String>>,
    publish_at: Option<String>,
) -> Result<serde_json::Value, String> {
    let file_name = std::path::Path::new(&file_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "clip.mp4".to_string());

    let read_path = file_path.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || std::fs::read(&read_path))
        .await
        .map_err(|e| format!("Failed to join read task: {}", e))?
        .map_err(|e| format!("Failed to read clip {}: {}", file_path, e))?;

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name)
        .mime_str("video/mp4")
        .map_err(|e| format!("Invalid clip part: {}", e))?;

    let mut form = reqwest::multipart::Form::new()
        .part("video", part)
        .text("title", title);
    if let Some(d) = description {
        form = form.text("description", d);
    }
    if let Some(p) = privacy {
        form = form.text("privacy", p);
    }
    if let Some(pa) = publish_at {
        form = form.text("publish_at", pa);
    }
    for t in tags.unwrap_or_default() {
        form = form.text("tags[]", t);
    }
    for s in services.unwrap_or_default() {
        form = form.text("services[]", s);
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://streamsnipe.live/api/autopost/posts")
        .bearer_auth(token)
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Upload request failed: {}", e))?;

    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read upload response: {}", e))?;
    let body: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "raw": text }));

    Ok(serde_json::json!({ "status": status, "body": body }))
}

#[tauri::command]
async fn discord_set_streaming(producer_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || discord_presence::set_streaming(producer_id))
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[tauri::command]
async fn discord_clear() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| discord_presence::clear())
        .await
        .map_err(|e| format!("Failed to join task: {}", e))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            #[cfg(windows)]
            {
                use tauri::Manager;

                let resource_dir = _app.path().resource_dir()
                    .expect("Failed to resolve resource directory");
                let gst_plugin_path = resource_dir.join("gstreamer-1.0");
                std::env::set_var("GST_PLUGIN_PATH", &gst_plugin_path);
                std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", "");
                std::env::set_var("GST_PLUGIN_SYSTEM_PATH", "");
                println!("[setup] GST_PLUGIN_PATH = {}", gst_plugin_path.display());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![start_stream, stop_stream, switch_source, list_windows, list_monitors, list_audio_inputs, set_mic_muted, get_preview_frame, start_recording, stop_recording, list_recordings, export_clip, autopost_submit, discord_set_streaming, discord_clear])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
